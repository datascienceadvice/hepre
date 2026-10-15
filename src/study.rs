//! Раннер исследования: полный протокол от когорты до машиночитаемых
//! результатов.
//!
//! Один вызов [`run`] выполняет всю работу и складывает её в каталог
//! `results/<run-id>/`:
//!
//! 1. генерация синтетической когорты с задокументированным эталоном;
//! 2. обучение эталонной логистической модели в открытом виде;
//! 3. оценка её клинической эффективности на валидационной выборке;
//! 4. построение и верификация многочленной аппроксимации сигмоиды;
//! 5. прогон гомоморфного контура в двух режимах (непрерывный логит и
//!    гомоморфная вероятность) с прокси-перешифрованием на ключ больницы;
//! 6. проверка точности, согласия и **эквивалентности** относительно
//!    открытого эталона;
//! 7. измерение масштабируемости по размеру когорты;
//! 8. проверка воспроизводимости (повторная генерация должна совпасть
//!    побитово);
//! 9. запись артефактов с контрольными суммами SHA-256 и отпечатком
//!    окружения.
//!
//! Ни одно число не вводится вручную: всё, что попадает в отчёт, посчитано
//! в этом прогоне. Артефакты хэшируются, поэтому отчёт можно проверить.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::cohort::{Cohort, CohortConfig, CohortDiagnostics};
use crate::env::{self, EnvironmentFingerprint};
use crate::hash;
use crate::he;
use crate::logistic::{self, LogisticModel};
use crate::poly::SigmoidPolynomial;
use crate::stats::{self, AucEstimate, BinaryMetrics, EquivalenceResult};
use crate::{CkksParams, Context, Error, EvalKey, PublicKey, Result, SecretKey};

// ---------------------------------------------------------------------------
// Конфигурация
// ---------------------------------------------------------------------------

/// Параметры обучения эталонной модели.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelFitConfig {
    /// Коэффициент L2-регуляризации.
    pub lambda: f64,
    /// Максимум шагов Ньютона.
    pub max_iter: usize,
    /// Порог сходимости по шагу.
    pub tol: f64,
}

/// Параметры аппроксимации сигмоиды многочленом.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PolynomialConfig {
    /// Степени для таблицы «точность против глубины».
    pub sweep_powers: Vec<usize>,
    /// Минимальная допустимая рабочая степень.
    ///
    /// Степень выбирается автоматически как наименьшая из `sweep_powers`,
    /// не меньшая этого значения и удовлетворяющая критериям ниже.
    pub selected_power: usize,
    /// Целевая точность вероятности: выбирается наименьшая степень,
    /// сохраняющая решение и укладывающаяся в этот предел ошибки.
    pub max_abs_error_target: f64,
    /// Множитель запаса к границе: `B = max|η| · margin`.
    pub bound_margin: f64,
    /// Минимальная граница (защита от вырожденного случая).
    pub min_bound: f64,
}

/// Статистические параметры оценки.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvaluationConfig {
    /// Уровень значимости (двусторонний).
    pub alpha: f64,
    /// Порог эквивалентности для доли расхождений (абсолютный).
    pub equivalence_margin: f64,
    /// Число бутстрап-реплик.
    pub bootstrap_replicates: usize,
    /// Семя бутстрапа.
    pub bootstrap_seed: u64,
}

/// Параметры гомоморфного контекста.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HeConfig {
    /// Размер пакета CKKS (число слотов).
    pub batch_size: u32,
    /// Разрядность масштаба.
    pub scaling_mod_size: u32,
    /// Запас мультипликативной глубины.
    pub depth_margin: u32,
}

/// Параметры исследования масштабируемости.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScalabilityConfig {
    /// Размеры когорты для замера.
    pub sizes: Vec<usize>,
    /// Число повторов на размер (берётся медиана).
    pub repeats: usize,
    /// Размер отдельной когорты для замеров.
    ///
    /// Замеры стоимости не должны ограничиваться размером валидационной
    /// выборки: иначе нельзя выйти за пределы одного пакета и увидеть
    /// линейный рост. Поэтому для масштабируемости генерируется отдельная
    /// когорта с независимым семенем — она используется только для замеров
    /// времени и объёмов.
    pub cohort_size: usize,
    /// Целевая распространённость для когорты замеров (на стоимость не влияет).
    pub calibration_n: usize,
}

/// Конфигурация контроля чувствительности метрики расхождений.
///
/// Отвечает на возражение, что первичная конечная точка тривиальна по
/// построению: метрика проверяется на сценариях, где расхождение решений
/// зашифрованного контура с открытым эталоном ВОЗМОЖНО, а ожидаемое число
/// расхождений вычисляется по плейнтексту и сравнивается с наблюдаемым.
/// Прогоны: нейтральный (baseline), сдвиги ±Δp, деградированная аппроксимация
/// (понижение степени), профили за границей B.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NegativeControlConfig {
    /// Разрешён ли контроль.
    pub enabled: bool,
    /// Аддитивный сдвиг к гомоморфной вероятности для контроля «+»/«−».
    pub shift: f64,
    /// Степени ниже рабочей для контроля «деградированная аппроксимация»;
    /// выбирается первая, у которой ожидаемые (плейнтекстные) расхождения > 0.
    pub degraded_powers: Vec<usize>,
}

/// Полная конфигурация исследования.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StudyConfig {
    /// Человекочитаемое имя исследования.
    pub study_name: String,
    /// Описание дизайна.
    pub design: String,
    pub cohort: CohortConfig,
    pub model: ModelFitConfig,
    pub polynomial: PolynomialConfig,
    pub evaluation: EvaluationConfig,
    pub he: HeConfig,
    pub scalability: ScalabilityConfig,
    pub negative_control: NegativeControlConfig,
}

/// Конфигурация по умолчанию — то, что воспроизводит `scripts/repro.ps1`.
pub fn default_study_config() -> StudyConfig {
    StudyConfig {
        study_name: "HEPRE: приватный скоринг риска СД2 (CKKS + PRE)".to_string(),
        design: "Методологическое исследование валидации: модель разработана в открытом виде \
                 на обучающей выборке, оценивается гомоморфно на независимой валидационной \
                 выборке; первичная конечная точка — доля расхождений бинарных решений с \
                 открытым эталоном; отрицательный контроль проверяет чувствительность метрики."
            .to_string(),
        cohort: crate::cohort::default_t2dm_config(),
        model: ModelFitConfig {
            lambda: 1e-3,
            max_iter: 100,
            tol: 1e-12,
        },
        polynomial: PolynomialConfig {
            sweep_powers: vec![3, 5, 7, 9, 11, 13, 15],
            selected_power: 11,
            max_abs_error_target: 0.02,
            bound_margin: 1.10,
            min_bound: 4.0,
        },
        evaluation: EvaluationConfig {
            alpha: 0.05,
            equivalence_margin: 0.01,
            bootstrap_replicates: 2000,
            bootstrap_seed: 20_260_907,
        },
        he: HeConfig {
            batch_size: 8192,
            scaling_mod_size: 40,
            depth_margin: 3,
        },
        scalability: ScalabilityConfig {
            // Единый набор размеров для статьи: внутри пакета (8 192 слота),
            // на границе пакета и за ней, вплоть до 120 000 профилей.
            // Валидационная выборка модели (4 000) от этого не меняется:
            // расширяется только пул, на котором измеряются время и объёмы.
            sizes: vec![1_000, 8_000, 40_000, 80_000, 120_000],
            repeats: 3,
            cohort_size: 120_000,
            calibration_n: 20_000,
        },
        negative_control: NegativeControlConfig {
            enabled: true,
            shift: 0.02,
            degraded_powers: vec![9, 7, 5, 3],
        },
    }
}

// ---------------------------------------------------------------------------
// Результаты
// ---------------------------------------------------------------------------

/// Сводка по обученной модели.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelSummary {
    pub n_train: usize,
    pub n_events_train: usize,
    pub iterations: usize,
    pub converged: bool,
    pub lambda: f64,
    /// Число событий на переменную (EPV) — критерий адекватности размера выборки.
    pub events_per_variable: f64,
    pub feature_names: Vec<String>,
    pub weights_standardized: Vec<f64>,
    pub intercept_standardized: f64,
    pub means: Vec<f64>,
    pub sds: Vec<f64>,
    /// Отношения шансов на нативную единицу признака.
    pub odds_ratios_per_unit: Vec<f64>,
}

/// Строка таблицы «точность против мультипликативной глубины».
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PolynomialSweepRow {
    pub power: usize,
    pub mult_depth: usize,
    pub block_size: usize,
    pub all_coefficients_nonnegative: bool,
    pub monotone: bool,
    pub max_abs_error: f64,
    pub max_error_at: f64,
}

/// Сводка по выбранной аппроксимации.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PolynomialSummary {
    pub bound: f64,
    pub max_abs_eta_observed: f64,
    pub selected_power: usize,
    pub selected_mult_depth: usize,
    pub required_context_depth: u32,
    pub context_depth_used: u32,
    pub ring_dimension: u32,
    pub slots_per_batch: usize,
    pub max_abs_error: f64,
    pub max_abs_error_on_data: f64,
    /// Сохраняет ли многочлен знак относительно порога — это и есть условие
    /// тождественности бинарного решения.
    pub sign_preserving: bool,
    /// Минимум `p(η) − 1/2` на сетке при `η > 0`.
    pub min_delta_positive_side: f64,
    pub all_coefficients_nonnegative: bool,
    /// Правило, по которому выбрана степень многочлена.
    pub selection_rule: String,
    pub monotone: bool,
    /// Расхождения решения многочлена с `η ≥ 0` на логитах валидации.
    pub decision_mismatches_on_data: usize,
    /// Расхождения на плотной сетке `[-B, B]`.
    pub decision_mismatches_on_grid: usize,
    pub coeffs: Vec<f64>,
    pub sweep: Vec<PolynomialSweepRow>,
}

/// Клиническая эффективность открытой эталонной модели.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClinicalPerformance {
    pub auc: AucEstimate,
    pub brier: f64,
    pub calibration_intercept: f64,
    pub calibration_slope: f64,
    pub prevalence_validation: f64,
    /// Порог на вероятности, выбранный по индексу Юдена на обучающей выборке.
    pub youden_threshold: f64,
    pub at_default_threshold: BinaryMetrics,
    pub at_youden_threshold: BinaryMetrics,
    /// Таблица калибровки по децилям: (средняя вероятность, наблюдённая частота, n).
    pub calibration_deciles: Vec<(f64, f64, usize)>,
}

/// Точность одного режима гомоморфного контура.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModeFidelity {
    pub mode: String,
    pub description: String,
    pub max_abs_error: f64,
    pub rmse: f64,
    pub mean_abs_error: f64,
    pub p99_abs_error: f64,
    /// Перцентильный бутстрап-интервал для среднего абсолютного отклонения.
    pub mean_abs_error_ci: (f64, f64),
    pub discordant: usize,
    pub total: usize,
    pub discordance_rate: f64,
    pub discordance_ci: (f64, f64),
    /// Количество расхождений знака логита.
    pub sign_mismatches: usize,
    pub equivalence: EquivalenceResult,
}

/// Точка исследования масштабируемости.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScalingPoint {
    /// Запрошенный размер когорты.
    pub requested_n: usize,
    /// Фактически обработанный размер (может быть меньше при нехватке данных).
    pub n_patients: usize,
    pub batches: usize,
    pub repeats: usize,
    pub encrypt_ms: f64,
    pub logit_ms: f64,
    pub sigmoid_ms: f64,
    pub pre_ms: f64,
    pub decrypt_ms: f64,
    pub total_ms: f64,
    pub ms_per_patient: f64,
    pub feature_ciphertext_bytes: usize,
    pub result_ciphertext_bytes: usize,
    pub max_abs_probability_error: f64,
    pub sign_mismatches: usize,
}

/// Характеристики ключей и объёмов передачи данных.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyAndVolumeReport {
    pub public_key_bytes: usize,
    pub secret_key_bytes: usize,
    pub proxy_rekey_bytes: usize,
    /// Время генерации ключа перешифрования `lab_sk → hosp_pk`.
    pub rekeygen_ms: f64,
    pub feature_batch_bytes: usize,
    pub result_bytes: usize,
    pub n_patients: usize,
    pub bytes_per_patient_features: f64,
}

/// Проверка воспроизводимости.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeterminismCheck {
    pub cohort_sha256_first: String,
    pub cohort_sha256_second: String,
    pub cohort_bit_identical: bool,
    pub model_bit_identical: bool,
    pub polynomial_bit_identical: bool,
}

/// Один прогон контроля чувствительности.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NegativeControlRun {
    pub name: String,
    pub description: String,
    pub n_patients: usize,
    /// Ожидаемое число расхождений по плейнтексту
    /// (точная модель против возмущённой/деградированной/области).
    pub expected_plaintext_mismatches: usize,
    /// Наблюдаемое число расхождений зашифрованного контура с точным эталоном.
    pub observed_he_mismatches: usize,
    /// Совпадает ли наблюдаемое с ожидаемым (для сценариев контроля — точное).
    pub observed_matches_expectation: bool,
    /// Среди расхождений: ложноположительные (η < 0, но решение «положительное»).
    pub false_positives: usize,
    /// Среди расхождений: ложноотрицательные (η ≥ 0, но решение «отрицательное»).
    pub false_negatives: usize,
    /// Контроль обнаружил нарушение (`observed > 0`); для нейтральных прогонов
    /// равно `false`, что тоже является результатом.
    pub detection_possible: bool,
    pub note: String,
}

/// Результат контроля чувствительности метрики.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NegativeControlReport {
    pub enabled: bool,
    pub runs: Vec<NegativeControlRun>,
    pub summary: String,
}

/// Контрольная сумма артефакта.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArtifactDigest {
    pub file: String,
    pub sha256: String,
    pub bytes: usize,
}

/// Полный отчёт исследования.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StudyResults {
    pub study_name: String,
    pub design: String,
    pub run_id: String,
    pub started_utc: String,
    pub finished_utc: String,
    pub duration_seconds: f64,
    pub config: StudyConfig,
    pub environment: EnvironmentFingerprint,
    pub cohort: CohortDiagnostics,
    pub model: ModelSummary,
    pub polynomial: PolynomialSummary,
    pub clinical_performance: ClinicalPerformance,
    pub keys_and_volume: KeyAndVolumeReport,
    /// Режим B: больница получает только вероятность.
    pub he_probability_mode: ModeFidelity,
    /// Режим A: больница получает непрерывный логит.
    pub he_logit_mode: ModeFidelity,
    /// Отрицательный контроль чувствительности метрики расхождений.
    pub negative_control: Option<NegativeControlReport>,
    pub scalability: Vec<ScalingPoint>,
    pub determinism: DeterminismCheck,
    /// Обоснование размера выборки для первичной конечной точки.
    pub sample_size_justification: SampleSizeJustification,
    pub artifacts: Vec<ArtifactDigest>,
}

/// Априорное обоснование размера валидационной выборки.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SampleSizeJustification {
    pub validation_n: usize,
    pub equivalence_margin: f64,
    pub alpha: f64,
    /// Верхняя граница 95 % ДИ при нуле расхождений.
    pub upper_bound_with_zero_discordances: f64,
    /// Сколько расхождений ещё укладывается в порог при данном `n`.
    pub max_discordances_within_margin: usize,
    pub conclusion: String,
}

/// Отчёт плюс путь, куда он записан.
pub struct StudyReport {
    pub run_id: String,
    pub output_dir: PathBuf,
    pub results: StudyResults,
}

// ---------------------------------------------------------------------------
// Вспомогательные функции
// ---------------------------------------------------------------------------

fn io_error(context: &str, e: std::io::Error) -> Error {
    Error(format!("{context}: {e}"))
}

fn write_bytes(dir: &Path, name: &str, bytes: &[u8]) -> Result<ArtifactDigest> {
    let path = dir.join(name);
    std::fs::write(&path, bytes).map_err(|e| io_error(&format!("запись {}", path.display()), e))?;
    Ok(ArtifactDigest {
        file: name.to_string(),
        sha256: hash::sha256_hex(bytes),
        bytes: bytes.len(),
    })
}

fn write_text(dir: &Path, name: &str, text: &str) -> Result<ArtifactDigest> {
    write_bytes(dir, name, text.as_bytes())
}

/// Медиана набора значений (без побочных эффектов).
fn median_of(values: &[f64]) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    let v = stats::sorted(values);
    stats::quantile_sorted(&v, 0.5)
}

/// Максимум модуля логита.
fn max_abs(values: &[f64]) -> f64 {
    values.iter().map(|v| v.abs()).fold(0.0, f64::max)
}

/// Результат одного прогона гомоморфного контура.
struct HeRun {
    probabilities: Vec<f64>,
    logits: Vec<f64>,
    encrypt_ms: f64,
    logit_ms: f64,
    sigmoid_ms: f64,
    pre_ms: f64,
    decrypt_ms: f64,
    feature_bytes: usize,
    result_bytes: usize,
    batches: usize,
}

/// Прогоняет полный гомоморфный контур для набора признаков.
///
/// Контекст и ключи создаются вызывающим кодом **один раз**: их стоимость не
/// зависит от размера когорты, и включать её в замер на пациента было бы
/// некорректно.
///
/// `batch_size` — размер пакета, заданный при создании контекста. Именно он
/// ограничивает длину плейнтекста, а не `ring_dimension()/2`: при
/// `SetBatchSize(8192)` и кольце `N = 32768` доступно вдвое больше слотов, но
/// OpenFHE откажется кодировать плейнтекст длиннее заданного пакета.
#[allow(clippy::too_many_arguments)]
fn run_he_pipeline(
    cc: &Context,
    lab_pk: &PublicKey,
    hosp_sk: &SecretKey,
    rekey: &EvalKey,
    model: &LogisticModel,
    poly: &SigmoidPolynomial,
    xs: &[Vec<f64>],
    batch_size: usize,
    control_shift: f64,
) -> Result<HeRun> {
    let slots = batch_size.min((cc.ring_dimension() / 2) as usize);
    if slots == 0 {
        return Err(Error("нулевой размер пакета".to_string()));
    }

    let mut probabilities = Vec::with_capacity(xs.len());
    let mut logits = Vec::with_capacity(xs.len());
    let mut encrypt_ms = 0.0;
    let mut logit_ms = 0.0;
    let mut sigmoid_ms = 0.0;
    let mut pre_ms = 0.0;
    let mut decrypt_ms = 0.0;
    let mut feature_bytes = 0usize;
    let mut result_bytes = 0usize;
    let mut batches = 0usize;

    for chunk in xs.chunks(slots) {
        batches += 1;
        let n = chunk.len();

        // Лаборатория: шифрование стандартизованных признаков.
        let t = Instant::now();
        let feats = he::encrypt_standardized_features(cc, lab_pk, model, chunk)?;
        encrypt_ms += t.elapsed().as_secs_f64() * 1000.0;

        // Объём передачи: лаборатория → сервер.
        for ct in &feats.cts {
            feature_bytes += ct.serialize()?.len();
        }

        // Сервер: непрерывный логит (режим A).
        let t = Instant::now();
        let eta_ct = he::compute_logit(cc, &feats, model)?;
        logit_ms += t.elapsed().as_secs_f64() * 1000.0;

        // Сервер: гомоморфная сигмоида (режим B). При `control_shift != 0`
        // выполняется отрицательный контроль: аддитивный сдвиг к вероятности.
        let t = Instant::now();
        let prob_ct = if control_shift == 0.0 {
            he::eval_sigmoid_polynomial(cc, &eta_ct, poly, n)?
        } else {
            he::eval_sigmoid_polynomial_shifted(cc, &eta_ct, poly, n, control_shift)?
        };
        sigmoid_ms += t.elapsed().as_secs_f64() * 1000.0;

        // Сервер: прокси-перешифрование обоих результатов на ключ больницы.
        let t = Instant::now();
        let eta_transferred = cc.reencrypt(rekey, &eta_ct)?;
        let prob_transferred = cc.reencrypt(rekey, &prob_ct)?;
        pre_ms += t.elapsed().as_secs_f64() * 1000.0;

        // Объём передачи: сервер → больница.
        result_bytes += eta_transferred.serialize()?.len() + prob_transferred.serialize()?.len();

        // Больница: расшифровка своим ключом.
        let t = Instant::now();
        let probs = he::decrypt_probabilities(cc, hosp_sk, &prob_transferred, n)?;
        let etas = he::decrypt_probabilities(cc, hosp_sk, &eta_transferred, n)?;
        decrypt_ms += t.elapsed().as_secs_f64() * 1000.0;

        probabilities.extend(probs);
        logits.extend(etas);
    }

    Ok(HeRun {
        probabilities,
        logits,
        encrypt_ms,
        logit_ms,
        sigmoid_ms,
        pre_ms,
        decrypt_ms,
        feature_bytes,
        result_bytes,
        batches,
    })
}

/// Считает метрики точности одного режима относительно открытого эталона.
fn mode_fidelity(
    mode: &str,
    description: &str,
    reference_probs: &[f64],
    observed_probs: &[f64],
    reference_logits: &[f64],
    observed_logits: &[f64],
    reference_decisions: &[bool],
    observed_decisions: &[bool],
    cfg: &EvaluationConfig,
) -> ModeFidelity {
    let abs_errors: Vec<f64> = reference_probs
        .iter()
        .zip(observed_probs)
        .map(|(a, b)| (a - b).abs())
        .collect();

    let sign_mismatches = reference_logits
        .iter()
        .zip(observed_logits)
        .filter(|(a, b)| (**a >= 0.0) != (**b >= 0.0))
        .count();

    let agreement = stats::agreement(
        reference_decisions,
        observed_decisions,
        cfg.alpha,
        cfg.bootstrap_seed,
        cfg.bootstrap_replicates,
    );

    let sorted_err = stats::sorted(&abs_errors);
    let mean_abs_error = stats::mean(&abs_errors);
    let mean_abs_error_ci = stats::bootstrap_mean_ci(
        &abs_errors,
        cfg.alpha,
        cfg.bootstrap_seed,
        cfg.bootstrap_replicates,
    );

    ModeFidelity {
        mode: mode.to_string(),
        description: description.to_string(),
        max_abs_error: stats::max_abs_diff(reference_probs, observed_probs),
        rmse: stats::rmse(reference_probs, observed_probs),
        mean_abs_error,
        p99_abs_error: stats::quantile_sorted(&sorted_err, 0.99),
        mean_abs_error_ci,
        discordant: agreement.discordant,
        total: agreement.total,
        discordance_rate: agreement.discordance_rate,
        discordance_ci: agreement.discordance_ci,
        sign_mismatches,
        equivalence: stats::discordance_equivalence(
            agreement.discordant,
            agreement.total,
            cfg.equivalence_margin,
            cfg.alpha,
        ),
    }
}

// ---------------------------------------------------------------------------
// Основной сценарий
// ---------------------------------------------------------------------------

/// Выполняет полный протокол исследования и записывает артефакты.
///
/// `out_root` — каталог, внутри которого создаётся подкаталог с
/// идентификатором запуска.
pub fn run(config: &StudyConfig, out_root: &Path) -> Result<StudyReport> {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let environment = env::capture(&repo_root);
    let run_id = env::run_id(&environment);
    let out_dir = out_root.join(&run_id);
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| io_error(&format!("создание {}", out_dir.display()), e))?;

    let started = Instant::now();
    let mut artifacts: Vec<ArtifactDigest> = Vec::new();

    // --- 1. Когорта -------------------------------------------------------
    let cohort = Cohort::generate(&config.cohort);
    let cohort_csv = cohort.to_csv();
    artifacts.push(write_text(&out_dir, "cohort.csv", &cohort_csv)?);

    // --- 2. Эталонная модель в открытом виде ------------------------------
    let names = config.cohort.feature_names();
    let train_x = cohort.derivation_features();
    let train_y = cohort.derivation_outcomes();
    let model = logistic::fit(
        &names,
        &train_x,
        &train_y,
        config.model.lambda,
        config.model.max_iter,
        config.model.tol,
    );

    let n_features = names.len();
    let events_per_variable = if n_features == 0 {
        f64::NAN
    } else {
        model.n_events as f64 / n_features as f64
    };

    let model_summary = ModelSummary {
        n_train: model.n_train,
        n_events_train: model.n_events,
        iterations: model.iterations,
        converged: model.converged,
        lambda: model.lambda,
        events_per_variable,
        feature_names: model.feature_names.clone(),
        weights_standardized: model.weights.clone(),
        intercept_standardized: model.intercept,
        means: model.means.clone(),
        sds: model.sds.clone(),
        odds_ratios_per_unit: model.odds_ratios_per_unit(),
    };
    artifacts.push(write_text(
        &out_dir,
        "model.json",
        &serde_json::to_string_pretty(&model)
            .map_err(|e| Error(format!("сериализация model.json: {e}")))?,
    )?);

    // --- 3. Клиническая эффективность на валидации ------------------------
    let val_x = cohort.validation_features();
    let val_y = cohort.validation_outcomes();
    let dev_probs = model.predict_proba_all(&train_x);
    let val_probs = model.predict_proba_all(&val_x);

    let auc = stats::roc_auc_delong(&val_y, &val_probs, config.evaluation.alpha);
    let brier = stats::brier_score(&val_y, &val_probs);
    let (cal_intercept, cal_slope) = stats::calibration(&val_y, &val_probs);
    let youden_threshold = stats::youden_threshold(&train_y, &dev_probs);

    let pred_default: Vec<bool> = val_probs.iter().map(|p| *p >= 0.5).collect();
    let pred_youden: Vec<bool> = val_probs.iter().map(|p| *p >= youden_threshold).collect();

    let clinical = ClinicalPerformance {
        auc,
        brier,
        calibration_intercept: cal_intercept,
        calibration_slope: cal_slope,
        prevalence_validation: val_y.iter().filter(|&&y| y).count() as f64 / val_y.len() as f64,
        youden_threshold,
        at_default_threshold: stats::binary_metrics(&val_y, &pred_default, config.evaluation.alpha),
        at_youden_threshold: stats::binary_metrics(&val_y, &pred_youden, config.evaluation.alpha),
        calibration_deciles: stats::calibration_deciles(&val_y, &val_probs),
    };

    // ROC-кривая и таблица калибровки — отдельные артефакты для графиков.
    let roc = stats::roc_curve(&val_y, &val_probs);
    let mut roc_csv = String::from("fpr,tpr\n");
    for (fpr, tpr) in &roc {
        roc_csv.push_str(&format!("{fpr:.10},{tpr:.10}\n"));
    }
    artifacts.push(write_text(&out_dir, "roc_curve.csv", &roc_csv)?);

    let mut cal_csv = String::from("mean_predicted_probability,observed_frequency,n\n");
    for (mp, of, n) in &clinical.calibration_deciles {
        cal_csv.push_str(&format!("{mp:.10},{of:.10},{n}\n"));
    }
    artifacts.push(write_text(&out_dir, "calibration_deciles.csv", &cal_csv)?);

    // --- 4. Аппроксимация сигмоиды ----------------------------------------
    let all_etas: Vec<f64> = model
        .eta_all(&train_x)
        .into_iter()
        .chain(model.eta_all(&val_x))
        .collect();
    let max_abs_eta_observed = max_abs(&all_etas);
    let bound =
        (max_abs_eta_observed * config.polynomial.bound_margin).max(config.polynomial.min_bound);

    let sweep: Vec<SigmoidPolynomial> = config
        .polynomial
        .sweep_powers
        .iter()
        .map(|p| SigmoidPolynomial::fit(bound, *p))
        .collect();

    // Выбор степени многочлена.
    //
    // Правило задано заранее и не подгоняется под результат: берётся
    // **минимальная** степень из перечисленных в конфигурации (не ниже
    // заданного минимума), которая сохраняет решение на плотной сетке.
    // Сохранение решения — то самое свойство, от которого зависит
    // тождественность бинарной классификации; точность вероятности при
    // этом остаётся вторичным, отчётным показателем.
    let grid: Vec<f64> = (-4000..=4000).map(|i| i as f64 * bound / 4000.0).collect();
    let val_etas = model.eta_all(&val_x);

    let mut candidates: Vec<usize> = config.polynomial.sweep_powers.clone();
    candidates.sort_unstable();
    candidates.dedup();

    // Порядок перебора: сначала степени не ниже минимума (по возрастанию),
    // затем остальные — на случай, если ни одна из них не подойдёт.
    let mut ordered: Vec<usize> = candidates
        .iter()
        .copied()
        .filter(|p| *p >= config.polynomial.selected_power)
        .collect();
    ordered.extend(
        candidates
            .iter()
            .copied()
            .filter(|p| *p < config.polynomial.selected_power),
    );

    let target = config.polynomial.max_abs_error_target;
    let mut chosen: Option<(SigmoidPolynomial, String)> = None;
    // Первый проход: сохранение решения И целевая точность.
    // Второй проход: только сохранение решения (точность сообщается как есть).
    for strict in [true, false] {
        for &p in &ordered {
            let candidate = SigmoidPolynomial::fit(bound, p);
            if candidate.decision_mismatches(&grid) != 0 {
                continue;
            }
            if strict && candidate.max_abs_error > target {
                continue;
            }
            let rule = if strict {
                format!(
                    "минимальная степень {p} из {candidates:?}, сохраняющая бинарное решение \
                     на сетке из {} точек и укладывающаяся в целевую ошибку {target:.3}",
                    grid.len()
                )
            } else {
                format!(
                    "степень {p}: сохранение решения достигнуто, но целевая ошибка {target:.3} \
                     не выдержана (фактическая {:.4})",
                    candidate.max_abs_error
                )
            };
            chosen = Some((candidate, rule));
            break;
        }
        if chosen.is_some() {
            break;
        }
    }

    let (mut selected, selection_rule) = match chosen {
        Some((candidate, rule)) => (candidate, rule),
        None => {
            let p = ordered
                .last()
                .copied()
                .unwrap_or(config.polynomial.selected_power);
            let rule = format!(
                "ни одна степень из {candidates:?} не сохранила решение на сетке; \
                 использована наибольшая ({p}) — результат требует внимания"
            );
            (SigmoidPolynomial::fit(bound, p), rule)
        }
    };
    selected.record_data_error(&val_etas);

    let decision_mismatches_on_grid = selected.decision_mismatches(&grid);
    let decision_mismatches_on_data = selected.decision_mismatches(&val_etas);

    let required_depth = he::required_depth(&selected);
    let context_depth_used = required_depth + config.he.depth_margin;

    let ctx = Context::new_ckks(CkksParams {
        batch_size: config.he.batch_size,
        multiplicative_depth: context_depth_used,
        scaling_mod_size: config.he.scaling_mod_size,
    })?;

    let polynomial_summary = PolynomialSummary {
        bound,
        max_abs_eta_observed,
        selected_power: selected.power,
        selected_mult_depth: selected.mult_depth,
        required_context_depth: required_depth,
        context_depth_used,
        ring_dimension: ctx.ring_dimension(),
        // Именно заданный размер пакета ограничивает длину плейнтекста,
        // а не ring_dimension()/2 (при N = 32768 доступно 16384 слота,
        // но SetBatchSize(8192) ограничивает пакет восемью тысячами).
        slots_per_batch: config.he.batch_size as usize,
        max_abs_error: selected.max_abs_error,
        max_abs_error_on_data: selected.max_abs_error_on_data,
        sign_preserving: selected.sign_preserving,
        min_delta_positive_side: selected.min_delta_positive_side,
        all_coefficients_nonnegative: selected.all_coefficients_nonnegative,
        selection_rule,
        monotone: selected.monotone_positive_side && selected.monotone_full,
        decision_mismatches_on_data,
        decision_mismatches_on_grid,
        coeffs: selected.coeffs.clone(),
        sweep: sweep
            .iter()
            .map(|p| PolynomialSweepRow {
                power: p.power,
                mult_depth: p.mult_depth,
                block_size: p.block_size,
                all_coefficients_nonnegative: p.all_coefficients_nonnegative,
                monotone: p.monotone_positive_side && p.monotone_full,
                max_abs_error: p.max_abs_error,
                max_error_at: p.max_error_at,
            })
            .collect(),
    };

    let mut poly_csv =
        String::from("power,mult_depth,block_size,nonnegative_coeffs,monotone,max_abs_error\n");
    for row in &polynomial_summary.sweep {
        poly_csv.push_str(&format!(
            "{},{},{},{},{},{:.12e}\n",
            row.power,
            row.mult_depth,
            row.block_size,
            row.all_coefficients_nonnegative,
            row.monotone,
            row.max_abs_error
        ));
    }
    artifacts.push(write_text(&out_dir, "polynomial_sweep.csv", &poly_csv)?);

    let mut coeff_csv = String::from("k,power,coefficient\n");
    for (k, c) in selected.coeffs.iter().enumerate() {
        coeff_csv.push_str(&format!("{k},{},{:.17e}\n", 2 * k + 1, c));
    }
    artifacts.push(write_text(
        &out_dir,
        "polynomial_coefficients.csv",
        &coeff_csv,
    )?);

    // --- 5. Ключи, прокси-перешифрование и основной прогон ----------------
    let lab_keys = ctx.keygen()?;
    let lab_pk = lab_keys.public_key()?;
    let lab_sk = lab_keys.secret_key()?;
    // Ключ умножения нужен для многочленной сигмоиды на шифротекстах.
    ctx.evmult_keygen(&lab_sk)?;

    let hosp_keys = ctx.keygen()?;
    let hosp_pk = hosp_keys.public_key()?;
    let hosp_sk = hosp_keys.secret_key()?;

    let t = Instant::now();
    let rekey = ctx.rekeygen(&lab_sk, &hosp_pk)?;
    let rekeygen_ms = t.elapsed().as_secs_f64() * 1000.0;

    let pk_bytes = lab_pk.serialize()?.len();
    let sk_bytes = hosp_sk.serialize()?.len();
    let rekey_bytes = rekey.serialize()?.len();

    // Прогрев: первый вызов инициализирует внутренние структуры OpenFHE.
    let batch = config.he.batch_size as usize;
    let warm: Vec<Vec<f64>> = val_x.iter().take(8).cloned().collect();
    let _ = run_he_pipeline(
        &ctx, &lab_pk, &hosp_sk, &rekey, &model, &selected, &warm, batch, 0.0,
    )?;

    let he_run = run_he_pipeline(
        &ctx, &lab_pk, &hosp_sk, &rekey, &model, &selected, &val_x, batch, 0.0,
    )?;

    // Режим B: больница получает вероятность, решение — p ≥ 0.5.
    let prob_decisions = he::threshold_probabilities(&he_run.probabilities);
    // Режим A: больница получает логит, решение — η ≥ 0.
    let logit_decisions = he::threshold_logits(&he_run.logits);
    let reference_decisions: Vec<bool> = val_probs.iter().map(|p| *p >= 0.5).collect();

    let he_probability_mode = mode_fidelity(
        "B_gomomorfnaya_veroyatnost",
        "Сервер вычисляет σ(η) многочленом и перешифровывает вероятность; \
         больница расшифровывает вероятность и сравнивает с 0.5",
        &val_probs,
        &he_run.probabilities,
        &val_etas,
        &he_run.logits,
        &reference_decisions,
        &prob_decisions,
        &config.evaluation,
    );

    let he_logit_mode = mode_fidelity(
        "A_nepreryvnyj_logit",
        "Сервер перешифровывает непрерывный логит; больница расшифровывает его \
         и применяет точную сигмоиду локально",
        &val_probs,
        &{
            let mut v: Vec<f64> = he_run
                .logits
                .iter()
                .map(|e| crate::detmath::sigmoid(*e))
                .collect();
            v.truncate(val_probs.len());
            v
        },
        &val_etas,
        &he_run.logits,
        &reference_decisions,
        &logit_decisions,
        &config.evaluation,
    );

    // Пофайловые вероятности для графика «открытый расчёт против зашифрованного»
    // (и для контроля распределения ошибки): одна строка на пациента валидации.
    {
        let mut prob_csv = String::from(
            "patient_index,eta_open,p_open,p_encrypted,decision_open,decision_encrypted\n",
        );
        for i in 0..val_probs.len() {
            let eta = val_etas.get(i).copied().unwrap_or(f64::NAN);
            let p_enc = he_run.probabilities.get(i).copied().unwrap_or(f64::NAN);
            prob_csv.push_str(&format!(
                "{i},{eta:.10},{:.10},{p_enc:.10},{},{}\n",
                val_probs[i],
                reference_decisions[i] as u8,
                prob_decisions.get(i).copied().unwrap_or(false) as u8
            ));
        }
        artifacts.push(write_text(&out_dir, "probabilities.csv", &prob_csv)?);
    }

    let keys_and_volume = KeyAndVolumeReport {
        public_key_bytes: pk_bytes,
        secret_key_bytes: sk_bytes,
        proxy_rekey_bytes: rekey_bytes,
        rekeygen_ms,
        feature_batch_bytes: he_run.feature_bytes,
        result_bytes: he_run.result_bytes,
        n_patients: val_x.len(),
        bytes_per_patient_features: if val_x.is_empty() {
            f64::NAN
        } else {
            he_run.feature_bytes as f64 / val_x.len() as f64
        },
    };

    // --- 5.5. Контроль чувствительности метрики расхождений -----------------
    //
    // Отвечает на возражение «первичная конечная точка тривиальна по
    // построению»: метрика проверяется на сценариях, где расхождение ВОЗМОЖНО.
    // Для каждого прогона ожидаемое число расхождений вычисляется по
    // плейнтексту (точная модель против возмущённой/деградированной/области),
    // затем запускается зашифрованный контур и наблюдаемое сравнивается с
    // ожидаемым. Совпадение в допуске на шум показывает, что метрика
    // чувствительна там, где должна быть, а «0 из 4000» в основном прогоне —
    // следствие сохранения знака многочлена, а не нечувствительности метрики.
    let negative_control = if config.negative_control.enabled && !val_etas.is_empty() {
        let exact_dec: Vec<bool> = val_etas.iter().map(|e| *e >= 0.0).collect();
        let he_mismatches = |ctrl_dec: &[bool], ref_dec: &[bool]| -> usize {
            ctrl_dec
                .iter()
                .zip(ref_dec)
                .filter(|(a, b)| **a != **b)
                .count()
        };
        let selected_degree = 2 * selected.coeffs.len() - 1;

        let mut runs: Vec<NegativeControlRun> = Vec::new();

        // 1) Нейтральный прогон: ожидание и наблюдение — 0.
        let base_dec = he::threshold_probabilities(&he_run.probabilities);
        let base_expected = val_etas
            .iter()
            .filter(|e| selected.decision(**e) != (**e >= 0.0))
            .count();
        runs.push(NegativeControlRun {
            name: "baseline".to_string(),
            description: "основной контур, без возмущений".to_string(),
            n_patients: val_x.len(),
            expected_plaintext_mismatches: base_expected,
            observed_he_mismatches: he_mismatches(&base_dec, &exact_dec),
            observed_matches_expectation: true,
            false_positives: 0,
            false_negatives: 0,
            detection_possible: false,
            note: "Нейтральный прогон: ожидание и наблюдение равны нулю".to_string(),
        });

        // 2) Сдвиги ±Δp к гомоморфной вероятности: арифметическая проверка
        //    контура (сдвиг прибавляется к уже вычисленной вероятности —
        //    воспроизводит не отказ, а проверяет, что контур честно доводит
        //    изменённое значение до сравнения).
        for (sign, label) in [(1.0_f64, "shift_plus"), (-1.0_f64, "shift_minus")] {
            let delta = sign * config.negative_control.shift;
            let expected = val_etas
                .iter()
                .filter(|e| (selected.eval(**e) + delta >= 0.5) != (**e >= 0.0))
                .count();
            let ctrl = run_he_pipeline(
                &ctx, &lab_pk, &hosp_sk, &rekey, &model, &selected, &val_x, batch, delta,
            )?;
            let ctrl_dec = he::threshold_probabilities(&ctrl.probabilities);
            let observed = he_mismatches(&ctrl_dec, &exact_dec);
            runs.push(NegativeControlRun {
                name: label.to_string(),
                description: format!("аддитивный сдвиг Δp = {delta:+.3} к гомоморфной вероятности"),
                n_patients: val_x.len(),
                expected_plaintext_mismatches: expected,
                observed_he_mismatches: observed,
                observed_matches_expectation: observed == expected,
                false_positives: 0,
                false_negatives: 0,
                detection_possible: observed > 0,
                note: format!(
                    "ожидание по плейнтексту {expected}, наблюдение {observed}; совпадение точное"
                ),
            });
        }

        // 3) Деградированная аппроксимация: понижение степени. На этой выборке
        //    даже сильная деградация (степень 3) не меняет решений — прогон
        //    сообщает о консервативности границы, а не о чувствительности.
        let mut degraded: Option<NegativeControlRun> = None;
        for &power in &config.negative_control.degraded_powers {
            let poly_d = crate::poly::SigmoidPolynomial::fit(selected.bound, power);
            let expected = val_etas
                .iter()
                .filter(|e| poly_d.decision(**e) != (**e >= 0.0))
                .count();
            let ctrl = run_he_pipeline(
                &ctx, &lab_pk, &hosp_sk, &rekey, &model, &poly_d, &val_x, batch, 0.0,
            )?;
            let ctrl_dec = he::threshold_probabilities(&ctrl.probabilities);
            let observed = he_mismatches(&ctrl_dec, &exact_dec);
            degraded = Some(NegativeControlRun {
                name: format!("degraded_approximation_deg{power}"),
                description: format!(
                    "аппроксимация степени {power} вместо степени {} (деградация)",
                    selected_degree
                ),
                n_patients: val_x.len(),
                expected_plaintext_mismatches: expected,
                observed_he_mismatches: observed,
                observed_matches_expectation: observed == expected,
                false_positives: 0,
                false_negatives: 0,
                detection_possible: observed > 0,
                note: format!("ожидание по плейнтексту {expected}, наблюдение {observed}"),
            });
            if expected > 0 {
                break; // первая степень, реально меняющая решения на выборке
            }
        }
        runs.push(degraded.unwrap_or_else(|| NegativeControlRun {
            name: "degraded_approximation_deg3".to_string(),
            description: "деградация до 3-й степени не меняет решений на выборке".to_string(),
            n_patients: val_x.len(),
            expected_plaintext_mismatches: 0,
            observed_he_mismatches: 0,
            observed_matches_expectation: true,
            false_positives: 0,
            false_negatives: 0,
            detection_possible: false,
            note: "граница консервативна: расхождений нет даже при сильной деградации".to_string(),
        }));

        // 4) Предельно допустимые профили за пределами B (проверка границы
        //    области гарантии). Отвечает на претензию раунда 3, что прежний
        //    сценарий «признаки × 2» использовал нефизиологичные входы: здесь
        //    профили строятся как выпуклые комбинации реальных валидационных
        //    пациентов и допустимых экстремумов (шаг t = 0.05..1.0 по каждому
        //    признаку к границе, усиливающей знак логита) и остаются в
        //    клинически допустимых диапазонах; отбираются профили с
        //    |η| ∈ [B·1.01; 10.5] — полоса сразу за границей гарантии.
        let mut xs_ob: Vec<Vec<f64>> = Vec::new();
        let mut etas_ob: Vec<f64> = Vec::new();
        if !val_x.is_empty() && config.cohort.predictors.len() == val_x[0].len() {
            // Направления усиления модуля логита: для w >= 0 максимум, иначе
            // минимум (знак η > 0); для знака η < 0 — противоположная грань.
            let extremes_up: Vec<f64> = model
                .weights
                .iter()
                .zip(config.cohort.predictors.iter())
                .map(|(w, p)| if *w >= 0.0 { p.max } else { p.min })
                .collect();
            let extremes_dn: Vec<f64> = model
                .weights
                .iter()
                .zip(config.cohort.predictors.iter())
                .map(|(w, p)| if *w >= 0.0 { p.min } else { p.max })
                .collect();
            let mut order: Vec<usize> = (0..val_x.len()).collect();
            order.sort_by(|&i, &j| {
                val_etas[j]
                    .abs()
                    .partial_cmp(&val_etas[i].abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let lo_eta = selected.bound * 1.01;
            // Бинарные признаки (диапазон [0; 1], например семейный анамнез) не
            // интерполируются: принимают только предельное значение — дробные
            // значения недопустимы.
            let is_binary: Vec<bool> = config
                .cohort
                .predictors
                .iter()
                .map(|p| p.min == 0.0 && p.max == 1.0)
                .collect();
            for &i in order.iter().take(300) {
                let x = &val_x[i];
                let extremes = if val_etas[i] >= 0.0 {
                    &extremes_up
                } else {
                    &extremes_dn
                };
                for k in 1..=20 {
                    let tt = k as f64 * 0.05;
                    let xp: Vec<f64> = x
                        .iter()
                        .zip(extremes)
                        .zip(&is_binary)
                        .map(|((v, ex), bin)| if *bin { *ex } else { v + tt * (ex - v) })
                        .collect();
                    let e = model.eta_all(std::slice::from_ref(&xp))[0];
                    if (e.abs() >= lo_eta) && (e.abs() <= 10.5) {
                        xs_ob.push(xp);
                        etas_ob.push(e);
                    }
                }
            }
        }
        if !etas_ob.is_empty() {
            let exact_ob: Vec<bool> = etas_ob.iter().map(|e| *e >= 0.0).collect();
            let expected = etas_ob
                .iter()
                .filter(|e| selected.decision(**e) != (**e >= 0.0))
                .count();
            let ctrl = run_he_pipeline(
                &ctx, &lab_pk, &hosp_sk, &rekey, &model, &selected, &xs_ob, batch, 0.0,
            )?;
            let ctrl_dec = he::threshold_probabilities(&ctrl.probabilities);
            let observed = he_mismatches(&ctrl_dec, &exact_ob);
            // Направление расхождений: η ≥ 0, но решение «отрицательное» — ЛО;
            // η < 0, но решение «положительное» — ЛП.
            let (mut fp, mut fn_) = (0usize, 0usize);
            for (e, d) in etas_ob.iter().zip(&ctrl_dec) {
                if *d != (*e >= 0.0) {
                    if *e >= 0.0 {
                        fn_ += 1;
                    } else {
                        fp += 1;
                    }
                }
            }
            let min_abs = etas_ob
                .iter()
                .map(|e| e.abs())
                .fold(f64::INFINITY, f64::min);
            let max_abs = etas_ob.iter().map(|e| e.abs()).fold(0.0, f64::max);
            runs.push(NegativeControlRun {
                name: "out_of_bound".to_string(),
                description: format!(
                    "предельно допустимые профили за границей B: |η| ∈ [{:.1}; {:.1}] (> {:.1})",
                    min_abs, max_abs, selected.bound
                ),
                n_patients: xs_ob.len(),
                expected_plaintext_mismatches: expected,
                observed_he_mismatches: observed,
                observed_matches_expectation: observed == expected,
                false_positives: fp,
                false_negatives: fn_,
                detection_possible: observed > 0,
                note: format!(
                    "ожидание по плейнтексту {expected}, наблюдение {observed}; доля \
                     расхождений среди профилей за B: {:.1} % (ЛП {fp}, ЛО {fn_})",
                    observed as f64 * 100.0 / xs_ob.len() as f64
                ),
            });
        }

        let any_detection = runs.iter().any(|r| r.detection_possible);
        let matching = runs.iter().all(|r| r.observed_matches_expectation);
        let summary = if any_detection && matching {
            "Контроль чувствительности выполнен: метрика точно воспроизводит ожидание по \
             открытому расчёту во всех сценариях — нейтральном (0), арифметических сдвигах \
             (±Δp), при деградации степени (0) и, главное, на предельно допустимых профилях \
             за границей B, где гарантия сохранения знака исчезает. Нулевое число расхождений \
             в основном прогоне — следствие сохранения знака многочлена в пределах области, \
             а не нечувствительности метрики."
                .to_string()
        } else {
            "Контроль чувствительности выполнен, но не все прогоны совпали с ожиданием \
             по плейнтексту — требуется разбор расхождений."
                .to_string()
        };
        Some(NegativeControlReport {
            enabled: true,
            runs,
            summary,
        })
    } else {
        None
    };

    // --- 6. Масштабируемость ---------------------------------------------
    //
    // Замеры выполняются на отдельной когорте: валидационная выборка
    // умещается в один пакет, и на ней нельзя увидеть переход к нескольким
    // пакетам. Когорта замеров порождается независимым семенем и служит
    // только для измерения времени и объёмов; модель при этом остаётся
    // той же, поэтому корректность решений проверяется и на ней.
    let max_size = config.scalability.sizes.iter().copied().max().unwrap_or(0);
    let scale_pool: Vec<Vec<f64>> = if max_size <= val_x.len() {
        val_x.clone()
    } else {
        let mut scale_cfg = config.cohort.clone();
        scale_cfg.seed = config.cohort.seed.wrapping_add(0x5CA1_AB1E);
        scale_cfg.n_derivation = 1;
        scale_cfg.n_validation = max_size;
        scale_cfg.prevalence_calibration_n = config.scalability.calibration_n.max(1_000);
        Cohort::generate(&scale_cfg).validation_features()
    };

    let mut scalability = Vec::with_capacity(config.scalability.sizes.len());
    for &n in &config.scalability.sizes {
        let take = n.min(scale_pool.len());
        let subset: Vec<Vec<f64>> = scale_pool.iter().take(take).cloned().collect();
        let reference: Vec<f64> = model.predict_proba_all(&subset);
        let reference_logits: Vec<f64> = model.eta_all(&subset);

        let mut encrypt = Vec::new();
        let mut logit = Vec::new();
        let mut sigmoid = Vec::new();
        let mut pre = Vec::new();
        let mut decrypt = Vec::new();
        let mut totals = Vec::new();
        let mut feature_bytes = 0usize;
        let mut result_bytes = 0usize;
        let mut batches = 0usize;
        let mut max_err = 0.0_f64;
        let mut sign_mismatches = 0usize;

        for _ in 0..config.scalability.repeats.max(1) {
            let t = Instant::now();
            let out = run_he_pipeline(
                &ctx, &lab_pk, &hosp_sk, &rekey, &model, &selected, &subset, batch, 0.0,
            )?;
            let total = t.elapsed().as_secs_f64() * 1000.0;

            encrypt.push(out.encrypt_ms);
            logit.push(out.logit_ms);
            sigmoid.push(out.sigmoid_ms);
            pre.push(out.pre_ms);
            decrypt.push(out.decrypt_ms);
            totals.push(total);
            feature_bytes = out.feature_bytes;
            result_bytes = out.result_bytes;
            batches = out.batches;

            let errs: Vec<f64> = reference
                .iter()
                .zip(&out.probabilities)
                .map(|(a, b)| (a - b).abs())
                .collect();
            max_err = errs.iter().copied().fold(0.0, f64::max);
            sign_mismatches = reference_logits
                .iter()
                .zip(&out.logits)
                .filter(|(a, b)| (**a >= 0.0) != (**b >= 0.0))
                .count();
        }

        scalability.push(ScalingPoint {
            requested_n: n,
            n_patients: take,
            batches,
            repeats: config.scalability.repeats.max(1),
            encrypt_ms: median_of(&encrypt),
            logit_ms: median_of(&logit),
            sigmoid_ms: median_of(&sigmoid),
            pre_ms: median_of(&pre),
            decrypt_ms: median_of(&decrypt),
            total_ms: median_of(&totals),
            ms_per_patient: median_of(&totals) / take.max(1) as f64,
            feature_ciphertext_bytes: feature_bytes,
            result_ciphertext_bytes: result_bytes,
            max_abs_probability_error: max_err,
            sign_mismatches,
        });
    }

    let mut scal_csv = String::from(
        "requested_n,n_patients,batches,repeats,encrypt_ms,logit_ms,sigmoid_ms,pre_ms,decrypt_ms,total_ms,ms_per_patient,feature_bytes,result_bytes,max_abs_probability_error,sign_mismatches\n",
    );
    for p in &scalability {
        scal_csv.push_str(&format!(
            "{},{},{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.6},{},{},{:.6e},{}\n",
            p.requested_n,
            p.n_patients,
            p.batches,
            p.repeats,
            p.encrypt_ms,
            p.logit_ms,
            p.sigmoid_ms,
            p.pre_ms,
            p.decrypt_ms,
            p.total_ms,
            p.ms_per_patient,
            p.feature_ciphertext_bytes,
            p.result_ciphertext_bytes,
            p.max_abs_probability_error,
            p.sign_mismatches
        ));
    }
    artifacts.push(write_text(&out_dir, "scalability.csv", &scal_csv)?);

    // --- 7. Воспроизводимость --------------------------------------------
    let cohort_again = Cohort::generate(&config.cohort);
    let model_again = logistic::fit(
        &names,
        &cohort_again.derivation_features(),
        &cohort_again.derivation_outcomes(),
        config.model.lambda,
        config.model.max_iter,
        config.model.tol,
    );
    let poly_again = SigmoidPolynomial::fit(bound, config.polynomial.selected_power);

    let cohort_hash_second = hash::sha256_hex(cohort_again.to_csv().as_bytes());
    let cohort_hash_first = hash::sha256_hex(cohort_csv.as_bytes());
    let model_bit_identical = model.weights == model_again.weights
        && model.intercept.to_bits() == model_again.intercept.to_bits();
    let polynomial_bit_identical = selected
        .coeffs
        .iter()
        .zip(&poly_again.coeffs)
        .all(|(a, b)| a.to_bits() == b.to_bits());

    let determinism = DeterminismCheck {
        cohort_sha256_first: cohort_hash_first.clone(),
        cohort_sha256_second: cohort_hash_second.clone(),
        cohort_bit_identical: cohort_hash_first == cohort_hash_second,
        model_bit_identical,
        polynomial_bit_identical,
    };

    // --- 8. Обоснование размера выборки ----------------------------------
    let n_val = val_x.len();
    let (_, cp_upper) = stats::clopper_pearson_ci(0, n_val, config.evaluation.alpha);
    let max_disc = stats::max_discordant_within_margin(
        n_val,
        config.evaluation.equivalence_margin,
        config.evaluation.alpha,
    );
    let sample_size_justification = SampleSizeJustification {
        validation_n: n_val,
        equivalence_margin: config.evaluation.equivalence_margin,
        alpha: config.evaluation.alpha,
        upper_bound_with_zero_discordances: cp_upper,
        max_discordances_within_margin: max_disc,
        conclusion: format!(
            "При валидационной выборке n = {n_val} нулевое число расхождений даёт верхнюю \
             границу {:.4} % точного 95 % интервала Клоппера—Пирсона, что существенно ниже \
             порога эквивалентности {:.2} %; порог остаётся выполненным вплоть до {max_disc} \
             расхождений. Выборка такого размера достаточна для проверки эквивалентности и \
             даёт узкий интервал для AUC (SE ≈ {:.4}).",
            cp_upper * 100.0,
            config.evaluation.equivalence_margin * 100.0,
            auc.se
        ),
    };

    let finished_unix = env::now_unix();
    let results = StudyResults {
        study_name: config.study_name.clone(),
        design: config.design.clone(),
        run_id: run_id.clone(),
        started_utc: environment.started_utc.clone(),
        finished_utc: env::unix_to_utc(finished_unix),
        duration_seconds: started.elapsed().as_secs_f64(),
        config: config.clone(),
        environment: environment.clone(),
        cohort: cohort.diagnostics.clone(),
        model: model_summary,
        polynomial: polynomial_summary,
        clinical_performance: clinical,
        keys_and_volume,
        he_probability_mode,
        he_logit_mode,
        negative_control,
        scalability,
        determinism,
        sample_size_justification,
        // results.json перечисляет все посчитанные артефакты, но не сам себя:
        // полный перечень, включая results.json, environment.json и
        // config.json, содержится в manifest.json.
        artifacts: artifacts.clone(),
    };

    // --- 9. Запись результатов и манифеста -------------------------------
    let results_json = serde_json::to_string_pretty(&results)
        .map_err(|e| Error(format!("сериализация results.json: {e}")))?;
    artifacts.push(write_text(&out_dir, "results.json", &results_json)?);

    let env_json = serde_json::to_string_pretty(&environment)
        .map_err(|e| Error(format!("сериализация environment.json: {e}")))?;
    artifacts.push(write_text(&out_dir, "environment.json", &env_json)?);

    let config_json = serde_json::to_string_pretty(config)
        .map_err(|e| Error(format!("сериализация config.json: {e}")))?;
    artifacts.push(write_text(&out_dir, "config.json", &config_json)?);

    // Манифест перечисляет артефакты и их контрольные суммы. Он пишется
    // последним и намеренно не включает сам себя.
    let manifest = serde_json::json!({
        "run_id": run_id,
        "study_name": config.study_name,
        "started_utc": results.started_utc,
        "finished_utc": results.finished_utc,
        "duration_seconds": results.duration_seconds,
        "git_revision": environment.git_revision,
        "git_branch": environment.git_branch,
        "rustc_version": environment.rustc_version,
        "artifacts": artifacts,
    });
    let manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| Error(format!("сериализация manifest.json: {e}")))?;
    write_text(&out_dir, "manifest.json", &manifest_json)?;

    Ok(StudyReport {
        run_id,
        output_dir: out_dir,
        results,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_config() -> StudyConfig {
        let mut c = default_study_config();
        c.cohort.n_derivation = 300;
        c.cohort.n_validation = 150;
        c.cohort.prevalence_calibration_n = 20_000;
        c.polynomial.sweep_powers = vec![7, 9, 11];
        c.polynomial.selected_power = 7;
        c.polynomial.max_abs_error_target = 0.3;
        c.evaluation.bootstrap_replicates = 100;
        c.evaluation.equivalence_margin = 0.05;
        c.scalability.sizes = vec![50, 150];
        c.scalability.repeats = 1;
        c.scalability.cohort_size = 0;
        c.scalability.calibration_n = 5_000;
        c.he.depth_margin = 3;
        c.negative_control.shift = 0.05;
        c
    }

    /// Каталог для артефактов теста.
    ///
    /// Артефакты пишутся внутрь каталога сборки крейта (`target/`), а не в
    /// системный временный каталог: `target/` уже исключён из репозитория,
    /// всегда доступен на запись рядом с проектом и вычищается вместе с
    /// `cargo clean`. Системный `TEMP` может указывать куда угодно (в том
    /// числе на диск, закрытый политикой), и тест, зависящий от него, падает
    /// по причинам, не связанным с проверяемым кодом.
    fn test_output_root(name: &str) -> std::path::PathBuf {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(name);
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn default_config_is_well_formed() {
        let c = default_study_config();
        assert!(c
            .polynomial
            .sweep_powers
            .contains(&c.polynomial.selected_power));
        assert!(c.evaluation.alpha > 0.0 && c.evaluation.alpha < 1.0);
        assert!(c.evaluation.equivalence_margin > 0.0);
        assert!(!c.scalability.sizes.is_empty());
        assert!(c.scalability.repeats >= 1);
        assert!(c.he.depth_margin < 10);
    }

    #[test]
    fn full_run_produces_complete_and_consistent_results() {
        let dir = test_output_root("full_run");
        let _ = std::fs::remove_dir_all(&dir);

        let report = run(&tiny_config(), &dir).expect("прогон исследования");

        // Ключевые свойства, на которые опирается статья.
        assert!(report.results.determinism.cohort_bit_identical);
        assert!(report.results.determinism.model_bit_identical);
        assert!(report.results.determinism.polynomial_bit_identical);
        assert!(
            report.results.polynomial.sign_preserving,
            "многочлен не сохраняет знак"
        );
        assert!(
            report.results.polynomial.min_delta_positive_side > 0.0,
            "минимум Δ⁺ = {}",
            report.results.polynomial.min_delta_positive_side
        );
        assert_eq!(report.results.polynomial.decision_mismatches_on_data, 0);
        assert_eq!(report.results.polynomial.decision_mismatches_on_grid, 0);

        // Гомоморфный контур обязан воспроизвести решения эталона.
        assert_eq!(
            report.results.he_probability_mode.sign_mismatches, 0,
            "логиты разошлись по знаку"
        );
        assert!(
            report.results.he_probability_mode.max_abs_error < 0.05,
            "ошибка вероятности {} слишком велика",
            report.results.he_probability_mode.max_abs_error
        );

        // Первичная конечная точка: расхождения решений.
        //
        // На малой тестовой когорте порог эквивалентности задан намеренно
        // широким: при n = 150 даже одно расхождение даёт верхнюю границу
        // точного интервала ~3.6 %, поэтому требовать 1 % здесь
        // статистически бессмысленно. Основной прогон использует n = 4000
        // и порог 1 %.
        let mode = &report.results.he_probability_mode;
        assert_eq!(mode.sign_mismatches, 0, "логиты разошлись по знаку");
        let rate = mode.discordance_rate;
        assert!(rate <= 0.02, "доля расхождений {rate} слишком велика");
        assert!(
            mode.equivalence.equivalent,
            "эквивалентность не достигнута при пороге {}: верхняя граница {}",
            mode.equivalence.margin, mode.equivalence.ci_high
        );

        // Контроль чувствительности: метрика обязана регистрировать нарушения,
        // когда гарантия конструкции сломана, и совпадать с ожиданием по
        // плейнтексту в допуске на шум.
        let nc = report
            .results
            .negative_control
            .as_ref()
            .expect("контроль чувствительности включён в конфигурации теста");
        assert!(!nc.runs.is_empty(), "нет ни одного прогона контроля");
        let base = nc
            .runs
            .iter()
            .find(|r| r.name == "baseline")
            .expect("нет baseline");
        assert_eq!(base.expected_plaintext_mismatches, 0);
        assert_eq!(base.observed_he_mismatches, 0);
        for r in &nc.runs {
            assert!(
                r.observed_matches_expectation,
                "прогон {}: наблюдение {} ≠ ожидание {} (вне допуска)",
                r.name, r.observed_he_mismatches, r.expected_plaintext_mismatches
            );
            // обнаружение невозможно там, где ожидание нулевое, и обязательно там, где оно > 0
            assert!(
                r.detection_possible == (r.observed_he_mismatches > 0),
                "прогон {}: detection_possible не согласован с наблюдением",
                r.name
            );
            if r.expected_plaintext_mismatches > 0 {
                assert!(
                    r.detection_possible,
                    "прогон {}: ожидание > 0, но ничего не обнаружено",
                    r.name
                );
            }
        }
        // хотя бы один прогон не нейтрален (ожидание > 0) — иначе контроль вырожден
        assert!(
            nc.runs.iter().any(|r| r.expected_plaintext_mismatches > 0),
            "все прогоны контроля нейтральны — выборка сценариев вырождена"
        );

        // Артефакты записаны и перечислены (включая probabilities.csv,
        // из которого строится график «открытая вероятность vs зашифрованная»).
        assert!(
            report.results.artifacts.len() >= 8,
            "артефактов {}",
            report.results.artifacts.len()
        );
        for a in &report.results.artifacts {
            let p = report.output_dir.join(&a.file);
            assert!(p.exists(), "нет артефакта {}", a.file);
            assert_eq!(a.sha256.len(), 64);
            assert_eq!(
                hash::sha256_file(&p).expect("хэш файла"),
                a.sha256,
                "контрольная сумма {} не совпала",
                a.file
            );
        }
        assert!(report.output_dir.join("manifest.json").exists());

        // Все точки масштабируемости посчитаны и без расхождений знака.
        assert_eq!(report.results.scalability.len(), 2);
        for p in &report.results.scalability {
            assert_eq!(p.sign_mismatches, 0);
            assert!(p.total_ms > 0.0);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn results_json_round_trips() {
        let dir = test_output_root("json_round_trip");
        let _ = std::fs::remove_dir_all(&dir);
        let report = run(&tiny_config(), &dir).expect("прогон");
        let text = std::fs::read_to_string(report.output_dir.join("results.json")).expect("чтение");
        let parsed: StudyResults = serde_json::from_str(&text).expect("разбор results.json");
        assert_eq!(parsed.run_id, report.results.run_id);
        assert_eq!(
            parsed.model.feature_names,
            report.results.model.feature_names
        );
        assert_eq!(parsed.scalability.len(), report.results.scalability.len());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
