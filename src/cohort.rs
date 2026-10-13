//! Синтетическая когорта с **документированным** процессом порождения данных.
//!
//! Исследование методологическое, поэтому эталон («ground truth») задаётся
//! явно, а не оценивается: признаки порождаются из заявленных распределений,
//! затем вычисляется истинный логит
//!
//! ```text
//! η_i = β₀ + Σ_j β_j · x_ij
//! ```
//!
//! и исход `y_i ~ Bernoulli(σ(η_i))`. Столбец `eta_true` и вероятность
//! `p_true` сохраняются вместе с данными, поэтому любая модель может быть
//! сопоставлена не только с разметкой, но и с истинной вероятностью.
//!
//! Интерцепт `β₀` не задаётся вручную: он **калибруется** бисекцией так,
//! чтобы ожидаемая распространённость совпала с заданной в конфигурации.
//! Это устраняет подгонку «на глаз» и делает распространённость
//! воспроизводимым параметром дизайна.
//!
//! Все источники случайности берутся из именованных подпотоков
//! ([`Rng::substream`]) одного семени, поэтому калибровка, генерация
//! ковариат, генерация исходов и разбиение на выборки независимы друг от
//! друга и воспроизводимы при любом порядке вызовов.
//!
//! **Данные полностью синтетические.** Реальные пациенты не участвуют,
//! персональные данные не используются.

use serde::{Deserialize, Serialize};

use crate::detmath;
use crate::rng::Rng;
use crate::stats;

/// Распределение признака в порождающей модели.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Distribution {
    /// Усечённое нормальное распределение.
    Normal { mean: f64, sd: f64 },
    /// Бернуллиевская величина (0/1).
    Bernoulli { p: f64 },
}

/// Описание одного предиктора порождающей модели.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PredictorSpec {
    /// Машинное имя (используется как заголовок столбца в CSV).
    pub name: String,
    /// Единицы измерения.
    pub units: String,
    /// Человекочитаемое описание.
    pub description: String,
    /// Распределение ковариаты.
    pub distribution: Distribution,
    /// Нижняя граница усечения (для `Bernoulli` игнорируется).
    #[serde(default)]
    pub min: f64,
    /// Верхняя граница усечения (для `Bernoulli` игнорируется).
    #[serde(default)]
    pub max: f64,
    /// Истинный коэффициент логистической модели (лог-отношение шансов).
    pub beta_true: f64,
}

/// Конфигурация исследования когорты.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CohortConfig {
    /// Семя генератора — единственный источник воспроизводимости.
    pub seed: u64,
    /// Размер обучающей (derivation) выборки.
    pub n_derivation: usize,
    /// Размер валидационной выборки.
    pub n_validation: usize,
    /// Целевая распространённость исходов, по которой калибруется `β₀`.
    pub target_prevalence: f64,
    /// Размер вспомогательной выборки для калибровки интерцепта.
    pub prevalence_calibration_n: usize,
    /// Спецификации предикторов.
    pub predictors: Vec<PredictorSpec>,
}

impl CohortConfig {
    /// Число предикторов.
    pub fn n_features(&self) -> usize {
        self.predictors.len()
    }

    /// Общий размер когорты.
    pub fn n_total(&self) -> usize {
        self.n_derivation + self.n_validation
    }

    /// Имена признаков.
    pub fn feature_names(&self) -> Vec<String> {
        self.predictors.iter().map(|p| p.name.clone()).collect()
    }
}

/// Один пациент синтетической когорты.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Patient {
    /// Идентификатор вида `S000123` — индекс в общем пуле.
    pub id: String,
    /// Значения признаков в порядке [`CohortConfig::predictors`].
    pub features: Vec<f64>,
    /// Истинный логит порождающей модели.
    pub eta_true: f64,
    /// Истинная вероятность положительного исхода `σ(η)`.
    pub p_true: f64,
    /// Наблюдаемый исход (эталонная разметка).
    pub outcome: bool,
}

/// Описательные характеристики сгенерированной когорты.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CohortDiagnostics {
    pub intercept_true: f64,
    pub target_prevalence: f64,
    pub calibration_iterations: usize,
    pub realized_prevalence_derivation: f64,
    pub realized_prevalence_validation: f64,
    pub realized_prevalence_all: f64,
    pub eta_mean_all: f64,
    pub eta_sd_all: f64,
    pub eta_min_all: f64,
    pub eta_max_all: f64,
    pub max_abs_eta_all: f64,
    pub feature_summary: Vec<FeatureSummary>,
}

/// Описательная статистика одного признака.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeatureSummary {
    pub name: String,
    pub units: String,
    pub mean: f64,
    pub sd: f64,
    pub min: f64,
    pub max: f64,
    /// Доля пропусков — в синтетической когорте всегда 0, но поле важно
    /// для отчётности: пропуски отсутствуют **по построению**.
    pub missing_fraction: f64,
}

/// Сгенерированная когорта с разбиением на обучающую и валидационную части.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cohort {
    pub config: CohortConfig,
    pub derivation: Vec<Patient>,
    pub validation: Vec<Patient>,
    pub diagnostics: CohortDiagnostics,
}

impl Cohort {
    /// Все пациенты подряд (обучающие, затем валидационные).
    pub fn all(&self) -> Vec<Patient> {
        let mut v = Vec::with_capacity(self.derivation.len() + self.validation.len());
        v.extend(self.derivation.iter().cloned());
        v.extend(self.validation.iter().cloned());
        v
    }

    /// Матрица признаков обучающей выборки.
    pub fn derivation_features(&self) -> Vec<Vec<f64>> {
        self.derivation.iter().map(|p| p.features.clone()).collect()
    }

    /// Матрица признаков валидационной выборки.
    pub fn validation_features(&self) -> Vec<Vec<f64>> {
        self.validation.iter().map(|p| p.features.clone()).collect()
    }

    /// Исходы обучающей выборки.
    pub fn derivation_outcomes(&self) -> Vec<bool> {
        self.derivation.iter().map(|p| p.outcome).collect()
    }

    /// Исходы валидационной выборки.
    pub fn validation_outcomes(&self) -> Vec<bool> {
        self.validation.iter().map(|p| p.outcome).collect()
    }

    /// Истинные вероятности валидационной выборки.
    pub fn validation_true_probabilities(&self) -> Vec<f64> {
        self.validation.iter().map(|p| p.p_true).collect()
    }

    /// CSV-представление когорты — замороженный артефакт исследования.
    ///
    /// Файл хэшируется и попадает в манифест, поэтому даже при смене
    /// версии генератора исходный набор данных остаётся проверяемым.
    pub fn to_csv(&self) -> String {
        let names = self.config.feature_names();
        let mut out = String::with_capacity(128 * self.config.n_total());

        out.push_str("id,split");
        for n in &names {
            out.push(',');
            out.push_str(n);
        }
        out.push_str(",eta_true,p_true,outcome\n");

        let write_row = |p: &Patient, split: &str, out: &mut String| {
            out.push_str(&p.id);
            out.push(',');
            out.push_str(split);
            for v in &p.features {
                out.push(',');
                out.push_str(&format!("{v:.10}"));
            }
            out.push(',');
            out.push_str(&format!("{:.10}", p.eta_true));
            out.push(',');
            out.push_str(&format!("{:.10}", p.p_true));
            out.push(',');
            out.push_str(if p.outcome { "1" } else { "0" });
            out.push('\n');
        };

        for p in &self.derivation {
            write_row(p, "derivation", &mut out);
        }
        for p in &self.validation {
            write_row(p, "validation", &mut out);
        }
        out
    }

    /// Оценки истинной модели на валидационной выборке — верхняя граница
    /// качества, достижимая при известных истинных коэффициентах.
    pub fn true_model_probabilities(&self) -> Vec<f64> {
        self.validation.iter().map(|p| p.p_true).collect()
    }
}

/// Генерирует одну ковариату по её спецификации.
///
/// Для усечённого нормального применяется отбраковка с ограничением числа
/// попыток: если значение так и не попало в интервал, оно зажимается в
/// границы. Ограничение делает процедуру детерминированной и гарантирует
/// завершение даже при неудачных параметрах распределения.
fn draw_covariate(rng: &mut Rng, spec: &PredictorSpec) -> f64 {
    match spec.distribution {
        Distribution::Normal { mean, sd } => {
            let (lo, hi) = if spec.min < spec.max {
                (spec.min, spec.max)
            } else {
                (f64::NEG_INFINITY, f64::INFINITY)
            };
            for _ in 0..1000 {
                let v = rng.normal(mean, sd);
                if v >= lo && v <= hi {
                    return v;
                }
            }
            rng.normal(mean, sd).clamp(lo, hi)
        }
        Distribution::Bernoulli { p } => {
            if rng.bernoulli(p) {
                1.0
            } else {
                0.0
            }
        }
    }
}

/// Генерирует матрицу ковариат `n x p`.
fn draw_covariates(rng: &mut Rng, specs: &[PredictorSpec], n: usize) -> Vec<Vec<f64>> {
    (0..n)
        .map(|_| specs.iter().map(|s| draw_covariate(rng, s)).collect())
        .collect()
}

/// Калибрует интерцепт `β₀` так, чтобы `E[σ(β₀ + xβ)]` совпало с целью.
///
/// Функция `β₀ ↦ E[σ(β₀ + xβ)]` строго возрастает, поэтому применима
/// бисекция. Ковариаты для калибровки берутся из отдельного подпотока и не
/// пересекаются с самой когортой.
fn calibrate_intercept(
    seed: u64,
    specs: &[PredictorSpec],
    target_prevalence: f64,
    calibration_n: usize,
) -> (f64, usize) {
    let mut rng = Rng::substream(seed, "prevalence_calibration");
    let covariates = draw_covariates(&mut rng, specs, calibration_n);
    let betas: Vec<f64> = specs.iter().map(|s| s.beta_true).collect();

    let mean_risk = |b0: f64| -> f64 {
        let sum: f64 = covariates
            .iter()
            .map(|x| {
                let eta = b0 + x.iter().zip(&betas).map(|(xi, b)| b * xi).sum::<f64>();
                detmath::sigmoid(eta)
            })
            .sum();
        sum / calibration_n as f64
    };

    let mut lo = -60.0_f64;
    let mut hi = 60.0_f64;
    let mut iterations = 0;
    for i in 0..200 {
        iterations = i + 1;
        let mid = 0.5 * (lo + hi);
        if mean_risk(mid) < target_prevalence {
            lo = mid;
        } else {
            hi = mid;
        }
        if hi - lo < 1e-12 {
            break;
        }
    }
    (0.5 * (lo + hi), iterations)
}

impl Cohort {
    /// Генерирует когорту по конфигурации.
    pub fn generate(config: &CohortConfig) -> Cohort {
        assert!(config.n_derivation > 0, "нужна непустая обучающая выборка");
        assert!(
            config.n_validation > 0,
            "нужна непустая валидационная выборка"
        );
        assert!(
            !config.predictors.is_empty(),
            "нужен хотя бы один предиктор"
        );

        let (intercept_true, calibration_iterations) = calibrate_intercept(
            config.seed,
            &config.predictors,
            config.target_prevalence,
            config.prevalence_calibration_n,
        );

        let n_total = config.n_total();
        let betas: Vec<f64> = config.predictors.iter().map(|s| s.beta_true).collect();

        // Ковариаты — отдельный подпоток.
        let mut cov_rng = Rng::substream(config.seed, "covariates");
        let covariates = draw_covariates(&mut cov_rng, &config.predictors, n_total);

        // Истинные логиты и вероятности.
        let etas: Vec<f64> = covariates
            .iter()
            .map(|x| intercept_true + x.iter().zip(&betas).map(|(xi, b)| b * xi).sum::<f64>())
            .collect();
        let probs: Vec<f64> = etas.iter().map(|e| detmath::sigmoid(*e)).collect();

        // Исходы — отдельный подпоток.
        let mut out_rng = Rng::substream(config.seed, "outcomes");
        let outcomes: Vec<bool> = probs.iter().map(|p| out_rng.bernoulli(*p)).collect();

        // Разбиение — отдельный подпоток: перемешиваем, затем режем.
        // Перемешивание снимает риск систематического дрейфа между частями,
        // который мог бы возникнуть при простом «первый кусок / второй кусок».
        let mut split_rng = Rng::substream(config.seed, "split");
        let mut order: Vec<usize> = (0..n_total).collect();
        split_rng.shuffle(&mut order);

        let mut patients: Vec<Patient> = (0..n_total)
            .map(|i| Patient {
                id: format!("S{i:06}"),
                features: covariates[i].clone(),
                eta_true: etas[i],
                p_true: probs[i],
                outcome: outcomes[i],
            })
            .collect();

        let mut derivation = Vec::with_capacity(config.n_derivation);
        let mut validation = Vec::with_capacity(config.n_validation);
        for (rank, &idx) in order.iter().enumerate() {
            if rank < config.n_derivation {
                derivation.push(patients[idx].clone());
            } else {
                validation.push(patients[idx].clone());
            }
        }
        // Освобождаем исходный пул явно, чтобы не держать дубликаты.
        patients.clear();

        let diagnostics = summarize(
            config,
            &derivation,
            &validation,
            intercept_true,
            calibration_iterations,
        );

        Cohort {
            config: config.clone(),
            derivation,
            validation,
            diagnostics,
        }
    }
}

/// Считает описательные характеристики когорты.
fn summarize(
    config: &CohortConfig,
    derivation: &[Patient],
    validation: &[Patient],
    intercept_true: f64,
    calibration_iterations: usize,
) -> CohortDiagnostics {
    let prev = |ps: &[Patient]| -> f64 {
        if ps.is_empty() {
            0.0
        } else {
            ps.iter().filter(|p| p.outcome).count() as f64 / ps.len() as f64
        }
    };

    let all_etas: Vec<f64> = derivation
        .iter()
        .chain(validation)
        .map(|p| p.eta_true)
        .collect();

    let feature_summary = config
        .predictors
        .iter()
        .enumerate()
        .map(|(j, spec)| {
            let col: Vec<f64> = derivation
                .iter()
                .chain(validation)
                .map(|p| p.features[j])
                .collect();
            let mut s = stats::sorted(&col);
            let min = s.first().copied().unwrap_or(f64::NAN);
            let max = s.pop().unwrap_or(f64::NAN);
            FeatureSummary {
                name: spec.name.clone(),
                units: spec.units.clone(),
                mean: stats::mean(&col),
                sd: stats::sd(&col),
                min,
                max,
                missing_fraction: 0.0,
            }
        })
        .collect();

    CohortDiagnostics {
        intercept_true,
        target_prevalence: config.target_prevalence,
        calibration_iterations,
        realized_prevalence_derivation: prev(derivation),
        realized_prevalence_validation: prev(validation),
        realized_prevalence_all: prev(
            &derivation
                .iter()
                .chain(validation)
                .cloned()
                .collect::<Vec<Patient>>(),
        ),
        eta_mean_all: stats::mean(&all_etas),
        eta_sd_all: stats::sd(&all_etas),
        eta_min_all: all_etas.iter().copied().fold(f64::INFINITY, f64::min),
        eta_max_all: all_etas.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        max_abs_eta_all: all_etas.iter().map(|e| e.abs()).fold(0.0, f64::max),
        feature_summary,
    }
}

/// Конфигурация по умолчанию: рутинные лабораторные и антропометрические
/// предикторы сахарного диабета 2-го типа.
///
/// Распределения выбраны клинически правдоподобными для взрослой популяции
/// с факторами риска; они **не** воспроизводят какое-либо конкретное
/// исследование или популяцию. Истинные коэффициенты заданы как
/// лог-отношения шансов на нативную единицу признака.
pub fn default_t2dm_config() -> CohortConfig {
    let p = |name: &str,
             units: &str,
             description: &str,
             distribution: Distribution,
             min: f64,
             max: f64,
             beta_true: f64| PredictorSpec {
        name: name.to_string(),
        units: units.to_string(),
        description: description.to_string(),
        distribution,
        min,
        max,
        beta_true,
    };

    CohortConfig {
        seed: 20_260_907,
        n_derivation: 8_000,
        n_validation: 4_000,
        target_prevalence: 0.12,
        prevalence_calibration_n: 200_000,
        predictors: vec![
            p(
                "hba1c_pct",
                "%",
                "Гликированный гемоглобин HbA1c",
                Distribution::Normal {
                    mean: 5.6,
                    sd: 0.85,
                },
                4.0,
                14.0,
                1.10,
            ),
            p(
                "fasting_glucose_mmol",
                "ммоль/л",
                "Глюкоза плазмы натощак",
                Distribution::Normal {
                    mean: 5.4,
                    sd: 1.10,
                },
                3.0,
                20.0,
                0.45,
            ),
            p(
                "triglycerides_mmol",
                "ммоль/л",
                "Триглицериды",
                Distribution::Normal {
                    mean: 1.7,
                    sd: 0.85,
                },
                0.3,
                12.0,
                0.30,
            ),
            p(
                "hdl_mmol",
                "ммоль/л",
                "Холестерин липопротеинов высокой плотности",
                Distribution::Normal {
                    mean: 1.40,
                    sd: 0.35,
                },
                0.4,
                3.2,
                -0.60,
            ),
            p(
                "bmi",
                "кг/м²",
                "Индекс массы тела",
                Distribution::Normal {
                    mean: 27.5,
                    sd: 5.0,
                },
                15.0,
                55.0,
                0.07,
            ),
            p(
                "age_years",
                "годы",
                "Возраст",
                Distribution::Normal {
                    mean: 52.0,
                    sd: 13.0,
                },
                18.0,
                90.0,
                0.035,
            ),
            p(
                "family_history",
                "0/1",
                "Семейный анамнез диабета",
                Distribution::Bernoulli { p: 0.30 },
                0.0,
                1.0,
                0.75,
            ),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small_config(seed: u64) -> CohortConfig {
        let mut c = default_t2dm_config();
        c.seed = seed;
        c.n_derivation = 500;
        c.n_validation = 250;
        c.prevalence_calibration_n = 20_000;
        c
    }

    #[test]
    fn generation_is_deterministic() {
        let c = small_config(777);
        let a = Cohort::generate(&c);
        let b = Cohort::generate(&c);
        assert_eq!(a.derivation.len(), b.derivation.len());
        for (x, y) in a.derivation.iter().zip(&b.derivation) {
            assert_eq!(x.id, y.id);
            assert_eq!(x.features, y.features);
            assert_eq!(x.outcome, y.outcome);
            assert_eq!(x.eta_true.to_bits(), y.eta_true.to_bits());
        }
        assert_eq!(a.to_csv(), b.to_csv());
    }

    #[test]
    fn different_seed_changes_data() {
        let a = Cohort::generate(&small_config(1));
        let b = Cohort::generate(&small_config(2));
        assert_ne!(a.to_csv(), b.to_csv());
    }

    #[test]
    fn prevalence_is_close_to_target() {
        let c = small_config(4242);
        let cohort = Cohort::generate(&c);
        // Калибровка идёт по большой выборке, поэтому систематического
        // смещения быть не должно; допускаем ошибку выборочной доли.
        let all = cohort.diagnostics.realized_prevalence_all;
        assert!(
            (all - c.target_prevalence).abs() < 0.03,
            "распространённость {all} против цели {}",
            c.target_prevalence
        );
    }

    #[test]
    fn split_partitions_the_pool_without_overlap() {
        let c = small_config(99);
        let cohort = Cohort::generate(&c);
        assert_eq!(cohort.derivation.len(), c.n_derivation);
        assert_eq!(cohort.validation.len(), c.n_validation);
        assert_eq!(
            cohort.derivation.len() + cohort.validation.len(),
            c.n_total()
        );

        let mut ids: Vec<String> = cohort
            .derivation
            .iter()
            .chain(&cohort.validation)
            .map(|p| p.id.clone())
            .collect();
        let before = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), before, "идентификаторы должны быть уникальны");
    }

    #[test]
    fn features_respect_declared_bounds() {
        let c = small_config(31337);
        let cohort = Cohort::generate(&c);
        for p in cohort.derivation.iter().chain(&cohort.validation) {
            for (j, spec) in c.predictors.iter().enumerate() {
                if let Distribution::Normal { .. } = spec.distribution {
                    assert!(
                        p.features[j] >= spec.min - 1e-9 && p.features[j] <= spec.max + 1e-9,
                        "{} = {} вне [{}, {}]",
                        spec.name,
                        p.features[j],
                        spec.min,
                        spec.max
                    );
                }
            }
        }
    }

    #[test]
    fn eta_and_probability_are_consistent_with_truth() {
        let c = small_config(2024);
        let cohort = Cohort::generate(&c);
        let betas: Vec<f64> = c.predictors.iter().map(|s| s.beta_true).collect();
        let b0 = cohort.diagnostics.intercept_true;
        for p in cohort.validation.iter().take(50) {
            let eta = b0
                + p.features
                    .iter()
                    .zip(&betas)
                    .map(|(x, b)| b * x)
                    .sum::<f64>();
            assert!((eta - p.eta_true).abs() < 1e-9);
            assert!((detmath::sigmoid(eta) - p.p_true).abs() < 1e-12);
        }
    }

    #[test]
    fn csv_has_header_and_all_rows() {
        let c = small_config(11);
        let cohort = Cohort::generate(&c);
        let csv = cohort.to_csv();
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), c.n_total() + 1);
        assert!(lines[0].starts_with("id,split,hba1c_pct"));
        assert!(lines[0].ends_with("eta_true,p_true,outcome"));
        for (i, line) in lines[1..].iter().enumerate() {
            // 2 служебных столбца + признаки + eta + p + исход
            assert_eq!(
                line.split(',').count(),
                2 + c.n_features() + 3,
                "строка {i}"
            );
        }
    }

    #[test]
    fn diagnostics_are_populated() {
        let c = small_config(5);
        let cohort = Cohort::generate(&c);
        let d = &cohort.diagnostics;
        assert!(d.calibration_iterations > 0);
        assert!(d.eta_sd_all > 0.0);
        assert!(d.eta_min_all < d.eta_max_all);
        assert!(d.max_abs_eta_all > 0.0);
        assert_eq!(d.feature_summary.len(), c.n_features());
        for f in &d.feature_summary {
            assert_eq!(f.missing_fraction, 0.0);
            assert!(f.sd > 0.0, "{}: sd = {}", f.name, f.sd);
            assert!(f.min <= f.max);
        }
    }
}
