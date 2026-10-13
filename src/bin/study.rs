//! CLI раннера исследования.
//!
//! ```text
//! study [--config <файл>] [--out <каталог>] [--write-default-config <файл>]
//! ```
//!
//! Без `--config` используется встроенная конфигурация по умолчанию; если
//! существует `configs/study.json`, он подхватывается автоматически.
//! Все результаты складываются в `results/<run-id>/` вместе с манифестом
//! контрольных сумм.
//!
//! Запускать следует в release-профиле: замеры времени в debug-сборке
//! бессмысленны.

use std::path::{Path, PathBuf};

use hepre::study::{self, StudyConfig};

const DEFAULT_CONFIG: &str = "configs/study.json";
const DEFAULT_OUT: &str = "results";

fn usage() {
    eprintln!("Использование:");
    eprintln!("  study [--config <файл>] [--out <каталог>] [--write-default-config <файл>]");
    eprintln!();
    eprintln!("  --config <файл>                конфигурация исследования (JSON)");
    eprintln!("  --out <каталог>                корень для результатов (по умолчанию {DEFAULT_OUT})");
    eprintln!("  --write-default-config <файл>  записать конфигурацию по умолчанию и выйти");
    eprintln!("  --help                         эта справка");
}

fn arg_after(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        usage();
        return;
    }

    // Режим генерации файла конфигурации.
    if let Some(path) = arg_after(&args, "--write-default-config") {
        let cfg = study::default_study_config();
        let json = match serde_json::to_string_pretty(&cfg) {
            Ok(j) => j,
            Err(e) => {
                eprintln!("ОШИБКА: сериализация конфигурации: {e}");
                std::process::exit(1);
            }
        };
        let path = PathBuf::from(path);
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    eprintln!("ОШИБКА: создание {}: {e}", parent.display());
                    std::process::exit(1);
                }
            }
        }
        if let Err(e) = std::fs::write(&path, json) {
            eprintln!("ОШИБКА: запись {}: {e}", path.display());
            std::process::exit(1);
        }
        println!("Конфигурация по умолчанию записана: {}", path.display());
        return;
    }

    // Загрузка конфигурации: явный путь приоритетнее файла по умолчанию.
    let config: StudyConfig = match arg_after(&args, "--config") {
        Some(path) => match load_config(Path::new(&path)) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("ОШИБКА: {e}");
                std::process::exit(1);
            }
        },
        None => {
            let default_path = Path::new(DEFAULT_CONFIG);
            if default_path.is_file() {
                println!("Конфигурация: {}", default_path.display());
                match load_config(default_path) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("ОШИБКА: {e}");
                        std::process::exit(1);
                    }
                }
            } else {
                println!("Конфигурация: встроенная по умолчанию ({} не найден)", DEFAULT_CONFIG);
                study::default_study_config()
            }
        }
    };

    let out_root = arg_after(&args, "--out")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_OUT));

    println!("=== {} ===", config.study_name);
    println!(
        "Когорта: обучающая {} / валидационная {} / признаков {}",
        config.cohort.n_derivation,
        config.cohort.n_validation,
        config.cohort.n_features()
    );
    println!(
        "Многочлен: степень {} (граница B подбирается по данным), глубина контекста {}",
        config.polynomial.selected_power,
        config.he.depth_margin
    );
    println!("Первичная конечная точка: доля расхождений, порог {}", config.evaluation.equivalence_margin);
    println!();

    let report = match study::run(&config, &out_root) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("ОШИБКА выполнения исследования: {e}");
            std::process::exit(1);
        }
    };

    let r = &report.results;

    println!("--- Когорта ---");
    println!(
        "   Распространённость: обучающая {:.4}, валидационная {:.4} (цель {:.4})",
        r.cohort.realized_prevalence_derivation,
        r.cohort.realized_prevalence_validation,
        r.cohort.target_prevalence
    );
    println!(
        "   Логит: среднее {:.3}, СКО {:.3}, диапазон [{:.3}, {:.3}]",
        r.cohort.eta_mean_all, r.cohort.eta_sd_all, r.cohort.eta_min_all, r.cohort.eta_max_all
    );

    println!("--- Модель (открытый эталон) ---");
    println!(
        "   Обучение: {} наблюдений, {} событий, {} шагов, сходимость: {}",
        r.model.n_train, r.model.n_events_train, r.model.iterations, r.model.converged
    );
    println!("   Событий на переменную (EPV): {:.1}", r.model.events_per_variable);
    println!(
        "   AUC = {:.4} (95 % ДИ {:.4}–{:.4}), Brier = {:.4}",
        r.clinical_performance.auc.auc,
        r.clinical_performance.auc.ci_low,
        r.clinical_performance.auc.ci_high,
        r.clinical_performance.brier
    );
    println!(
        "   Калибровка: свободный член {:.4}, наклон {:.4}",
        r.clinical_performance.calibration_intercept, r.clinical_performance.calibration_slope
    );
    let d = &r.clinical_performance.at_default_threshold;
    println!(
        "   При пороге 0.5: чувствительность {:.3} ({:.3}–{:.3}), специфичность {:.3} ({:.3}–{:.3}), PPV {:.3}, NPV {:.3}",
        d.sensitivity, d.sensitivity_ci.0, d.sensitivity_ci.1,
        d.specificity, d.specificity_ci.0, d.specificity_ci.1,
        d.ppv, d.npv
    );

    println!("--- Аппроксимация сигмоиды ---");
    println!(
        "   Граница B = {:.3} (наблюдаемый max|η| = {:.3}), степень {}, глубина {} (контекст {})",
        r.polynomial.bound,
        r.polynomial.max_abs_eta_observed,
        r.polynomial.selected_power,
        r.polynomial.selected_mult_depth,
        r.polynomial.context_depth_used
    );
    println!("   Кольцо N = {}, слотов в пакете: {}", r.polynomial.ring_dimension, r.polynomial.slots_per_batch);
    println!(
        "   Неотрицательные коэффициенты: {}, сохранение знака решения: {}, монотонность: {}",
        r.polynomial.all_coefficients_nonnegative,
        r.polynomial.sign_preserving,
        r.polynomial.monotone
    );
    println!("   Правило выбора степени: {}", r.polynomial.selection_rule);
    println!(
        "   min(σ̂(η) − 1/2) при η > 0 = {:.3e} (должно быть > 0)",
        r.polynomial.min_delta_positive_side
    );
    println!(
        "   Макс. ошибка вероятности: {:.3e} на [-B,B], {:.3e} на данных",
        r.polynomial.max_abs_error, r.polynomial.max_abs_error_on_data
    );
    println!(
        "   Расхождения решения с η ≥ 0: {} на данных, {} на сетке",
        r.polynomial.decision_mismatches_on_data, r.polynomial.decision_mismatches_on_grid
    );

    println!("--- Гомоморфный контур ---");
    println!(
        "   Ключи: pk {:.1} КБ, sk {:.1} КБ, rekey {:.1} КБ (ReKeyGen {:.1} ms)",
        r.keys_and_volume.public_key_bytes as f64 / 1024.0,
        r.keys_and_volume.secret_key_bytes as f64 / 1024.0,
        r.keys_and_volume.proxy_rekey_bytes as f64 / 1024.0,
        r.keys_and_volume.rekeygen_ms
    );
    println!(
        "   Объём передачи: фичи {:.1} КБ ({:.1} КБ/пациент), результат {:.1} КБ",
        r.keys_and_volume.feature_batch_bytes as f64 / 1024.0,
        r.keys_and_volume.bytes_per_patient_features / 1024.0,
        r.keys_and_volume.result_bytes as f64 / 1024.0
    );
    for mode in [&r.he_probability_mode, &r.he_logit_mode] {
        println!("   [{}]", mode.mode);
        println!(
            "     max|Δp| = {:.3e}, RMSE = {:.3e}, среднее |Δp| = {:.3e} (ДИ {:.3e}–{:.3e})",
            mode.max_abs_error, mode.rmse, mode.mean_abs_error,
            mode.mean_abs_error_ci.0, mode.mean_abs_error_ci.1
        );
        println!(
            "     расхождений решений: {} из {} ({:.6} %, 95 % ДИ {:.6}–{:.6} %)",
            mode.discordant,
            mode.total,
            mode.discordance_rate * 100.0,
            mode.discordance_ci.0 * 100.0,
            mode.discordance_ci.1 * 100.0
        );
        println!(
            "     эквивалентность (порог {:.2} %): {}",
            mode.equivalence.margin * 100.0,
            if mode.equivalence.equivalent { "достигнута" } else { "НЕ достигнута" }
        );
    }

    println!("--- Масштабируемость (медиана из {} повторов) ---", r.scalability.first().map(|p| p.repeats).unwrap_or(0));
    println!(
        "   {:>7} {:>7} {:>10} {:>10} {:>10} {:>10} {:>10} {:>12}",
        "N", "пакетов", "Encrypt", "Logit", "Sigmoid", "PRE", "Decrypt", "итого"
    );
    for p in &r.scalability {
        println!(
            "   {:>7} {:>7} {:>9.1}ms {:>9.1}ms {:>9.1}ms {:>9.1}ms {:>9.1}ms {:>10.1}ms",
            p.n_patients,
            p.batches,
            p.encrypt_ms,
            p.logit_ms,
            p.sigmoid_ms,
            p.pre_ms,
            p.decrypt_ms,
            p.total_ms
        );
    }

    println!("--- Воспроизводимость ---");
    println!(
        "   Когорта побитово совпала: {}, модель: {}, многочлен: {}",
        r.determinism.cohort_bit_identical,
        r.determinism.model_bit_identical,
        r.determinism.polynomial_bit_identical
    );

    println!("--- Обоснование размера выборки ---");
    println!("   {}", r.sample_size_justification.conclusion);

    println!();
    println!("Результаты: {}", report.output_dir.display());
    println!("Манифест:   {}", report.output_dir.join("manifest.json").display());
    println!("Идентификатор запуска: {}", report.run_id);
}

/// Читает конфигурацию из файла JSON.
fn load_config(path: &Path) -> Result<StudyConfig, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("не удалось прочитать {}: {e}", path.display()))?;
    serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("не удалось разобрать {}: {e}", path.display()))
}
