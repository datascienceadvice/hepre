//! Логистическая регрессия в открытом виде — эталонная модель исследования.
//!
//! Модель обучается на **стандартизованных** признаках:
//! `z_j = (x_j - mean_j) / sd_j`. Это не косметика, а требование
//! криптографического контура: масштаб значений определяет точность CKKS,
//! поэтому центр и разброс признаков приводятся к единому виду, а
//! интерцепт впитывает сдвиг. Средние и СКО — публичные метаданные модели,
//! они передаются лаборатории вместе с весами.
//!
//! Подгонка — метод Ньютона—Рафсона (IRLS) с L2-регуляризацией
//! (гребневой штраф), которая снимает вырождение при разделении классов.
//! Интерцепт штрафом не облагается.
//!
//! Решение линейной системы на каждом шаге — методом Гаусса с выбором
//! ведущего элемента; размер системы `(p+1) x (p+1)`, поэтому стоимость
//! шага пренебрежима.

use serde::{Deserialize, Serialize};

use crate::detmath;
use crate::stats;

/// Обученная логистическая модель в стандартизованном масштабе.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogisticModel {
    /// Имена признаков в порядке столбцов.
    pub feature_names: Vec<String>,
    /// Веса по стандартизованным признакам `z_j`.
    pub weights: Vec<f64>,
    /// Свободный член (логит при `z = 0`, то есть при средних значениях).
    pub intercept: f64,
    /// Средние значения признаков на обучающей выборке.
    pub means: Vec<f64>,
    /// Стандартные отклонения признаков на обучающей выборке.
    pub sds: Vec<f64>,
    /// Коэффициент L2-регуляризации.
    pub lambda: f64,
    /// Число выполненных шагов Ньютона.
    pub iterations: usize,
    /// Достигнута ли сходимость по шагу.
    pub converged: bool,
    /// Размер обучающей выборки.
    pub n_train: usize,
    /// Число событий (положительных исходов) в обучающей выборке.
    pub n_events: usize,
}

impl LogisticModel {
    /// Приводит сырые признаки к стандартизованному масштабу.
    pub fn standardize(&self, x: &[f64]) -> Vec<f64> {
        x.iter()
            .enumerate()
            .map(|(j, v)| (v - self.means[j]) / self.sds[j])
            .collect()
    }

    /// Логит по уже стандартизованным признакам.
    ///
    /// Именно эту линейную форму вычисляет сервер гомоморфно:
    /// `η = b + Σ_j w_j · z_j`.
    pub fn eta_standardized(&self, z: &[f64]) -> f64 {
        self.intercept
            + z.iter()
                .zip(&self.weights)
                .map(|(zj, wj)| wj * zj)
                .sum::<f64>()
    }

    /// Логит по сырым признакам.
    pub fn eta(&self, x: &[f64]) -> f64 {
        self.eta_standardized(&self.standardize(x))
    }

    /// Вероятность положительного исхода.
    pub fn predict_proba(&self, x: &[f64]) -> f64 {
        detmath::sigmoid(self.eta(x))
    }

    /// Бинарное решение по порогу 0.5.
    ///
    /// Эквивалентно `η ≥ 0`, что важно: гомоморфный контур воспроизводит
    /// именно знак логита, поэтому сравнение с нулём не зависит от того,
    /// как аппроксимируется сигмоида.
    pub fn predict(&self, x: &[f64]) -> bool {
        self.eta(x) >= 0.0
    }

    /// Отношения шансов в расчёте на **нативную** единицу признака.
    ///
    /// `OR_j = exp(w_j / sd_j)`: переход от стандартизованного масштаба
    /// обратно к единицам измерения (%, ммоль/л, кг/м², годы).
    pub fn odds_ratios_per_unit(&self) -> Vec<f64> {
        self.weights
            .iter()
            .zip(&self.sds)
            .map(|(w, sd)| detmath::exp(w / sd))
            .collect()
    }

    /// Средний логит на наборе — диагностика сдвига распределения.
    pub fn mean_eta(&self, xs: &[Vec<f64>]) -> f64 {
        if xs.is_empty() {
            return f64::NAN;
        }
        xs.iter().map(|x| self.eta(x)).sum::<f64>() / xs.len() as f64
    }

    /// Диапазон логита на наборе: `(минимум, максимум)`.
    ///
    /// Нужен, чтобы выбрать границу аппроксимации сигмоиды многочленом.
    pub fn eta_range(&self, xs: &[Vec<f64>]) -> (f64, f64) {
        xs.iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| {
                let e = self.eta(x);
                (lo.min(e), hi.max(e))
            })
    }

    /// Предсказанные вероятности для набора.
    pub fn predict_proba_all(&self, xs: &[Vec<f64>]) -> Vec<f64> {
        xs.iter().map(|x| self.predict_proba(x)).collect()
    }

    /// Логиты для набора (открытый эталон для сверки с HE).
    pub fn eta_all(&self, xs: &[Vec<f64>]) -> Vec<f64> {
        xs.iter().map(|x| self.eta(x)).collect()
    }

    /// Средние и СКО признаков — то, что уходит лаборатории как метаданные.
    pub fn standardization_metadata(&self) -> Vec<(String, f64, f64)> {
        self.feature_names
            .iter()
            .zip(self.means.iter().zip(&self.sds))
            .map(|(n, (m, s))| (n.clone(), *m, *s))
            .collect()
    }
}

/// Решает `A x = b` методом Гаусса с частичным выбором ведущего элемента.
///
/// Возвращает `None` при вырожденной матрице.
fn solve_linear(a: &mut [Vec<f64>], b: &mut [f64]) -> Option<Vec<f64>> {
    let n = b.len();
    for col in 0..n {
        // Выбор ведущего элемента по столбцу.
        let mut pivot = col;
        for row in col + 1..n {
            if a[row][col].abs() > a[pivot][col].abs() {
                pivot = row;
            }
        }
        if a[pivot][col].abs() < 1e-14 {
            return None;
        }
        if pivot != col {
            a.swap(col, pivot);
            b.swap(col, pivot);
        }
        let diag = a[col][col];
        for row in col + 1..n {
            let factor = a[row][col] / diag;
            if factor == 0.0 {
                continue;
            }
            for k in col..n {
                a[row][k] -= factor * a[col][k];
            }
            b[row] -= factor * b[col];
        }
    }

    // Обратная подстановка.
    let mut x = vec![0.0_f64; n];
    for row in (0..n).rev() {
        let mut s = b[row];
        for k in row + 1..n {
            s -= a[row][k] * x[k];
        }
        x[row] = s / a[row][row];
    }
    Some(x)
}

/// Обучает логистическую регрессию методом Ньютона—Рафсона.
///
/// * `xs` — строки признаков (сырые значения);
/// * `ys` — бинарные исходы;
/// * `lambda` — коэффициент L2-регуляризации (интерцепт не штрафуется);
/// * `max_iter`, `tol` — управление сходимостью.
///
/// Стандартизация выполняется внутри по обучающей выборке. Признак с нулевым
/// разбросом получает `sd = 1` (иначе деление на ноль); такой столбец
/// вырождается в константу и поглощается интерцептом.
pub fn fit(
    feature_names: &[String],
    xs: &[Vec<f64>],
    ys: &[bool],
    lambda: f64,
    max_iter: usize,
    tol: f64,
) -> LogisticModel {
    assert!(!xs.is_empty(), "fit: пустая обучающая выборка");
    assert_eq!(xs.len(), ys.len(), "fit: длины xs и ys не совпадают");
    let n = xs.len();
    let p = xs[0].len();
    assert!(p > 0, "fit: нет признаков");
    assert_eq!(
        feature_names.len(),
        p,
        "fit: число имён признаков не совпадает"
    );

    // Средние и СКО по обучающей выборке.
    let mut means = vec![0.0_f64; p];
    let mut sds = vec![1.0_f64; p];
    for j in 0..p {
        let col: Vec<f64> = xs.iter().map(|r| r[j]).collect();
        means[j] = stats::mean(&col);
        let s = stats::sd(&col);
        sds[j] = if s.is_finite() && s > 1e-12 { s } else { 1.0 };
    }

    // Стандартизованный дизайн-матричный контекст.
    let z: Vec<Vec<f64>> = xs
        .iter()
        .map(|r| (0..p).map(|j| (r[j] - means[j]) / sds[j]).collect())
        .collect();
    let t: Vec<f64> = ys.iter().map(|&y| if y { 1.0 } else { 0.0 }).collect();

    // Параметры: [интерцепт, w_1..w_p].
    let mut beta = vec![0.0_f64; p + 1];
    // Разумная стартовая точка: логит наблюдаемой распространённости.
    let prevalence = t.iter().sum::<f64>() / n as f64;
    beta[0] = detmath::logit(prevalence.clamp(1e-6, 1.0 - 1e-6));

    let mut iterations = 0;
    let mut converged = false;

    for _ in 0..max_iter {
        iterations += 1;

        // Градиент и гессиан логарифма правдоподобия.
        let mut grad = vec![0.0_f64; p + 1];
        let mut hess = vec![vec![0.0_f64; p + 1]; p + 1];

        for i in 0..n {
            let mut eta = beta[0];
            for j in 0..p {
                eta += beta[j + 1] * z[i][j];
            }
            let prob = detmath::sigmoid(eta);
            let w = (prob * (1.0 - prob)).clamp(1e-10, 0.25);
            let resid = t[i] - prob;

            grad[0] += resid;
            hess[0][0] += w;
            for j in 0..p {
                grad[j + 1] += resid * z[i][j];
                hess[0][j + 1] += w * z[i][j];
                hess[j + 1][0] += w * z[i][j];
                for k in 0..p {
                    hess[j + 1][k + 1] += w * z[i][j] * z[i][k];
                }
            }
        }

        // Гребневой штраф (без интерцепта) и регуляризация диагонали.
        let mut rhs = vec![0.0_f64; p + 1];
        for j in 0..=p {
            rhs[j] = grad[j];
        }
        for j in 1..=p {
            rhs[j] -= lambda * beta[j];
            hess[j][j] += lambda;
        }
        // Микро-демпфер для устойчивости обращения.
        for j in 0..=p {
            hess[j][j] += 1e-12;
        }

        let Some(step) = solve_linear(&mut hess, &mut rhs) else {
            // Гессиан вырожден — прекращаем, сохраняя текущие параметры.
            break;
        };

        let mut max_step = 0.0_f64;
        for j in 0..=p {
            beta[j] += step[j];
            max_step = max_step.max(step[j].abs());
        }
        if max_step < tol {
            converged = true;
            break;
        }
    }

    LogisticModel {
        feature_names: feature_names.to_vec(),
        weights: beta[1..].to_vec(),
        intercept: beta[0],
        means,
        sds,
        lambda,
        iterations,
        converged,
        n_train: n,
        n_events: ys.iter().filter(|&&y| y).count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(p: usize) -> Vec<String> {
        (0..p).map(|i| format!("x{i}")).collect()
    }

    /// Данные, идеально разделимые одним признаком: коэффициент должен быть
    /// большим и положительным, а предсказания — точными.
    #[test]
    fn recovers_clean_signal() {
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        for i in 0..400 {
            let x = (i as f64 - 200.0) / 100.0;
            xs.push(vec![x]);
            ys.push(x > 0.0);
        }
        let m = fit(&names(1), &xs, &ys, 1e-6, 100, 1e-10);
        assert!(m.converged, "не сошлось за {} шагов", m.iterations);
        assert!(m.weights[0] > 0.0, "вес = {}", m.weights[0]);

        let correct = xs
            .iter()
            .zip(&ys)
            .filter(|(x, y)| m.predict(x) == **y)
            .count();
        assert!(correct >= 396, "правильных {correct} из 400");
    }

    /// Известная истинная модель: проверяем, что оценки близки к правде
    /// на большой выборке (состоятельность).
    #[test]
    fn recovers_known_coefficients_on_large_sample() {
        use crate::rng::Rng;

        let mut rng = Rng::new(12345);
        let n = 40_000;
        let mut xs = Vec::with_capacity(n);
        let mut ys = Vec::with_capacity(n);

        // Истина: η = -1.2 + 0.9·x1 - 0.5·x2
        for _ in 0..n {
            let x1 = rng.normal(0.0, 1.0);
            let x2 = rng.normal(0.0, 1.0);
            let eta = -1.2 + 0.9 * x1 - 0.5 * x2;
            let p = detmath::sigmoid(eta);
            let y = rng.bernoulli(p);
            xs.push(vec![x1, x2]);
            ys.push(y);
        }

        let m = fit(&names(2), &xs, &ys, 1e-6, 100, 1e-12);
        assert!(m.converged);
        // Признаки генерировались стандартно нормальными, поэтому
        // стандартизация почти не меняет масштаб.
        assert!((m.weights[0] - 0.9).abs() < 0.06, "w1 = {}", m.weights[0]);
        assert!((m.weights[1] + 0.5).abs() < 0.06, "w2 = {}", m.weights[1]);
        assert!((m.intercept + 1.2).abs() < 0.05, "b = {}", m.intercept);
    }

    #[test]
    fn standardization_round_trip_and_odds_ratios() {
        let xs = vec![
            vec![10.0, 1.0],
            vec![20.0, 2.0],
            vec![30.0, 3.0],
            vec![40.0, 4.0],
        ];
        let ys = vec![false, false, true, true];
        let m = fit(&names(2), &xs, &ys, 1e-3, 50, 1e-10);

        // Среднее стандартизованных значений должно быть ~0.
        let zs: Vec<Vec<f64>> = xs.iter().map(|x| m.standardize(x)).collect();
        for j in 0..2 {
            let col: Vec<f64> = zs.iter().map(|z| z[j]).collect();
            assert!(stats::mean(&col).abs() < 1e-9, "среднее z[{j}] != 0");
        }

        // OR на единицу признака должно быть exp(w/sd).
        let ors = m.odds_ratios_per_unit();
        for (j, or) in ors.iter().enumerate() {
            let expected = detmath::exp(m.weights[j] / m.sds[j]);
            assert!((or - expected).abs() < 1e-12);
        }
    }

    #[test]
    fn eta_range_and_proba_are_consistent() {
        let xs = vec![vec![-2.0], vec![0.0], vec![2.0], vec![5.0]];
        let ys = vec![false, false, true, true];
        let m = fit(&names(1), &xs, &ys, 1e-6, 100, 1e-12);

        let (lo, hi) = m.eta_range(&xs);
        assert!(lo < hi);
        for x in &xs {
            let e = m.eta(x);
            assert!(e >= lo - 1e-9 && e <= hi + 1e-9);
            let p = m.predict_proba(x);
            assert!((p - detmath::sigmoid(e)).abs() < 1e-15);
            // Решение по 0.5 совпадает со знаком логита.
            assert_eq!(m.predict(x), e >= 0.0);
        }
    }

    #[test]
    fn constant_feature_does_not_break_fitting() {
        // Второй столбец постоянен: sd = 0 должен подменяться на 1.
        let xs = vec![
            vec![1.0, 7.0],
            vec![2.0, 7.0],
            vec![3.0, 7.0],
            vec![4.0, 7.0],
        ];
        let ys = vec![false, false, true, true];
        let m = fit(&names(2), &xs, &ys, 1e-3, 50, 1e-10);
        assert_eq!(m.sds[1], 1.0);
        assert!(m.weights.iter().all(|w| w.is_finite()));
    }

    #[test]
    fn linear_solver_handles_singular_matrix() {
        let mut a = vec![vec![1.0, 2.0], vec![2.0, 4.0]];
        let mut b = vec![3.0, 6.0];
        assert!(solve_linear(&mut a, &mut b).is_none());

        let mut a2 = vec![vec![2.0, 1.0], vec![1.0, 3.0]];
        let mut b2 = vec![5.0, 10.0];
        let x = solve_linear(&mut a2, &mut b2).unwrap();
        // Решение: x = (1, 3).
        assert!((x[0] - 1.0).abs() < 1e-12);
        assert!((x[1] - 3.0).abs() < 1e-12);
    }
}
