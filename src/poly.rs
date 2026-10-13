//! Приближение сигмоиды многочленом, пригодное для гомоморфного вычисления.
//!
//! # Зачем
//!
//! Гомоморфный контур умеет складывать и умножать, но не вычисляет
//! `σ(η) = 1/(1+e^{-η})`. Логистическая вероятность раскладывается в
//! многочлен от логита, который вычисляется на шифротекстах.
//!
//! # Конструкция и что именно гарантируется
//!
//! Аппроксимируется **не** сама `σ`, а нечётная функция
//! `g(η) = σ(η) - 1/2`. Нечётность даёт ровно `p(0) = 1/2`, то есть
//! многочлен проходит точно через порог решения.
//!
//! Для практики важно не «быть близко к сигмоиде» само по себе, а
//! **не изменить бинарное решение**. Решение сохраняется тождественно
//! тогда и только тогда, когда
//!
//! ```text
//! p(η) > 1/2  при  η > 0     и     p(η) < 1/2  при  η < 0,
//! ```
//!
//! то есть когда многочлен не меняет знак относительно порога. Именно это
//! свойство (`sign_preserving`) здесь проверяется численно на плотной сетке
//! и попадает в отчёт вместе с числом нарушений. Дополнительно
//! фиксируется монотонность: при нечётности она влечёт сохранение знака,
//! но является более сильным требованием.
//!
//! **Почему не через неотрицательность коэффициентов.** Требование
//! «все коэффициенты при `u^(2k+1)` неотрицательны» тоже даёт
//! монотонность, но оно неоправданно сильно: на широком `[-B, B]` сигмоида
//! почти ступенчата, и её разложение по нечётным мономам знакопеременно.
//! Принудительная неотрицательность разрушает точность (проверено: на
//! `B = 4`, степень 11, ошибка вырастает с ~1e-6 до ~0.1). Поэтому
//! свойство проверяется, а не навязывается.
//!
//! # Стоимость
//!
//! Подстановка `w = u²` превращает нечётный многочлен степени `P` в
//! многочлен степени `(P-1)/2` по `w`, который считается схемой
//! Патерсона—Стокмейера. Это снижает мультипликативную глубину контура
//! с `P` (схема Горнера) до `1 + depth_w`, что и делает вычисление
//! практически осуществимым в CKKS.

use serde::{Deserialize, Serialize};

use crate::detmath;

/// Многочленная аппроксимация `σ(η)` вместе с отчётом о её свойствах.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SigmoidPolynomial {
    /// Граница аппроксимации `B`: многочлен построен для `|η| ≤ B`.
    pub bound: f64,
    /// Старшая нечётная степень `P` (нечётное число).
    pub power: usize,
    /// Коэффициенты `d_k` при `u^{2k+1}`, где `u = η/B`.
    pub coeffs: Vec<f64>,
    /// Все ли коэффициенты неотрицательны. Свойство **информационное**:
    /// при неотрицательности монотонность гарантирована, но обратное
    /// неверно, и на широких интервалах коэффициенты знакопеременны.
    pub all_coefficients_nonnegative: bool,
    /// Минимум производной `dp/du` на `u ∈ [0, 1]`.
    pub min_derivative_positive_side: f64,
    /// Монотонность на `[0, B]` по численной проверке производной.
    pub monotone_positive_side: bool,
    /// Монотонность на всём `[-B, B]` (следует из нечётности).
    pub monotone_full: bool,
    /// Сохраняет ли многочлен знак относительно порога на `[-B, B]`:
    /// `p(η) > 1/2` при `η > 0` и `p(η) < 1/2` при `η < 0`.
    ///
    /// Именно это свойство обеспечивает тождественность бинарного решения
    /// решению `η ≥ 0`.
    pub sign_preserving: bool,
    /// Минимум `p(η) - 1/2` на сетке при `η > 0` (должен быть > 0).
    pub min_delta_positive_side: f64,
    /// Максимум `p(η) - 1/2` на сетке при `η < 0` (должен быть < 0).
    pub max_delta_negative_side: f64,
    /// Максимальная абсолютная ошибка на равномерной сетке `[-B, B]`.
    pub max_abs_error: f64,
    /// Аргумент, на котором достигается максимум ошибки.
    pub max_error_at: f64,
    /// Максимальная абсолютная ошибка на переданных логитах (если заданы).
    pub max_abs_error_on_data: f64,
    /// Мультипликативная глубина гомоморфного вычисления.
    pub mult_depth: usize,
    /// Размер блока схемы Патерсона—Стокмейера.
    pub block_size: usize,
}

/// Выбирает схему вычисления нечётного многочлена с `k_max + 1` членами.
///
/// Многочлен `q(u) = u · Σ_{k=0}^{k_max} d_k · w^k`, `w = u²`, разбивается на
/// блоки по `m` коэффициентов. Требуется вычислить `u^(2j+1)` для
/// `j = 0..m-1` (глубина `m-1`) и при `g > 1` — степень `w^m` (глубина `m`),
/// после чего блоки связываются схемой Горнера по `w^m`.
///
/// Мультипликативная глубина:
/// - при одном блоке (`g = 1`) — `m - 1`;
/// - иначе — `m + g - 1`.
///
/// Возвращает `(глубина, размер_блока)`, выбирая минимум глубины и при
/// равенстве — меньший размер блока.
pub fn select_evaluation_scheme(k_max: usize) -> (usize, usize) {
    if k_max == 0 {
        return (0, 1);
    }
    let mut best_depth = usize::MAX;
    let mut best_m = 2usize;
    for m in 2..=32usize {
        let blocks = (k_max + 1).div_ceil(m);
        let depth = if blocks == 1 { m - 1 } else { m + blocks - 1 };
        if depth < best_depth {
            best_depth = depth;
            best_m = m;
        }
    }
    (best_depth, best_m)
}

/// Разворачивает многочлены Чебышёва `T_0 .. T_n` в мономиальный базис.
///
/// `T_{n+1} = 2u·T_n - T_{n-1}`. Для степеней до ~20 операция устойчива
/// в двойной точности.
fn chebyshev_monomials(n_max: usize) -> Vec<Vec<f64>> {
    let mut t: Vec<Vec<f64>> = Vec::with_capacity(n_max + 1);
    t.push(vec![1.0]);
    if n_max >= 1 {
        t.push(vec![0.0, 1.0]);
    }
    for n in 1..n_max {
        let mut next = vec![0.0_f64; n + 2];
        for (i, c) in t[n].iter().enumerate() {
            next[i + 1] += 2.0 * c;
        }
        for (i, c) in t[n - 1].iter().enumerate() {
            next[i] -= c;
        }
        t.push(next);
    }
    t
}

/// Значение многочлена, заданного мономиальными коэффициентами, по схеме Горнера.
fn eval_monomials(coeffs: &[f64], x: f64) -> f64 {
    let mut acc = 0.0_f64;
    for c in coeffs.iter().rev() {
        acc = acc * x + c;
    }
    acc
}

impl SigmoidPolynomial {
    /// Строит аппроксимацию на `[-bound, bound]` старшей степенью `power`.
    ///
    /// `power` должно быть нечётным и не меньше 3.
    ///
    /// Коэффициенты находятся методом наименьших квадратов в **базисе
    /// Чебышёва**: на чебышёвских узлах этот базис ортогонален, матрица
    /// Грама диагональна, поэтому обращение не требуется и подгонка
    /// устойчива даже для высоких степеней. Затем многочлен переводится
    /// в мономиальный базис по `u` — он нужен для схемы Горнера на
    /// шифротекстах.
    pub fn fit(bound: f64, power: usize) -> Self {
        assert!(bound > 0.0, "fit: граница должна быть положительной");
        assert!(
            power >= 3 && power % 2 == 1,
            "fit: степень должна быть нечётной и ≥ 3"
        );

        let k_max = (power - 1) / 2;
        let n_nodes = (8 * (k_max + 1)).max(64);

        // Узлы Чебышёва и значения нечётной части сигмоиды на них.
        let nodes: Vec<f64> = (0..n_nodes)
            .map(|j| detmath::cos(std::f64::consts::PI * (j as f64 + 0.5) / n_nodes as f64))
            .collect();
        let values: Vec<f64> = nodes
            .iter()
            .map(|x| detmath::sigmoid(bound * x) - 0.5)
            .collect();

        let cheb = chebyshev_monomials(2 * k_max + 1);

        // Коэффициенты Чебышёва: на этих узлах <T_m, T_m> = N/2 при m ≥ 1,
        // поэтому каждый коэффициент считается независимо.
        let mut cheb_coeffs = vec![0.0_f64; k_max + 1];
        for (k, coeff) in cheb_coeffs.iter_mut().enumerate() {
            let idx = 2 * k + 1;
            let mut acc = 0.0_f64;
            for (j, x) in nodes.iter().enumerate() {
                acc += values[j] * eval_monomials(&cheb[idx], *x);
            }
            *coeff = 2.0 * acc / n_nodes as f64;
        }

        // Переход к мономиальному базису по u.
        let mut mono = vec![0.0_f64; power + 1];
        for (k, ck) in cheb_coeffs.iter().enumerate() {
            let idx = 2 * k + 1;
            for (i, c) in cheb[idx].iter().enumerate() {
                mono[i] += ck * c;
            }
        }
        let coeffs: Vec<f64> = (0..=k_max).map(|k| mono[2 * k + 1]).collect();

        let (mult_depth, block_size) = select_evaluation_scheme(k_max);

        let mut poly = SigmoidPolynomial {
            bound,
            power,
            coeffs,
            all_coefficients_nonnegative: false,
            min_derivative_positive_side: f64::INFINITY,
            monotone_positive_side: true,
            monotone_full: true,
            sign_preserving: true,
            min_delta_positive_side: f64::INFINITY,
            max_delta_negative_side: f64::NEG_INFINITY,
            max_abs_error: 0.0,
            max_error_at: 0.0,
            max_abs_error_on_data: f64::NAN,
            mult_depth,
            block_size,
        };

        poly.all_coefficients_nonnegative = poly.coeffs.iter().all(|c| *c >= 0.0);
        poly.verify();
        poly
    }

    /// Значение аппроксимированной вероятности в точке `eta`.
    pub fn eval(&self, eta: f64) -> f64 {
        0.5 + self.delta(eta)
    }

    /// Нечётная часть `p(η) = σ(η) - 1/2` в приближении.
    pub fn delta(&self, eta: f64) -> f64 {
        let u = eta / self.bound;
        let u2 = u * u;
        let mut acc = 0.0_f64;
        for c in self.coeffs.iter().rev() {
            acc = acc * u2 + c;
        }
        u * acc
    }

    /// Бинарное решение по порогу 0.5.
    pub fn decision(&self, eta: f64) -> bool {
        self.delta(eta) >= 0.0
    }

    /// Производная многочлена по `u` в точке `u`.
    fn derivative_u(&self, u: f64) -> f64 {
        let u2 = u * u;
        let mut acc = 0.0_f64;
        for (k, c) in self.coeffs.iter().enumerate().rev() {
            let deg = (2 * k + 1) as f64;
            acc = acc * u2 + c * deg;
        }
        acc
    }

    /// Проверяет сохранение знака и монотонность, считает ошибку аппроксимации.
    fn verify(&mut self) {
        const GRID: usize = 20_001;

        let mut max_err = 0.0_f64;
        let mut max_err_at = 0.0_f64;
        let mut min_deriv = f64::INFINITY;
        let mut monotone_pos = true;
        let mut min_delta_pos = f64::INFINITY;
        let mut max_delta_neg = f64::NEG_INFINITY;
        let mut sign_ok = true;

        for i in 0..GRID {
            let u = -1.0 + 2.0 * i as f64 / (GRID - 1) as f64;
            let eta = u * self.bound;
            let delta = self.delta(eta);

            let err = (0.5 + delta - detmath::sigmoid(eta)).abs();
            if err > max_err {
                max_err = err;
                max_err_at = eta;
            }

            if u > 0.0 {
                if delta < min_delta_pos {
                    min_delta_pos = delta;
                }
                if delta <= 0.0 {
                    sign_ok = false;
                }
                let d = self.derivative_u(u);
                if d < min_deriv {
                    min_deriv = d;
                }
                if d <= 0.0 {
                    monotone_pos = false;
                }
            } else if u < 0.0 {
                if delta > max_delta_neg {
                    max_delta_neg = delta;
                }
                if delta >= 0.0 {
                    sign_ok = false;
                }
            }
        }

        self.max_abs_error = max_err;
        self.max_error_at = max_err_at;
        self.min_derivative_positive_side = min_deriv;
        self.monotone_positive_side = monotone_pos;
        // Нечётный многочлен: монотонность справа влечёт монотонность слева.
        self.monotone_full = monotone_pos;
        self.min_delta_positive_side = min_delta_pos;
        self.max_delta_negative_side = max_delta_neg;
        self.sign_preserving = sign_ok;
    }

    /// Максимальная абсолютная ошибка вероятности на конкретных логитах.
    pub fn error_on_logits(&self, etas: &[f64]) -> f64 {
        etas.iter()
            .map(|e| (self.eval(*e) - detmath::sigmoid(*e)).abs())
            .fold(0.0, f64::max)
    }

    /// Записывает ошибку на фактических данных в отчёт.
    pub fn record_data_error(&mut self, etas: &[f64]) {
        self.max_abs_error_on_data = self.error_on_logits(etas);
    }

    /// Проверяет, что решение многочлена совпадает с `η ≥ 0` на наборе логитов.
    ///
    /// Возвращает число расхождений — их должно быть ровно ноль.
    pub fn decision_mismatches(&self, etas: &[f64]) -> usize {
        etas.iter()
            .filter(|e| self.decision(**e) != (**e >= 0.0))
            .count()
    }
}

/// Строит набор аппроксимаций разной степени — для таблицы компромисса
/// «точность / мультипликативная глубина».
pub fn sweep(bound: f64, powers: &[usize]) -> Vec<SigmoidPolynomial> {
    powers
        .iter()
        .map(|p| SigmoidPolynomial::fit(bound, *p))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polynomial_is_odd_and_passes_through_the_threshold() {
        for power in [3usize, 5, 7, 9, 11, 13] {
            let p = SigmoidPolynomial::fit(8.0, power);
            // Нечётность гарантирует ровно 0.5 в нуле и антисимметрию.
            assert!(
                p.delta(0.0).abs() < 1e-15,
                "power {power}: delta(0) = {}",
                p.delta(0.0)
            );
            assert!((p.eval(0.0) - 0.5).abs() < 1e-15);
            for eta in [0.5_f64, 1.0, 2.0, 5.0] {
                let sum = p.delta(eta) + p.delta(-eta);
                assert!(
                    sum.abs() < 1e-12,
                    "power {power}, η = {eta}: нечётность нарушена ({sum})"
                );
            }
        }
    }

    #[test]
    fn decision_matches_the_reference_rule_on_a_dense_grid() {
        // Порог решения эталонной модели — η ≥ 0, поэтому при η = 0
        // решение «да» (вероятность ровно 0.5). Многочлен обязан совпасть
        // с этим правилом на всей области определения.
        for bound in [3.0_f64, 5.0, 7.0, 9.0] {
            for power in [5usize, 7, 9, 11, 13, 15] {
                let p = SigmoidPolynomial::fit(bound, power);
                let grid: Vec<f64> = (-4000..=4000).map(|i| i as f64 * bound / 4000.0).collect();
                assert_eq!(
                    p.decision_mismatches(&grid),
                    0,
                    "B = {bound}, degree {power}: решение разошлось с η ≥ 0"
                );
                assert!(
                    p.decision(0.0),
                    "при η = 0 решение обязано совпасть с эталоном"
                );
            }
        }
    }

    #[test]
    fn sign_preservation_is_verified_for_a_wide_range_of_settings() {
        for bound in [2.0_f64, 4.0, 6.0, 8.0, 10.0, 12.0] {
            for power in [5usize, 7, 9, 11, 13, 15] {
                let p = SigmoidPolynomial::fit(bound, power);
                assert!(
                    p.sign_preserving,
                    "B = {bound}, degree {power}: знак не сохраняется \
                     (min Δ⁺ = {}, max Δ⁻ = {})",
                    p.min_delta_positive_side, p.max_delta_negative_side
                );
                assert!(p.min_delta_positive_side > 0.0);
                assert!(p.max_delta_negative_side < 0.0);
                assert!(p.max_abs_error.is_finite());
            }
        }
    }

    #[test]
    fn monotonicity_is_reported_not_guaranteed() {
        // Монотонность — более сильное свойство, чем сохранение знака, и
        // на многочленах умеренной степени она НЕ гарантирована: у
        // чебышёвской аппроксимации производная может слегка уходить в
        // минус на краю интервала. Поэтому она измеряется и попадает в
        // отчёт, но решающим свойством является сохранение знака.
        for bound in [2.0_f64, 4.0, 8.0] {
            for power in [9usize, 11, 13, 15] {
                let p = SigmoidPolynomial::fit(bound, power);
                assert!(
                    p.min_derivative_positive_side.is_finite(),
                    "B = {bound}, degree {power}: производная не посчитана"
                );
                // Независимо от монотонности решение обязано сохраниться.
                assert!(p.sign_preserving, "B = {bound}, degree {power}");
            }
        }
    }

    #[test]
    fn coefficients_are_reported_truthfully() {
        // Коэффициенты в мономиальном базисе получаются знакопеременными
        // даже на узком интервале: это следствие разложения по Чебышёву, а
        // не дефект. Важно, что поле отчёта соответствует факту, и что
        // сохранение знака достигается БЕЗ требования неотрицательности.
        for bound in [2.0_f64, 4.0, 8.0, 12.0] {
            for power in [5usize, 7, 9, 11, 13] {
                let p = SigmoidPolynomial::fit(bound, power);
                let expected = p.coeffs.iter().all(|c| *c >= 0.0);
                assert_eq!(
                    p.all_coefficients_nonnegative, expected,
                    "B = {bound}, degree {power}: отчёт о знаках не совпал с коэффициентами"
                );
                assert!(
                    p.sign_preserving,
                    "B = {bound}, degree {power}: знак не сохранён"
                );
                assert!(
                    p.coeffs[0] > 0.0,
                    "линейный коэффициент должен быть положителен"
                );
            }
        }
    }

    #[test]
    fn fit_stays_accurate() {
        // Узкий интервал при высокой степени аппроксимируется хорошо;
        // широкий — хуже, потому что сигмоида там почти ступенчата.
        let p = SigmoidPolynomial::fit(4.0, 11);
        assert!(
            p.max_abs_error < 1e-4,
            "B = 4, degree 11: ошибка {}",
            p.max_abs_error
        );

        let p = SigmoidPolynomial::fit(6.0, 13);
        assert!(
            p.max_abs_error < 1e-2,
            "B = 6, degree 13: ошибка {}",
            p.max_abs_error
        );
    }

    #[test]
    fn error_decreases_with_degree() {
        let bound = 8.0;
        let coarse = SigmoidPolynomial::fit(bound, 5);
        let fine = SigmoidPolynomial::fit(bound, 13);
        assert!(
            fine.max_abs_error <= coarse.max_abs_error + 1e-12,
            "ошибка не убывает: {} против {}",
            fine.max_abs_error,
            coarse.max_abs_error
        );
        assert!(fine.max_abs_error > 0.0);
    }

    #[test]
    fn error_is_smaller_on_a_narrower_interval() {
        let wide = SigmoidPolynomial::fit(12.0, 11);
        let narrow = SigmoidPolynomial::fit(4.0, 11);
        assert!(
            narrow.max_abs_error < wide.max_abs_error,
            "сужение интервала должно уменьшать ошибку: {} против {}",
            narrow.max_abs_error,
            wide.max_abs_error
        );
    }

    #[test]
    fn evaluation_scheme_is_optimal_and_consistent() {
        let expected: [(usize, usize); 10] = [
            (1, 1),
            (2, 2),
            (3, 3),
            (4, 4),
            (5, 4),
            (6, 5),
            (7, 5),
            (8, 5),
            (9, 6),
            (0, 0),
        ];
        for (k_max, depth) in expected {
            let (d, m) = select_evaluation_scheme(k_max);
            assert_eq!(d, depth, "k_max = {k_max}");
            if k_max > 0 {
                let g = (k_max + 1).div_ceil(m);
                let recomputed = if g == 1 { m - 1 } else { m + g - 1 };
                assert_eq!(recomputed, d, "k_max = {k_max}, m = {m}");
                assert!(d <= k_max, "k_max = {k_max}: глубина {d} > {k_max}");
            }
        }
    }

    #[test]
    fn mult_depth_matches_the_analytic_value() {
        assert_eq!(SigmoidPolynomial::fit(8.0, 3).mult_depth, 1, "power 3");
        assert_eq!(SigmoidPolynomial::fit(8.0, 5).mult_depth, 2, "power 5");
        assert_eq!(SigmoidPolynomial::fit(8.0, 11).mult_depth, 4, "power 11");
        assert_eq!(SigmoidPolynomial::fit(8.0, 13).mult_depth, 5, "power 13");

        for power in [3usize, 5, 7, 9, 11, 13, 15, 17, 19] {
            let p = SigmoidPolynomial::fit(7.0, power);
            let (expected, m) = select_evaluation_scheme((power - 1) / 2);
            assert_eq!(p.mult_depth, expected, "power {power}");
            assert_eq!(p.block_size, m, "power {power}");
        }
    }

    #[test]
    fn sweep_covers_all_requested_degrees() {
        let polys = sweep(7.0, &[3, 5, 7, 9, 11]);
        assert_eq!(polys.len(), 5);
        for (p, expected) in polys.iter().zip([3usize, 5, 7, 9, 11]) {
            assert_eq!(p.power, expected);
            assert_eq!(p.coeffs.len(), (expected + 1) / 2);
        }
    }

    #[test]
    fn data_error_recording_works() {
        let mut p = SigmoidPolynomial::fit(8.0, 11);
        let etas = vec![-3.0, -1.0, 0.0, 1.0, 3.0];
        p.record_data_error(&etas);
        let expected = p.error_on_logits(&etas);
        assert_eq!(p.max_abs_error_on_data, expected);
        assert!(p.max_abs_error_on_data <= p.max_abs_error + 1e-15);
    }
}
