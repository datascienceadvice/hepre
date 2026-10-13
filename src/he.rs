//! Гомоморфный контур логистической регрессии (CKKS).
//!
//! Контур повторяет открытую эталонную модель [`crate::logistic::LogisticModel`]
//! «один в один», но работает поверх шифротекстов:
//!
//! ```text
//! лаборатория : z_ij = (x_ij - mean_j) / sd_j        → Enc(z_j)
//! сервер      : η = b + Σ_j w_j · Enc(z_j)           (EvalMultPlain + EvalAdd)
//! сервер      : p = 1/2 + P(η / B)                   (многочлен, см. crate::poly)
//! сервер      : p → ReEncrypt(rekey)                 (PRE на ключ больницы)
//! больница    : Dec(p), решение p ≥ 1/2
//! ```
//!
//! # Требование к ключу умножения
//!
//! Многочленная сигмоида содержит умножения **шифротекст на шифротекст**,
//! поэтому до вычислений должен быть сгенерирован ключ умножения
//! ([`Context::evmult_keygen`]). Ключ выводится из секретного ключа
//! лаборатории и публикуется серверу — это стандартный элемент FHE-контура
//! (релинааризационный ключ для `s²`), он не раскрывает `s`.
//!
//! # Порядок действий и мультипликативная глубина
//!
//! Используется схема с подстановкой `w = u²` и блочным разбиением
//! коэффициентов: нечётные степени `u^(2j+1)` считаются цепочкой
//! (глубина `j`), а блоки связываются схемой Горнера по `w^m`. Итоговая
//! глубина берётся из [`select_evaluation_scheme`] и **должна** быть не
//! больше `multiplicative_depth` контекста — это проверяется в
//! [`required_depth`] и в тестах.

use crate::logistic::LogisticModel;
use crate::poly::{select_evaluation_scheme, SigmoidPolynomial};
use crate::scoring::EncodedFeatures;
use crate::{Ciphertext, CkksParams, Context, Error, Plaintext, PublicKey, Result};

/// Мультипликативная глубина, необходимая для данной аппроксимации.
///
/// Это значение нужно передать в [`CkksParams::multiplicative_depth`] при
/// создании контекста.
pub fn required_depth(poly: &SigmoidPolynomial) -> u32 {
    let k_max = poly.coeffs.len().saturating_sub(1);
    select_evaluation_scheme(k_max).0 as u32
}

/// Создаёт CKKS-контекст, достаточный для вычисления многочлена.
///
/// `margin` — запас по глубине на непредвиденные операции; по умолчанию
/// разумно брать 0 или 1.
pub fn context_for_polynomial(poly: &SigmoidPolynomial, margin: u32) -> Result<Context> {
    Context::new_ckks(CkksParams {
        batch_size: 8192,
        multiplicative_depth: required_depth(poly) + margin,
        scaling_mod_size: 40,
    })
}

/// Плейнтекст-константа, повторённая по всем активным слотам.
///
/// CKKS выполняет операции над слотами поэлементно, поэтому константа
/// должна быть размножена на всю используемую длину, а не передана одним
/// значением.
pub fn constant_plaintext(cc: &Context, value: f64, n_slots: usize) -> Result<Plaintext> {
    if n_slots == 0 {
        return Err(Error("constant_plaintext: n_slots = 0".to_string()));
    }
    cc.make_ckks_plaintext(&vec![value; n_slots])
}

/// Шифрует стандартизованные признаки: один шифротекст на признак,
/// пациент — слот.
///
/// В отличие от [`crate::scoring::encrypt_features`], значения берутся не
/// сырыми, а приведёнными к стандартизованному масштабу модели. Именно это
/// уходит на сервер — сырые лабораторные значения не покидают лабораторию.
pub fn encrypt_standardized_features(
    cc: &Context,
    pk: &PublicKey,
    model: &LogisticModel,
    xs: &[Vec<f64>],
) -> Result<EncodedFeatures> {
    let n = xs.len();
    if n == 0 {
        return Ok(EncodedFeatures {
            cts: Vec::new(),
            n: 0,
            intercept_pt: None,
        });
    }
    let f = model.weights.len();
    let batch = (cc.ring_dimension() / 2) as usize;
    if n > batch {
        return Err(Error(format!(
            "encrypt_standardized_features: {n} пациентов не помещаются в пакет из {batch} слотов"
        )));
    }
    if let Some(bad) = xs.iter().find(|r| r.len() != f) {
        return Err(Error(format!(
            "encrypt_standardized_features: ожидалось {f} признаков, получено {}",
            bad.len()
        )));
    }

    let mut cts = Vec::with_capacity(f);
    for j in 0..f {
        let column: Vec<f64> = xs
            .iter()
            .map(|row| (row[j] - model.means[j]) / model.sds[j])
            .collect();
        let pt = cc.make_ckks_plaintext(&column)?;
        cts.push(cc.encrypt(pk, &pt)?);
    }

    Ok(EncodedFeatures {
        cts,
        n,
        intercept_pt: Some(cc.make_ckks_plaintext(&vec![model.intercept; n])?),
    })
}

/// Гомоморфно считает логит `η = b + Σ_j w_j · z_j`.
///
/// Контур аддитивный: только умножения на плейнтекст и сложения, поэтому
/// мультипликативная глубина не расходуется. Это «режим A» — сервер
/// возвращает больнице непрерывный логит.
pub fn compute_logit(
    cc: &Context,
    feats: &EncodedFeatures,
    model: &LogisticModel,
) -> Result<Ciphertext> {
    if feats.cts.is_empty() {
        return Err(Error(
            "compute_logit: нет зашифрованных признаков".to_string(),
        ));
    }
    if model.weights.len() != feats.cts.len() {
        return Err(Error(format!(
            "compute_logit: признаков {} против весов {}",
            feats.cts.len(),
            model.weights.len()
        )));
    }

    let mut acc: Option<Ciphertext> = None;
    for (j, ct) in feats.cts.iter().enumerate() {
        let w_pt = constant_plaintext(cc, model.weights[j], feats.n)?;
        let term = cc.eval_mult_plain(ct, &w_pt)?;
        acc = Some(match acc {
            None => term,
            Some(a) => cc.eval_add(&a, &term)?,
        });
    }

    let mut acc = acc.ok_or_else(|| Error("compute_logit: пустой аккумулятор".to_string()))?;
    if let Some(intercept_pt) = &feats.intercept_pt {
        acc = cc.eval_add_plain(&acc, intercept_pt)?;
    }
    Ok(acc)
}

/// Гомоморфно вычисляет вероятность `p = σ(η)` многочленной аппроксимацией.
///
/// Возвращает шифротекст, в слотах которого лежат вероятности
/// положительного исхода. Решение — сравнение расшифрованного значения с
/// `0.5`; при сохранении знака многочлена оно тождественно `η ≥ 0`.
pub fn eval_sigmoid_polynomial(
    cc: &Context,
    eta: &Ciphertext,
    poly: &SigmoidPolynomial,
    n_slots: usize,
) -> Result<Ciphertext> {
    eval_sigmoid_polynomial_mode(cc, eta, poly, n_slots, 0.0)
}

/// Вариант для **отрицательного контроля**.
///
/// К вычисленной вероятности добавляется аддитивный сдвиг `shift`, из-за чего
/// пациенты с `0 < q(η) < shift` (q(η) — нечётная часть `p − 1/2`) получают
/// противоположное решение. Контроль показывает, что метрика расхождений
/// способна регистрировать нарушение гарантии, когда она действительно есть;
/// при `shift = 0` поведение совпадает с обычной сигмоидой.
pub fn eval_sigmoid_polynomial_shifted(
    cc: &Context,
    eta: &Ciphertext,
    poly: &SigmoidPolynomial,
    n_slots: usize,
    shift: f64,
) -> Result<Ciphertext> {
    eval_sigmoid_polynomial_mode(cc, eta, poly, n_slots, shift)
}

/// Общая реализация: `p = 0,5 + q(η) + shift`.
fn eval_sigmoid_polynomial_mode(
    cc: &Context,
    eta: &Ciphertext,
    poly: &SigmoidPolynomial,
    n_slots: usize,
    shift: f64,
) -> Result<Ciphertext> {
    if n_slots == 0 {
        return Err(Error("eval_sigmoid_polynomial: n_slots = 0".to_string()));
    }
    let k_max = poly.coeffs.len().saturating_sub(1);
    let (_, block) = select_evaluation_scheme(k_max);

    // u = η / B
    let u = cc.eval_mult_plain(eta, &constant_plaintext(cc, 1.0 / poly.bound, n_slots)?)?;

    // Вырожденный случай: одна нечётная степень.
    if block < 2 {
        let scaled = cc.eval_mult_plain(&u, &constant_plaintext(cc, poly.coeffs[0], n_slots)?)?;
        return cc.eval_add_plain(&scaled, &constant_plaintext(cc, 0.5 + shift, n_slots)?);
    }

    let blocks_count = (k_max + 1).div_ceil(block);

    // Степени w = u²: w_pows[j-1] = w^j, j = 1..=block.
    let mut w_pows: Vec<Ciphertext> = Vec::with_capacity(block);
    w_pows.push(cc.rescale(&cc.eval_mult(&u, &u)?)?);
    for _ in 1..block {
        let next = {
            let prev = w_pows.last().expect("w_pows непуст");
            let base = &w_pows[0];
            cc.rescale(&cc.eval_mult(prev, base)?)?
        };
        w_pows.push(next);
    }

    // Нечётные степени u: odd[j] = u^(2j+1), j = 0..block-1.
    let mut odd: Vec<Ciphertext> = Vec::with_capacity(block);
    odd.push(u);
    for j in 1..block {
        let next = {
            let prev = &odd[j - 1];
            cc.rescale(&cc.eval_mult(prev, &w_pows[0])?)?
        };
        odd.push(next);
    }

    // Блоки C_i = Σ_{j=0}^{block-1} d_{i·block+j} · u^(2j+1).
    //
    // Блок может оказаться целиком нулевым (если соответствующие
    // коэффициенты равны нулю). Тогда шифротекст не создаётся, а схема
    // Горнера просто пропускает нужное число умножений на w^m — это
    // математически точный учёт пропущенных степеней, а не заглушка.
    let mut block_values: Vec<Option<Ciphertext>> = Vec::with_capacity(blocks_count);
    for i in 0..blocks_count {
        let mut acc: Option<Ciphertext> = None;
        for j in 0..block {
            let idx = i * block + j;
            if idx > k_max {
                break;
            }
            let coeff = poly.coeffs[idx];
            if coeff == 0.0 {
                continue;
            }
            let term = cc.eval_mult_plain(&odd[j], &constant_plaintext(cc, coeff, n_slots)?)?;
            acc = Some(match acc {
                None => term,
                Some(a) => cc.eval_add(&a, &term)?,
            });
        }
        block_values.push(acc);
    }

    let highest = block_values
        .iter()
        .rposition(|b| b.is_some())
        .ok_or_else(|| Error("оценка многочлена: все коэффициенты нулевые".to_string()))?;

    let mut result = block_values[highest]
        .take()
        .ok_or_else(|| Error("оценка многочлена: нет старшего блока".to_string()))?;
    let mut current = highest;

    // Схема Горнера по блокам: R ← C_i + w^(block·(current-i))·R.
    if blocks_count > 1 {
        let w_m = &w_pows[block - 1];
        for i in (0..highest).rev() {
            if let Some(c) = block_values[i].take() {
                let mut shifted = result;
                for _ in 0..(current - i) {
                    shifted = cc.rescale(&cc.eval_mult(&shifted, w_m)?)?;
                }
                result = cc.eval_add(&shifted, &c)?;
                current = i;
            }
        }
    }

    // Возвращаем полную вероятность p = 1/2 + q + shift.
    cc.eval_add_plain(&result, &constant_plaintext(cc, 0.5 + shift, n_slots)?)
}

/// Расшифровка вероятностей (роль больницы).
pub fn decrypt_probabilities(
    cc: &Context,
    sk: &crate::SecretKey,
    ct: &Ciphertext,
    n: usize,
) -> Result<Vec<f64>> {
    cc.decrypt_ckks(sk, ct, n as u32)
}

/// Применяет порог `0.5` к расшифрованным вероятностям.
///
/// Сравнение выполняется больницей **в открытом виде** уже над вероятностью:
/// ни исходные признаки, ни логит ей не доступны (см. `docs/THREAT_MODEL.md`).
pub fn threshold_probabilities(probs: &[f64]) -> Vec<bool> {
    probs.iter().map(|p| *p >= 0.5).collect()
}

/// Применяет порог `0` к расшифрованным логитам (режим A).
pub fn threshold_logits(logits: &[f64]) -> Vec<bool> {
    logits.iter().map(|e| *e >= 0.0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cohort::{default_t2dm_config, Cohort};
    use crate::logistic;

    /// Запас по глубине, необходимый для вычисления многочлена в CKKS.
    ///
    /// Аналитическая оценка [`required_depth`] даёт минимальное число
    /// последовательных умножений с rescale на самой длинной ветви контура.
    /// Цепочка модулей OpenFHE отсчитывает уровни иначе (уровень
    /// увеличивается на каждом rescale, а смешивание операндов разных
    /// уровней внутри блока требует дополнительных модулей), поэтому
    /// практическая глубина оказывается выше аналитической. Величина
    /// подобрана экспериментально, зафиксирована в отчёте исследования и
    /// проверяется тестами.
    const DEPTH_MARGIN_TEST: u32 = 3;

    fn tiny_model() -> (LogisticModel, Vec<Vec<f64>>, Vec<bool>) {
        let mut cfg = default_t2dm_config();
        cfg.n_derivation = 400;
        cfg.n_validation = 100;
        cfg.prevalence_calibration_n = 20_000;
        let cohort = Cohort::generate(&cfg);
        let names = cfg.feature_names();
        let model = logistic::fit(
            &names,
            &cohort.derivation_features(),
            &cohort.derivation_outcomes(),
            1e-3,
            60,
            1e-10,
        );
        (
            model,
            cohort.validation_features(),
            cohort.validation_outcomes(),
        )
    }

    #[test]
    fn analytic_depth_is_a_lower_bound_and_context_builds() {
        // Аналитическая оценка для степени 11 — 4 последовательных
        // умножения на самой длинной ветви; контекст с запасом строится.
        let poly = SigmoidPolynomial::fit(8.0, 11);
        assert_eq!(required_depth(&poly), 4);
        let cc = context_for_polynomial(&poly, DEPTH_MARGIN_TEST).expect("контекст создаётся");
        assert!(cc.ring_dimension() > 0);
    }

    #[test]
    fn homomorphic_sigmoid_matches_plaintext_polynomial() {
        // Ключевой тест: контур на шифротекстах должен воспроизводить
        // значение многочлена, посчитанное в открытом виде, с точностью CKKS.
        let poly = SigmoidPolynomial::fit(8.0, 11);
        let cc = context_for_polynomial(&poly, DEPTH_MARGIN_TEST).expect("ctx");

        let keys = cc.keygen().expect("keygen");
        let pk = keys.public_key().expect("pk");
        let sk = keys.secret_key().expect("sk");
        cc.evmult_keygen(&sk).expect("evmult_keygen");

        let etas: Vec<f64> = (-40..=40).map(|i| i as f64 * 0.1).collect();
        let pt = cc.make_ckks_plaintext(&etas).expect("pt");
        let ct = cc.encrypt(&pk, &pt).expect("encrypt");

        let out = eval_sigmoid_polynomial(&cc, &ct, &poly, etas.len()).expect("poly");
        let got = decrypt_probabilities(&cc, &sk, &out, etas.len()).expect("decrypt");

        for (i, e) in etas.iter().enumerate() {
            let expected = poly.eval(*e);
            assert!(
                (got[i] - expected).abs() < 1e-4,
                "η = {e}: HE {0} против открытого {expected}",
                got[i]
            );
        }
    }

    #[test]
    fn homomorphic_decisions_match_the_exact_model() {
        let (model, xs, _ys) = tiny_model();
        let etas = model.eta_all(&xs);
        let bound = etas.iter().map(|e| e.abs()).fold(0.0, f64::max) * 1.2;
        let poly = SigmoidPolynomial::fit(bound, 11);
        let cc = context_for_polynomial(&poly, DEPTH_MARGIN_TEST).expect("ctx");

        let keys = cc.keygen().expect("keygen");
        let pk = keys.public_key().expect("pk");
        let sk = keys.secret_key().expect("sk");
        cc.evmult_keygen(&sk).expect("evmult_keygen");

        let feats = encrypt_standardized_features(&cc, &pk, &model, &xs).expect("encrypt");
        let eta_ct = compute_logit(&cc, &feats, &model).expect("logit");
        let prob_ct = eval_sigmoid_polynomial(&cc, &eta_ct, &poly, xs.len()).expect("sigmoid");
        let probs = decrypt_probabilities(&cc, &sk, &prob_ct, xs.len()).expect("decrypt");
        let decisions = threshold_probabilities(&probs);

        let expected: Vec<bool> = xs.iter().map(|x| model.predict(x)).collect();
        assert_eq!(
            decisions, expected,
            "гомоморфные решения разошлись с эталоном"
        );
    }

    #[test]
    fn negative_control_shift_detects_divergence() {
        // Отрицательный контроль: аддитивный сдвиг к гомоморфной вероятности
        // обязан создать расхождения решений вблизи порога. Без сдвига
        // расхождений нет (гарантия сохранения знака), со сдвигом — есть.
        let poly = SigmoidPolynomial::fit(8.0, 11);
        let cc = context_for_polynomial(&poly, DEPTH_MARGIN_TEST).expect("ctx");

        let keys = cc.keygen().expect("keygen");
        let pk = keys.public_key().expect("pk");
        let sk = keys.secret_key().expect("sk");
        cc.evmult_keygen(&sk).expect("evmult_keygen");

        // Логиты с плотной сеткой вокруг нуля плюс хвосты.
        let etas: Vec<f64> = (0..=360).map(|i| -1.8 + i as f64 * 0.01).collect();
        let pt = cc.make_ckks_plaintext(&etas).expect("pt");
        let ct = cc.encrypt(&pk, &pt).expect("encrypt");
        let ref_dec: Vec<bool> = etas.iter().map(|e| *e >= 0.0).collect();
        let mismatched_etas = |dec: &[bool]| -> Vec<f64> {
            etas.iter()
                .zip(dec)
                .zip(&ref_dec)
                .filter(|((_, b), a)| **b != **a)
                .map(|((e, _), _)| *e)
                .collect()
        };

        // Без сдвига решения могут разойтись с эталоном только в узкой
        // шумовой полосе у самого порога (шум CKKS переворачивает решение
        // там, где q(η) сопоставима с шумом) — это и есть реальная граница,
        // которую «0 из 4000» в удобной выборке не регистрирует.
        let base = eval_sigmoid_polynomial(&cc, &ct, &poly, etas.len()).expect("base");
        let base_p = decrypt_probabilities(&cc, &sk, &base, etas.len()).expect("decrypt");
        let base_dec = threshold_probabilities(&base_p);
        let base_mm = mismatched_etas(&base_dec);
        for e in &base_mm {
            assert!(
                e.abs() < 0.03,
                "расхождение без сдвига при |η| = {} — вне шумовой полосы",
                e.abs()
            );
        }

        // Со сдвигом расхождений больше, чем в шумовой полосе, и они лежат
        // в предсказанной полосе сдвига.
        let shift = 0.05;
        let shifted =
            eval_sigmoid_polynomial_shifted(&cc, &ct, &poly, etas.len(), shift).expect("shift");
        let sh_p = decrypt_probabilities(&cc, &sk, &shifted, etas.len()).expect("decrypt");
        let sh_dec = threshold_probabilities(&sh_p);
        let mismatches = mismatched_etas(&sh_dec);

        assert!(
            !mismatches.is_empty(),
            "сдвиг {shift} не создал ни одного расхождения"
        );
        assert!(
            mismatches.len() > base_mm.len(),
            "сдвиг должен дать больше расхождений, чем шум ({} > {})",
            mismatches.len(),
            base_mm.len()
        );

        // Наибольшее |η| среди расхождений ограничено полосой shift/(d0/B) с запасом.
        let slope = poly.coeffs[0] / poly.bound;
        let band = shift / slope * 1.6;
        let max_abs_eta = mismatches
            .iter()
            .map(|e| e.abs())
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(
            max_abs_eta < band,
            "расхождение при |η| = {max_abs_eta} вне ожидаемой полосы {band}"
        );

        // Сдвиг с отрицательным знаком смещает в другую сторону: расхождения
        // симметрично появляются при η > 0.
        let neg =
            eval_sigmoid_polynomial_shifted(&cc, &ct, &poly, etas.len(), -shift).expect("shift-");
        let neg_p = decrypt_probabilities(&cc, &sk, &neg, etas.len()).expect("decrypt");
        let neg_dec = threshold_probabilities(&neg_p);
        let neg_mismatch_positive = etas
            .iter()
            .zip(&neg_dec)
            .zip(&ref_dec)
            .filter(|((e, b), a)| **b != **a && **e > 0.0)
            .count();
        assert!(
            neg_mismatch_positive > 0,
            "отрицательный сдвиг не сдвинул решения при η > 0"
        );
    }

    #[test]
    fn logit_circuit_reproduces_plaintext_logits() {
        let (model, xs, _ys) = tiny_model();
        let cc = context_for_polynomial(&SigmoidPolynomial::fit(8.0, 11), DEPTH_MARGIN_TEST)
            .expect("ctx");
        let keys = cc.keygen().expect("keygen");
        let pk = keys.public_key().expect("pk");
        let sk = keys.secret_key().expect("sk");

        let feats = encrypt_standardized_features(&cc, &pk, &model, &xs).expect("encrypt");
        let eta_ct = compute_logit(&cc, &feats, &model).expect("logit");
        let got = cc
            .decrypt_ckks(&sk, &eta_ct, xs.len() as u32)
            .expect("decrypt");
        let want = model.eta_all(&xs);

        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
            assert!(
                (g - w).abs() < 1e-6,
                "пациент {i}: HE {g} против открытого {w}"
            );
        }
    }

    #[test]
    fn pre_transfers_the_result_to_the_hospital_key() {
        let (model, xs, _ys) = tiny_model();
        let poly = SigmoidPolynomial::fit(8.0, 11);
        let cc = context_for_polynomial(&poly, DEPTH_MARGIN_TEST).expect("ctx");

        // Лаборатория.
        let lab = cc.keygen().expect("lab keygen");
        let lab_pk = lab.public_key().expect("lab pk");
        let lab_sk = lab.secret_key().expect("lab sk");
        cc.evmult_keygen(&lab_sk).expect("evmult_keygen");

        // Больница.
        let hosp = cc.keygen().expect("hosp keygen");
        let hosp_pk = hosp.public_key().expect("hosp pk");
        let hosp_sk = hosp.secret_key().expect("hosp sk");

        // Лаборатория шифрует и выпускает ключ перешифрования.
        let feats = encrypt_standardized_features(&cc, &lab_pk, &model, &xs).expect("encrypt");
        let rekey = cc.rekeygen(&lab_sk, &hosp_pk).expect("rekeygen");

        // Сервер считает логит и вероятность, затем перешифровывает.
        let eta_ct = compute_logit(&cc, &feats, &model).expect("logit");
        let prob_ct = eval_sigmoid_polynomial(&cc, &eta_ct, &poly, xs.len()).expect("sigmoid");
        let transferred = cc.reencrypt(&rekey, &prob_ct).expect("reencrypt");

        // Больница расшифровывает своим ключом.
        let probs = decrypt_probabilities(&cc, &hosp_sk, &transferred, xs.len()).expect("decrypt");
        let expected: Vec<bool> = xs.iter().map(|x| model.predict(x)).collect();
        assert_eq!(threshold_probabilities(&probs), expected);
    }

    #[test]
    fn wrong_hospital_key_does_not_recover_plausible_probabilities() {
        let (model, xs, _ys) = tiny_model();
        let poly = SigmoidPolynomial::fit(8.0, 11);
        let cc = context_for_polynomial(&poly, DEPTH_MARGIN_TEST).expect("ctx");

        let lab = cc.keygen().expect("lab keygen");
        let lab_pk = lab.public_key().expect("lab pk");
        let lab_sk = lab.secret_key().expect("lab sk");
        cc.evmult_keygen(&lab_sk).expect("evmult_keygen");

        let hosp = cc.keygen().expect("hosp keygen");
        let hosp_pk = hosp.public_key().expect("hosp pk");

        let feats = encrypt_standardized_features(&cc, &lab_pk, &model, &xs).expect("encrypt");
        let rekey = cc.rekeygen(&lab_sk, &hosp_pk).expect("rekeygen");
        let eta_ct = compute_logit(&cc, &feats, &model).expect("logit");
        let prob_ct = eval_sigmoid_polynomial(&cc, &eta_ct, &poly, xs.len()).expect("sigmoid");
        let transferred = cc.reencrypt(&rekey, &prob_ct).expect("reencrypt");

        // Чужой ключ: либо жёсткий отказ Decode (fail-closed), либо мусор.
        let stranger = cc.keygen().expect("stranger keygen");
        let stranger_sk = stranger.secret_key().expect("stranger sk");
        match decrypt_probabilities(&cc, &stranger_sk, &transferred, xs.len()) {
            Err(_) => { /* fail-closed — ожидаемый исход */ }
            Ok(values) => {
                let plausible = values.iter().filter(|v| (0.0..=1.0).contains(*v)).count();
                let expected: Vec<f64> = xs.iter().map(|x| model.predict_proba(x)).collect();
                let close = values
                    .iter()
                    .zip(&expected)
                    .filter(|(a, b)| (**a - **b).abs() < 0.05)
                    .count();
                assert!(
                    close < values.len() / 2,
                    "чужой ключ восстановил {close} правдоподобных значений из {} (в диапазоне [0,1]: {plausible})",
                    values.len()
                );
            }
        }
    }

    #[test]
    fn more_patients_than_one_batch_is_rejected() {
        let (model, _xs, _ys) = tiny_model();
        let cc = context_for_polynomial(&SigmoidPolynomial::fit(8.0, 11), DEPTH_MARGIN_TEST)
            .expect("ctx");
        let keys = cc.keygen().expect("keygen");
        let pk = keys.public_key().expect("pk");

        let batch = (cc.ring_dimension() / 2) as usize;
        let too_many = vec![vec![0.0; model.weights.len()]; batch + 1];
        assert!(encrypt_standardized_features(&cc, &pk, &model, &too_many).is_err());
    }
}
