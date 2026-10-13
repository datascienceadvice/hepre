//! Этап A2 — приватный скоринг риска диабета 2 типа поверх CKKS + PRE.
//!
//! Роли:
//! - **лаборатория**: знает фичи пациентов и шифрует их (пакетно по `N`);
//! - **сервер**: гомоморфно считает непрерывный скор `Σ_j w_j·x_ij + b` и
//!   перешифровывает результат на ключ больницы (PRE), не видя ни фичи, ни скор;
//! - **больница**: получает PRE-перешифрованный скор, расшифровывает своим ключом
//!   и применяет порог (`threshold`) в открытом виде.

use crate::{Ciphertext, CkksParams, Context, Plaintext, PublicKey, Result, SecretKey};

/// Модель логистико-подобного линейного скора.
#[derive(Clone, Debug)]
pub struct ScoringModel {
    /// Веса признаков; `len(weights)` = число фич, заполняется в слоты SIMD-пакета.
    pub weights: Vec<f64>,
    /// Свободный член (интерцепт).
    pub intercept: f64,
    /// Порог бинарного риска, применяется больницей в открытом виде.
    pub threshold: f64,
}

impl ScoringModel {
    /// Непрерывный скор в открытом виде (для проверки точности бенчмарков).
    pub fn score_plain(&self, x: &[f64]) -> f64 {
        self.intercept
            + x.iter()
                .zip(&self.weights)
                .map(|(xi, w)| w * xi)
                .sum::<f64>()
    }

    /// Бинарный риск в открытом виде.
    pub fn predict_plain(&self, x: &[f64]) -> bool {
        self.score_plain(x) >= self.threshold
    }
}

/// Набор шифротекстов: один [Ciphertext] на признак, слот `i` несёт значение
/// признака для `i`-го пациента (`batch_size` слотов на пакет).
pub struct EncodedFeatures {
    /// Шифротексты фич с лаборатории.
    pub cts: Vec<Ciphertext>,
    /// Количество пациентов в наборе.
    pub n: usize,
    /// Свободный член как plaintext (готов к EvalAdd).
    pub intercept_pt: Option<Plaintext>,
}

/// Шифрует `N` векторов признаков: контрольная лаборатория упаковывает каждый
/// признак в отдельный шифротекст (колонка фичи по слоту пациента).
///
/// `features` — `N` строк × `f` колонок. Требуется `f <= batch_size` (фичи втыкаются
/// в первые `f` слотов одного пакета) во избежание overcapacity; на практике обычно
/// `f` много меньше `batch_size`, поэтому используем слоты `0..f`.
pub fn encrypt_features(
    cc: &Context,
    pk: &PublicKey,
    model: &ScoringModel,
    features: &[Vec<f64>],
) -> Result<EncodedFeatures> {
    let n = features.len();
    let f = features.first().map_or(0, Vec::len);
    if n == 0 {
        return Ok(EncodedFeatures {
            cts: Vec::new(),
            n: 0,
            intercept_pt: None,
        });
    }

    // Колонка j → слот j. Упаковываем наоборот: строим по столбцам.
    // Для каждого признака j создаём вектор длины f (или batch), где i-й слот = x[i][j].
    let batch = cc.ring_dimension() / 2;
    if f > batch as usize {
        let msg = format!("too many features {f} for batch {batch}");
        return Err(crate::Error(msg));
    }

    let mut cts = Vec::with_capacity(f);
    for j in 0..f {
        // Слот i пакета = значение фичи j для пациента i (пациенты в слотах 0..n).
        let mut slot = vec![0.0_f64; n];
        for i in 0..n {
            slot[i] = features[i].get(j).copied().unwrap_or(0.0);
        }
        let pt = cc.make_ckks_plaintext(&slot)?;
        let ct = cc.encrypt(pk, &pt)?;
        cts.push(ct);
    }

    Ok(EncodedFeatures {
        cts,
        n,
        intercept_pt: Some(cc.make_ckks_plaintext(&vec![model.intercept; n])?),
    })
}

/// Гомоморфный скор на сервере: `result_i = Σ_j w_j·ct_j[i] + b`.
/// Возвращает один шифротекст (скор по всем пациентам в слотах).
pub fn compute_score(
    cc: &Context,
    feats: &EncodedFeatures,
    model: &ScoringModel,
) -> Result<Ciphertext> {
    if feats.cts.is_empty() {
        return Err(crate::Error("no features encoded".to_string()));
    }
    if model.weights.len() != feats.cts.len() {
        return Err(crate::Error("model/weights length mismatch".to_string()));
    }

    let mut acc = None;
    for (j, ct) in feats.cts.iter().enumerate() {
        // Вес w_j повторяется во всех n слотах (слот = пациент).
        let wpt = cc.make_ckks_plaintext(&vec![model.weights[j]; feats.n])?;
        let t = cc.eval_mult_plain(ct, &wpt)?;
        acc = Some(match acc {
            None => t,
            Some(a) => cc.eval_add(&a, &t)?,
        });
    }
    let mut acc = acc.ok_or_else(|| crate::Error("no features encoded".to_string()))?;
    if let Some(pt) = &feats.intercept_pt {
        acc = cc.eval_add_plain(&acc, pt)?;
    }
    Ok(acc)
}

/// PRE-конвейер: сервер перешифровывает скор под ключ больницы.
/// Возвращает шифротекст, расшифровываемый больницей.
pub fn transfer_score(cc: &Context, ek: &crate::EvalKey, score: &Ciphertext) -> Result<Ciphertext> {
    cc.reencrypt(ek, score)
}

/// Больница: расшифровывает скоры (первые `k` слота) своим секретным ключом.
pub fn decrypt_scores(
    cc: &Context,
    sk: &SecretKey,
    score: &Ciphertext,
    k: usize,
) -> Result<Vec<f64>> {
    cc.decrypt_ckks(sk, score, k as u32)
}

/// Параметры контекста по умолчанию для скоринга.
pub fn scoring_context() -> Result<Context> {
    Context::new_ckks(CkksParams {
        batch_size: 8192,
        multiplicative_depth: 3,
        scaling_mod_size: 40,
    })
}
