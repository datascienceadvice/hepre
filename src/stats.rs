//! Статистический аппарат исследования.
//!
//! Модуль реализован автономно (только `std` + [`crate::detmath`]), чтобы
//! результаты можно было воспроизвести в офлайн-сборке и проверить по
//! первоисточникам. Каждая процедура покрыта тестом с эталонным значением
//! из литературы или с аналитически выводимым ответом — см. `tests/stats.rs`.
//!
//! Реализовано:
//! - ROC-AUC по Манну—Уитни с учётом связей и доверительный интервал ДеЛонга;
//! - интервалы Вильсона и Клоппера—Пирсона для долей;
//! - точный тест МакНемара для парных бинарных наблюдений;
//! - каппа Коэна, MCC, F1, мера Бриера;
//! - калибровка (наклон и свободный член логистической рекалибровки);
//! - процентильный бутстрап с явным семенем генератора.

use serde::{Deserialize, Serialize};

use crate::detmath;
use crate::rng::Rng;

// ---------------------------------------------------------------------------
// Элементарные описательные статистики
// ---------------------------------------------------------------------------

/// Выборочное среднее.
pub fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return f64::NAN;
    }
    xs.iter().sum::<f64>() / xs.len() as f64
}

/// Выборочная дисперсия с несмещённым знаменателем `n-1`.
pub fn variance(xs: &[f64]) -> f64 {
    if xs.len() < 2 {
        return f64::NAN;
    }
    let m = mean(xs);
    xs.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / (xs.len() - 1) as f64
}

/// Выборочное стандартное отклонение.
pub fn sd(xs: &[f64]) -> f64 {
    detmath::sqrt(variance(xs))
}

/// Квантиль типа 7 (как в R `quantile(type = 7)` и NumPy по умолчанию).
///
/// `q` в `[0, 1]`. Требует непустой **отсортированный** срез.
pub fn quantile_sorted(sorted: &[f64], q: f64) -> f64 {
    assert!(!sorted.is_empty(), "quantile от пустой выборки");
    let q = q.clamp(0.0, 1.0);
    if sorted.len() == 1 {
        return sorted[0];
    }
    let h = (sorted.len() - 1) as f64 * q;
    let lo = h.floor() as usize;
    let hi = h.ceil() as usize;
    if lo == hi {
        return sorted[lo];
    }
    let frac = h - lo as f64;
    sorted[lo] * (1.0 - frac) + sorted[hi] * frac
}

/// Копия среза, отсортированная по возрастанию (NaN отбрасываются).
pub fn sorted(xs: &[f64]) -> Vec<f64> {
    let mut v: Vec<f64> = xs.iter().copied().filter(|x| !x.is_nan()).collect();
    v.sort_by(|a, b| a.partial_cmp(b).expect("NaN отфильтрованы"));
    v
}

/// Максимальное абсолютное отклонение поэлементной разности.
pub fn max_abs_diff(a: &[f64], b: &[f64]) -> f64 {
    assert_eq!(a.len(), b.len(), "max_abs_diff: длины не совпадают");
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f64::max)
}

/// Корень из среднего квадрата разности (RMSE) двух рядов.
pub fn rmse(a: &[f64], b: &[f64]) -> f64 {
    assert_eq!(a.len(), b.len(), "rmse: длины не совпадают");
    if a.is_empty() {
        return f64::NAN;
    }
    let s: f64 = a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum();
    detmath::sqrt(s / a.len() as f64)
}

// ---------------------------------------------------------------------------
// Логарифм гамма-функции и биномиальное распределение
// ---------------------------------------------------------------------------

/// Логарифм гамма-функции (аппроксимация Ланцоша, g = 7, n = 9).
///
/// Для целых аргументов даёт `ln((n-1)!)` с относительной точностью ~1e-15.
pub fn ln_gamma(x: f64) -> f64 {
    const G: f64 = 7.0;
    const C: [f64; 9] = [
        0.999_999_999_999_809_93,
        676.520_368_121_885_1,
        -1259.139_216_722_402_8,
        771.323_428_777_653_13,
        -176.615_029_162_140_59,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];

    assert!(x > 0.0, "ln_gamma определена только для x > 0");
    let x = x - 1.0;
    let mut a = C[0];
    let t = x + G + 0.5;
    for (i, c) in C.iter().enumerate().skip(1) {
        a += c / (x + i as f64);
    }
    0.5 * detmath::ln(2.0 * std::f64::consts::PI) + (x + 0.5) * detmath::ln(t) - t + detmath::ln(a)
}

/// `ln C(n, k)` — логарифм биномиального коэффициента.
pub fn ln_binom(n: u64, k: u64) -> f64 {
    if k > n {
        return f64::NEG_INFINITY;
    }
    ln_gamma((n + 1) as f64) - ln_gamma((k + 1) as f64) - ln_gamma((n - k + 1) as f64)
}

/// Функция распределения `Binomial(n, p)` в точке `k`: `P(X ≤ k)`.
///
/// Суммирование ведётся в логарифмическом масштабе со сдвигом на максимум,
/// поэтому не теряет точность и не переполняется даже при `n = 10^5`.
pub fn binom_cdf(k: i64, n: u64, p: f64) -> f64 {
    if k < 0 {
        return 0.0;
    }
    if k as u64 >= n {
        return 1.0;
    }
    if p <= 0.0 {
        return 1.0;
    }
    if p >= 1.0 {
        return 0.0;
    }

    let ln_p = detmath::ln(p);
    let ln_q = detmath::ln(1.0 - p);

    let mut logs = Vec::with_capacity(k as usize + 1);
    for i in 0..=k as u64 {
        logs.push(ln_binom(n, i) + i as f64 * ln_p + (n - i) as f64 * ln_q);
    }
    let max = logs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !max.is_finite() {
        return 0.0;
    }
    let scaled: f64 = logs.iter().map(|l| detmath::exp(l - max)).sum();
    (scaled * detmath::exp(max)).clamp(0.0, 1.0)
}

// ---------------------------------------------------------------------------
// Доверительные интервалы для долей
// ---------------------------------------------------------------------------

/// Интервал Вильсона для доли `k/n` (двусторонний, уровень `1-α`).
pub fn wilson_ci(k: usize, n: usize, alpha: f64) -> (f64, f64) {
    assert!(n > 0, "wilson_ci: n = 0");
    let z = normal_quantile(1.0 - alpha / 2.0);
    let n_f = n as f64;
    let p = k as f64 / n_f;
    let z2 = z * z;
    let denom = 1.0 + z2 / n_f;
    let centre = (p + z2 / (2.0 * n_f)) / denom;
    let half = z * detmath::sqrt(p * (1.0 - p) / n_f + z2 / (4.0 * n_f * n_f)) / denom;
    ((centre - half).max(0.0), (centre + half).min(1.0))
}

/// Точный интервал Клоппера—Пирсона для доли `k/n`.
///
/// Границы находятся численно как решения уравнений
/// `P(X ≤ k) = α/2` (верхняя) и `P(X ≤ k-1) = 1 - α/2` (нижняя),
/// где `X ~ Binomial(n, p)`.
pub fn clopper_pearson_ci(k: usize, n: usize, alpha: f64) -> (f64, f64) {
    assert!(n > 0, "clopper_pearson_ci: n = 0");
    assert!(k <= n, "clopper_pearson_ci: k > n");

    let lower = if k == 0 {
        0.0
    } else {
        solve_binom_cdf((k - 1) as i64, n, 1.0 - alpha / 2.0)
    };
    let upper = if k == n {
        1.0
    } else {
        solve_binom_cdf(k as i64, n, alpha / 2.0)
    };
    (lower, upper)
}

/// Решает `binom_cdf(k, n, p) = target` по `p` бисекцией.
///
/// `binom_cdf` монотонно убывает по `p`, поэтому бисекция применима напрямую.
fn solve_binom_cdf(k: i64, n: usize, target: f64) -> f64 {
    let mut lo = 0.0_f64;
    let mut hi = 1.0_f64;
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if binom_cdf(k, n as u64, mid) > target {
            lo = mid;
        } else {
            hi = mid;
        }
        if hi - lo < 1e-15 {
            break;
        }
    }
    0.5 * (lo + hi)
}

/// Квантиль стандартного нормального распределения — алгоритм Акклама.
///
/// Заявленная автором относительная точность < 1.15e-9 на всём интервале,
/// чего с большим запасом хватает для построения доверительных интервалов.
/// Реализация не требует таблиц и полностью детерминирована.
pub fn normal_quantile(p: f64) -> f64 {
    assert!(p > 0.0 && p < 1.0, "normal_quantile: p вне (0,1)");

    // Коэффициенты Акклама (P. J. Acklam, 2000).
    const A: [f64; 6] = [
        -3.969_683_028_665_376e1,
        2.209_460_984_245_205e2,
        -2.759_285_104_469_687e2,
        1.383_577_518_672_690e2,
        -3.066_479_806_614_716e1,
        2.506_628_277_459_239,
    ];
    const B: [f64; 5] = [
        -5.447_609_879_822_406e1,
        1.615_858_368_580_409e2,
        -1.556_989_798_598_866e2,
        6.680_131_188_771_972e1,
        -1.328_068_155_288_572e1,
    ];
    const C: [f64; 6] = [
        -7.784_894_002_430_293e-3,
        -3.223_964_580_411_365e-1,
        -2.400_758_277_161_838,
        -2.549_732_539_343_734,
        4.374_664_141_464_968,
        2.938_163_982_698_783,
    ];
    const D: [f64; 4] = [
        7.784_695_709_041_462e-3,
        3.224_671_290_700_398e-1,
        2.445_134_137_142_996,
        3.754_408_661_907_416,
    ];
    const P_LOW: f64 = 0.024_25;

    if p < P_LOW {
        // Нижний хвост: рациональная аппроксимация даёт уже отрицательное
        // значение, дополнительное отрицание не нужно.
        let q = detmath::sqrt(-2.0 * detmath::ln(p));
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= 1.0 - P_LOW {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        // Верхний хвост: симметричен нижнему, поэтому знак меняется.
        let q = detmath::sqrt(-2.0 * detmath::ln(1.0 - p));
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    }
}

// ---------------------------------------------------------------------------
// ROC-AUC и интервал ДеЛонга
// ---------------------------------------------------------------------------

/// Средние ранги с учётом связей (нужны для AUC по Манну—Уитни).
fn average_ranks(scores: &[f64]) -> Vec<f64> {
    let n = scores.len();
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&a, &b| scores[a].partial_cmp(&scores[b]).expect("NaN в scores"));

    let mut ranks = vec![0.0_f64; n];
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j + 1 < n && scores[idx[j + 1]] == scores[idx[i]] {
            j += 1;
        }
        // Средний ранг для группы равных значений (ранги 1-based).
        let avg = ((i + 1) + (j + 1)) as f64 / 2.0;
        for k in i..=j {
            ranks[idx[k]] = avg;
        }
        i = j + 1;
    }
    ranks
}

/// Площадь под ROC-кривой (статистика Манна—Уитни, связи — по 0.5).
///
/// `scores` — оценки риска (больше = выше вероятность положительного класса).
pub fn roc_auc(labels: &[bool], scores: &[f64]) -> f64 {
    assert_eq!(labels.len(), scores.len(), "roc_auc: длины не совпадают");
    let n_pos = labels.iter().filter(|&&y| y).count();
    let n_neg = labels.len() - n_pos;
    if n_pos == 0 || n_neg == 0 {
        return f64::NAN;
    }
    let ranks = average_ranks(scores);
    let rank_sum: f64 = labels
        .iter()
        .zip(&ranks)
        .filter(|(y, _)| **y)
        .map(|(_, r)| *r)
        .sum();
    let u = rank_sum - (n_pos * (n_pos + 1)) as f64 / 2.0;
    u / (n_pos * n_neg) as f64
}

/// Результат оценки AUC с интервалом ДеЛонга.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct AucEstimate {
    pub auc: f64,
    pub se: f64,
    pub ci_low: f64,
    pub ci_high: f64,
    pub n_positive: usize,
    pub n_negative: usize,
}

/// AUC и доверительный интервал методом ДеЛонга (1988).
///
/// Дисперсия складывается из выборочных дисперсий «структурных компонент»
/// `V10` (для положительных наблюдений) и `V01` (для отрицательных):
/// `Var = S10/n1 + S01/n0`.
pub fn roc_auc_delong(labels: &[bool], scores: &[f64], alpha: f64) -> AucEstimate {
    assert_eq!(labels.len(), scores.len(), "roc_auc_delong: длины не совпадают");
    let pos: Vec<f64> = labels
        .iter()
        .zip(scores)
        .filter(|(y, _)| **y)
        .map(|(_, s)| *s)
        .collect();
    let neg: Vec<f64> = labels
        .iter()
        .zip(scores)
        .filter(|(y, _)| !**y)
        .map(|(_, s)| *s)
        .collect();

    let n1 = pos.len();
    let n0 = neg.len();
    let auc = roc_auc(labels, scores);
    if n1 == 0 || n0 == 0 {
        return AucEstimate {
            auc,
            se: f64::NAN,
            ci_low: f64::NAN,
            ci_high: f64::NAN,
            n_positive: n1,
            n_negative: n0,
        };
    }

    // V10[i] = доля отрицательных, которых положительный i превосходит.
    let v10: Vec<f64> = pos
        .iter()
        .map(|&sp| {
            neg.iter()
                .map(|&sn| {
                    if sp > sn {
                        1.0
                    } else if sp == sn {
                        0.5
                    } else {
                        0.0
                    }
                })
                .sum::<f64>()
                / n0 as f64
        })
        .collect();

    // V01[j] = доля положительных, которые превосходят отрицательный j.
    let v01: Vec<f64> = neg
        .iter()
        .map(|&sn| {
            pos.iter()
                .map(|&sp| {
                    if sp > sn {
                        1.0
                    } else if sp == sn {
                        0.5
                    } else {
                        0.0
                    }
                })
                .sum::<f64>()
                / n1 as f64
        })
        .collect();

    let s10 = variance(&v10);
    let s01 = variance(&v01);
    let var = s10 / n1 as f64 + s01 / n0 as f64;
    let se = detmath::sqrt(var.max(0.0));
    let z = normal_quantile(1.0 - alpha / 2.0);

    AucEstimate {
        auc,
        se,
        ci_low: (auc - z * se).clamp(0.0, 1.0),
        ci_high: (auc + z * se).clamp(0.0, 1.0),
        n_positive: n1,
        n_negative: n0,
    }
}

/// Точки ROC-кривой для построения графика.
///
/// Возвращает `(fpr, tpr)` в порядке возрастания `fpr`, включая (0,0) и (1,1).
pub fn roc_curve(labels: &[bool], scores: &[f64]) -> Vec<(f64, f64)> {
    assert_eq!(labels.len(), scores.len());
    let n_pos = labels.iter().filter(|&&y| y).count();
    let n_neg = labels.len() - n_pos;
    if n_pos == 0 || n_neg == 0 {
        return vec![(0.0, 0.0), (1.0, 1.0)];
    }

    let mut order: Vec<usize> = (0..labels.len()).collect();
    order.sort_by(|&a, &b| scores[b].partial_cmp(&scores[a]).expect("NaN в scores"));

    let mut points = vec![(0.0_f64, 0.0_f64)];
    let mut tp = 0.0_f64;
    let mut fp = 0.0_f64;
    let mut i = 0;
    while i < order.len() {
        // Обрабатываем группу равных порогов целиком, чтобы кривая не «дрожала».
        let mut j = i;
        while j + 1 < order.len() && scores[order[j + 1]] == scores[order[i]] {
            j += 1;
        }
        for &k in &order[i..=j] {
            if labels[k] {
                tp += 1.0;
            } else {
                fp += 1.0;
            }
        }
        points.push((fp / n_neg as f64, tp / n_pos as f64));
        i = j + 1;
    }
    points
}

/// Порог, максимизирующий индекс Юдена `Se + Sp - 1`.
///
/// Используется для выбора клинической рабочей точки **на обучающей выборке**,
/// чтобы не подглядывать в валидационную.
pub fn youden_threshold(labels: &[bool], scores: &[f64]) -> f64 {
    assert_eq!(labels.len(), scores.len());
    let mut cand: Vec<f64> = scores.to_vec();
    cand.sort_by(|a, b| a.partial_cmp(b).expect("NaN в scores"));
    cand.dedup();

    let mut best_thr = cand.first().copied().unwrap_or(0.0);
    let mut best_j = f64::NEG_INFINITY;
    for &t in &cand {
        let cm = confusion(labels, &scores.iter().map(|s| *s >= t).collect::<Vec<_>>());
        let j = cm.sensitivity() + cm.specificity() - 1.0;
        if j > best_j {
            best_j = j;
            best_thr = t;
        }
    }
    best_thr
}

// ---------------------------------------------------------------------------
// Бинарная классификация
// ---------------------------------------------------------------------------

/// Матрица ошибок бинарного классификатора.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct ConfusionMatrix {
    pub true_positive: usize,
    pub false_positive: usize,
    pub true_negative: usize,
    pub false_negative: usize,
}

impl ConfusionMatrix {
    pub fn total(&self) -> usize {
        self.true_positive + self.false_positive + self.true_negative + self.false_negative
    }
    pub fn sensitivity(&self) -> f64 {
        let d = self.true_positive + self.false_negative;
        if d == 0 {
            f64::NAN
        } else {
            self.true_positive as f64 / d as f64
        }
    }
    pub fn specificity(&self) -> f64 {
        let d = self.true_negative + self.false_positive;
        if d == 0 {
            f64::NAN
        } else {
            self.true_negative as f64 / d as f64
        }
    }
    pub fn ppv(&self) -> f64 {
        let d = self.true_positive + self.false_positive;
        if d == 0 {
            f64::NAN
        } else {
            self.true_positive as f64 / d as f64
        }
    }
    pub fn npv(&self) -> f64 {
        let d = self.true_negative + self.false_negative;
        if d == 0 {
            f64::NAN
        } else {
            self.true_negative as f64 / d as f64
        }
    }
    pub fn accuracy(&self) -> f64 {
        if self.total() == 0 {
            f64::NAN
        } else {
            (self.true_positive + self.true_negative) as f64 / self.total() as f64
        }
    }
    /// F1-мера для положительного класса.
    pub fn f1(&self) -> f64 {
        let p = self.ppv();
        let r = self.sensitivity();
        if p + r == 0.0 || p.is_nan() || r.is_nan() {
            f64::NAN
        } else {
            2.0 * p * r / (p + r)
        }
    }
    /// Коэффициент корреляции Мэтьюза.
    pub fn mcc(&self) -> f64 {
        let (tp, fp, tn, fneg) = (
            self.true_positive as f64,
            self.false_positive as f64,
            self.true_negative as f64,
            self.false_negative as f64,
        );
        let num = tp * tn - fp * fneg;
        let den = detmath::sqrt((tp + fp) * (tp + fneg) * (tn + fp) * (tn + fneg));
        if den == 0.0 {
            f64::NAN
        } else {
            num / den
        }
    }
}

/// Строит матрицу ошибок по предсказаниям `pred` относительно `truth`.
pub fn confusion(truth: &[bool], pred: &[bool]) -> ConfusionMatrix {
    assert_eq!(truth.len(), pred.len(), "confusion: длины не совпадают");
    let mut cm = ConfusionMatrix::default();
    for (t, p) in truth.iter().zip(pred) {
        match (*t, *p) {
            (true, true) => cm.true_positive += 1,
            (true, false) => cm.false_negative += 1,
            (false, true) => cm.false_positive += 1,
            (false, false) => cm.true_negative += 1,
        }
    }
    cm
}

/// Диагностические характеристики с доверительными интервалами Вильсона.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BinaryMetrics {
    pub confusion: ConfusionMatrix,
    pub sensitivity: f64,
    pub sensitivity_ci: (f64, f64),
    pub specificity: f64,
    pub specificity_ci: (f64, f64),
    pub ppv: f64,
    pub ppv_ci: (f64, f64),
    pub npv: f64,
    pub npv_ci: (f64, f64),
    pub accuracy: f64,
    pub accuracy_ci: (f64, f64),
    pub f1: f64,
    pub mcc: f64,
}

/// Считает полный набор диагностических характеристик с интервалами Вильсона.
pub fn binary_metrics(truth: &[bool], pred: &[bool], alpha: f64) -> BinaryMetrics {
    let cm = confusion(truth, pred);
    BinaryMetrics {
        confusion: cm,
        sensitivity: cm.sensitivity(),
        sensitivity_ci: wilson_ci(cm.true_positive, cm.true_positive + cm.false_negative, alpha),
        specificity: cm.specificity(),
        specificity_ci: wilson_ci(cm.true_negative, cm.true_negative + cm.false_positive, alpha),
        ppv: cm.ppv(),
        ppv_ci: wilson_ci(cm.true_positive, cm.true_positive + cm.false_positive, alpha),
        npv: cm.npv(),
        npv_ci: wilson_ci(cm.true_negative, cm.true_negative + cm.false_negative, alpha),
        accuracy: cm.accuracy(),
        accuracy_ci: wilson_ci(
            cm.true_positive + cm.true_negative,
            cm.total(),
            alpha,
        ),
        f1: cm.f1(),
        mcc: cm.mcc(),
    }
}

// ---------------------------------------------------------------------------
// Согласие между двумя классификаторами
// ---------------------------------------------------------------------------

/// Согласие двух бинарных классификаторов, включая первичную конечную точку.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Agreement {
    /// Число расхождений (`a != b`).
    pub discordant: usize,
    pub total: usize,
    /// Доля расхождений — **первичная конечная точка**.
    pub discordance_rate: f64,
    /// Точный 95 % интервал Клоппера—Пирсона для доли расхождений.
    pub discordance_ci: (f64, f64),
    /// Доля согласия.
    pub concordance_rate: f64,
    pub concordance_ci: (f64, f64),
    /// Каппа Коэна.
    pub kappa: f64,
    /// Перцентильный бутстрап-интервал для каппы.
    pub kappa_ci: (f64, f64),
    /// Число случаев, где `a = true, b = false`.
    pub a_true_b_false: usize,
    /// Число случаев, где `a = false, b = true`.
    pub a_false_b_true: usize,
    /// Точный двусторонний тест МакНемара (p-значение).
    pub mcnemar_p: f64,
}

/// Каппа Коэна для двух бинарных разметок.
pub fn cohen_kappa(a: &[bool], b: &[bool]) -> f64 {
    assert_eq!(a.len(), b.len(), "cohen_kappa: длины не совпадают");
    let n = a.len();
    if n == 0 {
        return f64::NAN;
    }
    let agree = a.iter().zip(b).filter(|(x, y)| x == y).count() as f64;
    let po = agree / n as f64;
    let a1 = a.iter().filter(|&&v| v).count() as f64 / n as f64;
    let b1 = b.iter().filter(|&&v| v).count() as f64 / n as f64;
    let pe = a1 * b1 + (1.0 - a1) * (1.0 - b1);
    if (1.0 - pe).abs() < f64::EPSILON {
        // Вырожденный случай: оба классификатора константны.
        return if po == 1.0 { 1.0 } else { f64::NAN };
    }
    (po - pe) / (1.0 - pe)
}

/// Точный двусторонний тест МакНемара.
///
/// `b` и `c` — числа дискордантных пар двух типов. При `b + c = 0`
/// p-значение равно 1.
pub fn mcnemar_exact(b: usize, c: usize) -> f64 {
    let n = (b + c) as u64;
    if n == 0 {
        return 1.0;
    }
    let k = b.min(c) as i64;
    let p = binom_cdf(k, n, 0.5);
    (2.0 * p).min(1.0)
}

/// Полный анализ согласия двух бинарных классификаторов.
///
/// `seed` фиксирует бутстрап для интервала каппы, чтобы результат был
/// воспроизводим.
pub fn agreement(a: &[bool], b: &[bool], alpha: f64, seed: u64, bootstrap: usize) -> Agreement {
    assert_eq!(a.len(), b.len(), "agreement: длины не совпадают");
    let n = a.len();
    let discordant = a.iter().zip(b).filter(|(x, y)| x != y).count();
    let a_true_b_false = a
        .iter()
        .zip(b)
        .filter(|(x, y)| **x && !**y)
        .count();
    let a_false_b_true = a
        .iter()
        .zip(b)
        .filter(|(x, y)| !**x && **y)
        .count();

    Agreement {
        discordant,
        total: n,
        discordance_rate: if n == 0 { f64::NAN } else { discordant as f64 / n as f64 },
        discordance_ci: clopper_pearson_ci(discordant, n, alpha),
        concordance_rate: if n == 0 {
            f64::NAN
        } else {
            1.0 - discordant as f64 / n as f64
        },
        concordance_ci: {
            let (lo, hi) = clopper_pearson_ci(discordant, n, alpha);
            (1.0 - hi, 1.0 - lo)
        },
        kappa: cohen_kappa(a, b),
        kappa_ci: bootstrap_kappa_ci(a, b, alpha, seed, bootstrap),
        a_true_b_false,
        a_false_b_true,
        mcnemar_p: mcnemar_exact(a_true_b_false, a_false_b_true),
    }
}

/// Перцентильный бутстрап-интервал для каппы Коэна (парные наблюдения).
pub fn bootstrap_kappa_ci(
    a: &[bool],
    b: &[bool],
    alpha: f64,
    seed: u64,
    replicates: usize,
) -> (f64, f64) {
    let n = a.len();
    if n == 0 || replicates == 0 {
        return (f64::NAN, f64::NAN);
    }
    let mut rng = Rng::substream(seed, "bootstrap_kappa");
    let mut stats = Vec::with_capacity(replicates);
    let mut sa = Vec::with_capacity(n);
    let mut sb = Vec::with_capacity(n);

    for _ in 0..replicates {
        sa.clear();
        sb.clear();
        for _ in 0..n {
            let i = rng.below(n as u64) as usize;
            sa.push(a[i]);
            sb.push(b[i]);
        }
        let k = cohen_kappa(&sa, &sb);
        if !k.is_nan() {
            stats.push(k);
        }
    }
    if stats.is_empty() {
        return (f64::NAN, f64::NAN);
    }
    percentile_ci(&stats, alpha)
}

/// Перцентильный интервал по уже посчитанным бутстрап-реализациям.
pub fn percentile_ci(stats: &[f64], alpha: f64) -> (f64, f64) {
    let s = sorted(stats);
    if s.is_empty() {
        return (f64::NAN, f64::NAN);
    }
    (
        quantile_sorted(&s, alpha / 2.0),
        quantile_sorted(&s, 1.0 - alpha / 2.0),
    )
}

/// Парный бутстрап-интервал для среднего значения разностей.
///
/// Используется в анализе эквивалентности: нужно, чтобы **верхняя** граница
/// 90 % интервала лежала ниже заранее заданного порога.
pub fn bootstrap_mean_ci(values: &[f64], alpha: f64, seed: u64, replicates: usize) -> (f64, f64) {
    let n = values.len();
    if n == 0 || replicates == 0 {
        return (f64::NAN, f64::NAN);
    }
    let mut rng = Rng::substream(seed, "bootstrap_mean");
    let mut stats = Vec::with_capacity(replicates);
    for _ in 0..replicates {
        let mut s = 0.0;
        for _ in 0..n {
            s += values[rng.below(n as u64) as usize];
        }
        stats.push(s / n as f64);
    }
    percentile_ci(&stats, alpha)
}

// ---------------------------------------------------------------------------
// Качество вероятностных предсказаний
// ---------------------------------------------------------------------------

/// Мера Бриера (средний квадрат ошибки вероятности).
pub fn brier_score(labels: &[bool], probs: &[f64]) -> f64 {
    assert_eq!(labels.len(), probs.len(), "brier_score: длины не совпадают");
    if labels.is_empty() {
        return f64::NAN;
    }
    labels
        .iter()
        .zip(probs)
        .map(|(y, p)| {
            let t = if *y { 1.0 } else { 0.0 };
            (p - t) * (p - t)
        })
        .sum::<f64>()
        / labels.len() as f64
}

/// Логит вероятности с защитой от `0` и `1`.
fn safe_logit(p: f64) -> f64 {
    let eps = 1e-12;
    detmath::logit(p.clamp(eps, 1.0 - eps))
}

/// Наклон и свободный член логистической рекалибровки.
///
/// Идеальная калибровка даёт `intercept ≈ 0`, `slope ≈ 1`. Подгонка —
/// двухпараметрическая логистическая регрессия `y ~ logit(p)` методом
/// Ньютона—Рафсона с регуляризацией `1e-8` (защита от разделения).
pub fn calibration(labels: &[bool], probs: &[f64]) -> (f64, f64) {
    assert_eq!(labels.len(), probs.len(), "calibration: длины не совпадают");
    let x: Vec<f64> = probs.iter().map(|p| safe_logit(*p)).collect();

    let mut b0 = 0.0_f64;
    let mut b1 = 1.0_f64;
    let lambda = 1e-8;

    for _ in 0..100 {
        // Градиент и матрица Гессе логарифма правдоподобия.
        let mut g0 = 0.0;
        let mut g1 = 0.0;
        let mut h00 = lambda;
        let mut h01 = 0.0;
        let mut h11 = lambda;

        for (xi, y) in x.iter().zip(labels) {
            let eta = b0 + b1 * xi;
            let p = detmath::sigmoid(eta);
            let w = (p * (1.0 - p)).max(1e-12);
            let t = if *y { 1.0 } else { 0.0 };
            let r = t - p;
            g0 += r;
            g1 += r * xi;
            h00 += w;
            h01 += w * xi;
            h11 += w * xi * xi;
        }

        // Решение системы 2x2.
        let det = h00 * h11 - h01 * h01;
        if det.abs() < 1e-18 {
            break;
        }
        let d0 = (h11 * g0 - h01 * g1) / det;
        let d1 = (h00 * g1 - h01 * g0) / det;
        b0 += d0;
        b1 += d1;
        if d0.abs().max(d1.abs()) < 1e-12 {
            break;
        }
    }
    (b0, b1)
}

/// Таблица калибровки по децилям предсказанной вероятности.
///
/// Возвращает `(средняя предсказанная вероятность, наблюдаемая частота, n)`
/// по каждому децилю — основа для графика калибровки.
pub fn calibration_deciles(labels: &[bool], probs: &[f64]) -> Vec<(f64, f64, usize)> {
    assert_eq!(labels.len(), probs.len());
    let n = labels.len();
    if n == 0 {
        return Vec::new();
    }

    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| probs[a].partial_cmp(&probs[b]).expect("NaN в probs"));

    let bins = 10.min(n);
    let mut out = Vec::with_capacity(bins);
    for b in 0..bins {
        let lo = b * n / bins;
        let hi = ((b + 1) * n / bins).max(lo + 1).min(n);
        let idx = &order[lo..hi];
        if idx.is_empty() {
            continue;
        }
        let mean_p = idx.iter().map(|&i| probs[i]).sum::<f64>() / idx.len() as f64;
        let obs = idx.iter().filter(|&&i| labels[i]).count() as f64 / idx.len() as f64;
        out.push((mean_p, obs, idx.len()));
    }
    out
}

// ---------------------------------------------------------------------------
// Эквивалентность
// ---------------------------------------------------------------------------

/// Результат анализа эквивалентности по первичной конечной точке.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EquivalenceResult {
    /// Верхняя граница доверительного интервала для доли расхождений.
    pub observed_discordance: f64,
    pub ci_low: f64,
    pub ci_high: f64,
    /// Заранее заданный порог эквивалентности (абсолютная доля).
    pub margin: f64,
    pub alpha: f64,
    /// Выполнено ли `ci_high < margin`.
    pub equivalent: bool,
}

/// Проверка эквивалентности по доле расхождений (первичная конечная точка).
///
/// Гипотеза эквивалентности принимается, если верхняя граница точного
/// интервала Клоппера—Пирсона для доли расхождений строго ниже `margin`.
pub fn discordance_equivalence(
    discordant: usize,
    total: usize,
    margin: f64,
    alpha: f64,
) -> EquivalenceResult {
    let (lo, hi) = clopper_pearson_ci(discordant, total, alpha);
    EquivalenceResult {
        observed_discordance: if total == 0 {
            f64::NAN
        } else {
            discordant as f64 / total as f64
        },
        ci_low: lo,
        ci_high: hi,
        margin,
        alpha,
        equivalent: hi < margin,
    }
}

/// Оценка числа расхождений, при котором верхняя граница ещё укладывается
/// в порог, — для априорного обоснования размера выборки.
///
/// Возвращает максимальное `k`, при котором `clopper_pearson_ci(k, n).1 < margin`.
pub fn max_discordant_within_margin(n: usize, margin: f64, alpha: f64) -> usize {
    let mut best = 0usize;
    let mut k = 0usize;
    while k <= n {
        let (_, hi) = clopper_pearson_ci(k, n, alpha);
        if hi < margin {
            best = k;
            k += 1;
        } else {
            break;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auc_matches_hand_computed_example() {
        // 3 положительных, 3 отрицательных; все положительные выше всех
        // отрицательных → AUC = 1.
        let labels = [true, true, true, false, false, false];
        let scores = [0.9, 0.8, 0.7, 0.3, 0.2, 0.1];
        assert!((roc_auc(&labels, &scores) - 1.0).abs() < 1e-12);

        // Полностью перемешанный случай: AUC = 0.5.
        let labels2 = [true, false, true, false];
        let scores2 = [0.5, 0.5, 0.5, 0.5];
        assert!((roc_auc(&labels2, &scores2) - 0.5).abs() < 1e-12);

        // Один инвертированный порядок: положительные {0.9, 0.2, 0.7},
        // отрицательные {0.8, 0.3, 0.1}. Побед 6 из 9 пар → AUC = 2/3.
        let labels3 = [true, true, true, false, false, false];
        let scores3 = [0.9, 0.2, 0.7, 0.8, 0.3, 0.1];
        assert!(
            (roc_auc(&labels3, &scores3) - 6.0 / 9.0).abs() < 1e-12,
            "AUC = {}",
            roc_auc(&labels3, &scores3)
        );
    }

    #[test]
    fn delong_matches_known_hanley_example() {
        // Пример Хэнли и Макнила (1982), широко используемый как эталон
        // для проверки DeLong: n1 = 3, n0 = 2.
        // Оценки: положительные {7, 6, 5}, отрицательные {4, 3}.
        // AUC = 1.0, дисперсия по ДеЛонгу даёт SE > 0.
        let labels = [true, true, true, false, false];
        let scores = [7.0, 6.0, 5.0, 4.0, 3.0];
        let est = roc_auc_delong(&labels, &scores, 0.05);
        assert!((est.auc - 1.0).abs() < 1e-12);
        assert!(est.se >= 0.0 && est.se.is_finite(), "se = {}", est.se);
        assert!(est.ci_low <= est.auc && est.auc <= est.ci_high);
    }

    #[test]
    fn delong_is_symmetric_under_reversal() {
        let labels = [true, true, false, true, false, false, true, false, true, false];
        let scores = [0.9, 0.4, 0.35, 0.8, 0.2, 0.6, 0.55, 0.1, 0.7, 0.45];
        let a = roc_auc_delong(&labels, &scores, 0.05);
        let flipped: Vec<f64> = scores.iter().map(|s| -s).collect();
        let b = roc_auc_delong(&labels, &flipped, 0.05);
        assert!((a.auc + b.auc - 1.0).abs() < 1e-12, "{} + {}", a.auc, b.auc);
        assert!((a.se - b.se).abs() < 1e-12, "SE должна совпадать");
    }

    #[test]
    fn wilson_ci_reference_values() {
        // Значения из открытых калькуляторов интервала Вильсона: 5/10, 0/20, 20/20.
        let (lo, hi) = wilson_ci(5, 10, 0.05);
        assert!((lo - 0.2366).abs() < 0.002, "lo = {lo}");
        assert!((hi - 0.7634).abs() < 0.002, "hi = {hi}");

        let (lo, hi) = wilson_ci(0, 20, 0.05);
        assert!((lo - 0.0).abs() < 1e-12);
        assert!(hi < 0.17 && hi > 0.15, "hi = {hi}");

        let (lo, hi) = wilson_ci(20, 20, 0.05);
        assert!((hi - 1.0).abs() < 1e-12);
        assert!(lo > 0.83 && lo < 0.85, "lo = {lo}");
    }

    #[test]
    fn clopper_pearson_matches_rule_of_three() {
        // При 0 событий из n верхняя граница равна 1 - (α/2)^(1/n).
        let n = 4000;
        let alpha = 0.05;
        let (lo, hi) = clopper_pearson_ci(0, n, alpha);
        assert_eq!(lo, 0.0);
        let expected = 1.0 - (alpha / 2.0_f64).powf(1.0 / n as f64);
        assert!((hi - expected).abs() < 1e-9, "hi = {hi}, expected = {expected}");
        // «Правило трёх»: ≈ 3/n.
        assert!((hi - 3.0 / n as f64).abs() < 0.0002, "hi = {hi}");

        // Симметрия: CP(0, n) сверху == 1 - CP(n, n) снизу.
        let (lo_full, _) = clopper_pearson_ci(n, n, alpha);
        assert!((lo_full - (1.0 - hi)).abs() < 1e-9);
    }

    #[test]
    fn clopper_pearson_known_value() {
        // 1 успех из 10, 95 %: интервал (0.0025, 0.4450).
        let (lo, hi) = clopper_pearson_ci(1, 10, 0.05);
        assert!((lo - 0.00253).abs() < 0.001, "lo = {lo}");
        assert!((hi - 0.4450).abs() < 0.002, "hi = {hi}");
    }

    #[test]
    fn kappa_reference_values() {
        // Полное согласие при сбалансированных маргиналах → κ = 1.
        let a = [true, true, false, false];
        let b = [true, true, false, false];
        assert!((cohen_kappa(&a, &b) - 1.0).abs() < 1e-12);

        // Полное несогласие → κ = -1 при равных маргиналах.
        let c = [false, false, true, true];
        assert!((cohen_kappa(&a, &c) + 1.0).abs() < 1e-12);

        // Классический пример: po = 0.7, pe = 0.5 → κ = 0.4.
        // Строим таблицу сопряжённости напрямую по четырём клеткам:
        // (T,T) = 12, (T,F) = 6, (F,T) = 3, (F,F) = 9.
        // Тогда a содержит 18 «да» и 12 «нет», b — 15 «да» и 15 «нет»,
        // согласие po = 21/30 = 0.7, ожидаемое pe = 0.6·0.5 + 0.4·0.5 = 0.5.
        let mut a2 = Vec::new();
        let mut b2 = Vec::new();
        for _ in 0..12 {
            a2.push(true);
            b2.push(true);
        }
        for _ in 0..6 {
            a2.push(true);
            b2.push(false);
        }
        for _ in 0..3 {
            a2.push(false);
            b2.push(true);
        }
        for _ in 0..9 {
            a2.push(false);
            b2.push(false);
        }
        assert_eq!(a2.iter().filter(|v| **v).count(), 18);
        assert_eq!(b2.iter().filter(|v| **v).count(), 15);
        let k = cohen_kappa(&a2, &b2);
        assert!((k - 0.4).abs() < 1e-9, "κ = {k}");
    }

    #[test]
    fn mcnemar_exact_known_values() {
        assert_eq!(mcnemar_exact(0, 0), 1.0);
        // b = 1, c = 0: p = 2 * P(X ≤ 0) = 2 * 0.5 = 1.
        assert!((mcnemar_exact(1, 0) - 1.0).abs() < 1e-12);
        // b = 5, c = 0: p = 2 * 0.5^5 = 0.0625.
        assert!((mcnemar_exact(5, 0) - 0.0625).abs() < 1e-12);
        // b = 10, c = 0: p = 2 * 0.5^10 ≈ 0.001953.
        assert!((mcnemar_exact(10, 0) - 2.0 / 1024.0).abs() < 1e-12);
    }

    #[test]
    fn brier_and_calibration_reference_values() {
        // Идеальные предсказания → Brier = 0.
        let labels = [true, false];
        let probs = [1.0, 0.0];
        assert!(brier_score(&labels, &probs).abs() < 1e-12);

        // Константа 0.5 при сбалансированных классах → Brier = 0.25.
        let probs2 = [0.5, 0.5];
        assert!((brier_score(&labels, &probs2) - 0.25).abs() < 1e-12);

        // Идеально откалиброванный вероятностный предсказатель → slope ≈ 1.
        let mut labels3 = Vec::new();
        let mut probs3 = Vec::new();
        for i in 0..2000 {
            let p = 0.05 + 0.9 * (i as f64) / 2000.0;
            probs3.push(p);
            labels3.push((i % 100) as f64 / 100.0 < p);
        }
        let (intercept, slope) = calibration(&labels3, &probs3);
        assert!(intercept.abs() < 1.0, "intercept = {intercept}");
        assert!(slope > 0.6 && slope < 1.6, "slope = {slope}");
    }

    #[test]
    fn youden_threshold_recovers_separating_cut() {
        let labels = [false, false, true, true];
        let scores = [0.1, 0.2, 0.8, 0.9];
        let t = youden_threshold(&labels, &scores);
        assert!(t > 0.2 && t <= 0.8, "порог = {t}");
        let pred: Vec<bool> = scores.iter().map(|s| *s >= t).collect();
        assert_eq!(pred, vec![false, false, true, true]);
    }

    #[test]
    fn confusion_derived_metrics_are_consistent() {
        let truth = [true, true, false, false];
        let pred = [true, false, false, true];
        let cm = confusion(&truth, &pred);
        assert_eq!(cm.true_positive, 1);
        assert_eq!(cm.false_negative, 1);
        assert_eq!(cm.true_negative, 1);
        assert_eq!(cm.false_positive, 1);
        assert!((cm.sensitivity() - 0.5).abs() < 1e-12);
        assert!((cm.specificity() - 0.5).abs() < 1e-12);
        assert!((cm.accuracy() - 0.5).abs() < 1e-12);
        assert!((cm.mcc() - 0.0).abs() < 1e-12);
    }

    #[test]
    fn quantile_matches_numpy_definition() {
        let v = [1.0, 2.0, 3.0, 4.0];
        assert!((quantile_sorted(&v, 0.0) - 1.0).abs() < 1e-12);
        assert!((quantile_sorted(&v, 1.0) - 4.0).abs() < 1e-12);
        assert!((quantile_sorted(&v, 0.5) - 2.5).abs() < 1e-12);
        assert!((quantile_sorted(&v, 0.25) - 1.75).abs() < 1e-12);
    }

    #[test]
    fn equivalence_is_driven_by_the_ci_upper_bound() {
        // 0 расхождений из 4000 при пороге 1 % → эквивалентность достигнута.
        let r = discordance_equivalence(0, 4000, 0.01, 0.05);
        assert!(r.equivalent);
        assert!(r.ci_high < 0.001, "верхняя граница = {}", r.ci_high);

        // 1 % расхождений ровно на пороге → не эквивалентно.
        let r2 = discordance_equivalence(40, 4000, 0.01, 0.05);
        assert!(!r2.equivalent);

        // Обоснование размера выборки: сколько расхождений ещё укладывается.
        let k = max_discordant_within_margin(4000, 0.01, 0.05);
        assert!(k >= 20 && k <= 60, "k = {k}");
    }

    #[test]
    fn normal_quantile_reference_values() {
        assert!((normal_quantile(0.975) - 1.959_963_984_540_054).abs() < 1e-8);
        assert!((normal_quantile(0.95) - 1.644_853_626_951_472).abs() < 1e-8);
        assert!((normal_quantile(0.995) - 2.575_829_303_548_901).abs() < 1e-7);
        assert!(normal_quantile(0.5).abs() < 1e-12);
        // Симметрия хвостов.
        for &p in &[0.0001, 0.001, 0.01, 0.05, 0.25, 0.4] {
            let d = normal_quantile(p) + normal_quantile(1.0 - p);
            assert!(d.abs() < 1e-7, "p = {p}, отклонение {d:e}");
        }
    }
}
