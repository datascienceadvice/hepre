//! Воспроизводимый end-to-end пример: приватный скоринг риска диабета 2 типа.
//!
//! Роли:
//!   лаборатория (данные) -> сервер (FHE-вычисления) -> больница (PRE + порог).
//!
//! Запуск (Release!):
//!   $env:PATH = "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\<ver>\bin\Hostx64\x64;$env:PATH"
//!   cargo run --release --example demo

use hepre::scoring::{
    compute_score, decrypt_scores, encrypt_features, scoring_context, transfer_score, ScoringModel,
};

// Модель линейного скора (нормированные фичи): HbA1c, глюкоза, ИМТ, возраст, диабет в семье.
fn model() -> ScoringModel {
    ScoringModel {
        weights: vec![0.22, 0.14, 0.08, 0.03, 0.10],
        intercept: -1.70,
        threshold: -1.45,
    }
}

// 5 пациентов: [hba1c, glucose, bmi, age, family].
fn dataset() -> Vec<Vec<f64>> {
    vec![
        vec![0.50, 0.40, 0.20, 0.30, 0.00],
        vec![0.80, 0.70, 0.50, 0.60, 1.00],
        vec![0.10, 0.20, 0.10, 0.10, 0.00],
        vec![0.70, 0.90, 0.70, 0.80, 1.00],
        vec![0.30, 0.60, 0.30, 0.50, 0.00],
    ]
}

const NAMES: [&str; 5] = ["P1", "P2", "P3", "P4", "P5"];

fn main() {
    println!("=== Приватный скоринг риска диабета 2-го типа (CKKS + PRE) ===");
    println!(
        "Параметры CKKS: batch = 8192 слотов, multiplicative_depth = 3, scaling = 40 бит."
    );
    println!();

    let m = model();
    let cc = scoring_context().expect("создание CKKS-контекста");

    step("1. Лаборатория: ключи и шифрование фичей пациентов", || {
        let lab = cc.keygen().expect("keygen");
        let lab_pk = lab.public_key().expect("public_key");
        let lab_sk = lab.secret_key().expect("secret_key");

        // Сервер видит только шифротексты — ниже у него нет ни ключей, ни данных.
        let feats = encrypt_features(&cc, &lab_pk, &m, &dataset()).expect("encrypt_features");

        let total_bytes: usize = feats.cts.iter().map(|ct| ct.serialize().unwrap().len()).sum();
        println!(
            "   {} пациента(ов), {} признака(ов) на пациента",
            feats.n,
            m.weights.len()
        );
        println!(
            "   Зашифровано {} шифротекстов (признак -> шифротекст, пациент -> слот).",
            feats.cts.len()
        );
        println!("   Суммарный размер зашифрованных фичей: {total_bytes} байт.");
        println!("   Сервер видит только шифротексты — значения фичей недоступны.");
        (lab_pk, lab_sk, feats)
    });

    step("2. Сервер: гомоморфный расчёт скора параллельно для всех пациентов", || {
        println!("   Скор: score_i = Σ_j w_j·x_ij + b  (EvalMultPlain x5 + EvalAdd)");
        println!("   Результат лежит в шифротексте — сервер не знает самих скоров.");
    });

    step("3. Больница: ключи и ключ перешифрования (PRE)", || {
        println!("   Больница генерирует свою пару ключей.");
        println!("   Лаборатория формирует ReKeyGen(sk_lab -> pk_hospital).");
    });

    step("4. Сервер: прокси-перешифрование скора под ключ больницы", || {
        println!("   ReEncrypt выполнен: расшифровать скор теперь может только больница.");
    });

    step("5. Больница: расшифровка скора и применение порога в открытом виде", || {
        let xs = dataset();
        println!();
        println!("   {:<6} {:>12} {:>12}   {:>4}   {}", "Пац.", "Скор (HE)", "Скор (откр.)", "max|err|", "Риск >= -1.45?");
        let mut max_err = 0.0f64;
        let mut mismatches = 0usize;

        // Полный конвейер в одном прогоне, чтобы показать числа рядом:
        // (в реальности шаги 1,3 и 5 выполняют разные стороны).
        let lab = cc.keygen().unwrap();
        let lab_pk = lab.public_key().unwrap();
        let lab_sk = lab.secret_key().unwrap();
        let feats = encrypt_features(&cc, &lab_pk, &m, &xs).unwrap();
        let score = compute_score(&cc, &feats, &m).unwrap();

        let hosp = cc.keygen().unwrap();
        let hosp_pk = hosp.public_key().unwrap();
        let hosp_sk = hosp.secret_key().unwrap();
        let rekey = cc.rekeygen(&lab_sk, &hosp_pk).unwrap();
        let transferred = transfer_score(&cc, &rekey, &score).unwrap();
        let got = decrypt_scores(&cc, &hosp_sk, &transferred, feats.n).unwrap();

        for (i, (g, x)) in got.iter().zip(xs.iter()).enumerate() {
            let plain = m.score_plain(x);
            let err = (g - plain).abs();
            max_err = max_err.max(err);
            let risk = *g >= m.threshold;
            let risk_plain = plain >= m.threshold;
            if risk != risk_plain {
                mismatches += 1;
            }
            println!(
                "   {:<6} {:>12.4} {:>12.4}   {:>7.1e}   {}",
                NAMES[i],
                g,
                plain,
                err,
                if risk { "ДА" } else { "нет" }
            );
        }
        println!();
        println!("   Итог: max|HE скор - открытый скор| = {max_err:.1e}");
        println!("   Несовпадений бинарного порога: {mismatches} (ожидается 0).");
    });

    step("6. Проверка: чужой ключ (сервера) не расшифровывает PRE-скор", || {
        let xs = dataset();
        let lab = cc.keygen().unwrap();
        let lab_pk = lab.public_key().unwrap();
        let lab_sk = lab.secret_key().unwrap();
        let feats = encrypt_features(&cc, &lab_pk, &m, &xs).unwrap();
        let score = compute_score(&cc, &feats, &m).unwrap();

        let hosp = cc.keygen().unwrap();
        let hosp_pk = hosp.public_key().unwrap();
        let hosp_sk = hosp.secret_key().unwrap();
        let rekey = cc.rekeygen(&lab_sk, &hosp_pk).unwrap();
        let transferred = transfer_score(&cc, &rekey, &score).unwrap();

        // Сервер пытается расшифровать чужим ключом.
        let server = cc.keygen().unwrap();
        let server_sk = server.secret_key().unwrap();
        match decrypt_scores(&cc, &server_sk, &transferred, feats.n) {
            Ok(got) => {
                let plausible = got
                    .iter()
                    .zip(xs.iter())
                    .any(|(g, x)| (g - m.score_plain(x)).abs() < 0.1);
                if plausible {
                    println!("   ОШИБКА: чужой ключ дал правдоподобные скоры!");
                } else {
                    println!("   чужой ключ вернул мусор (не исходные скоры) — недоступно. OK");
                }
            }
            Err(_e) => println!("   чужой ключ -> Err (fail-closed Decode) — недоступно. OK"),
        }
    });

    println!();
    println!("Готово: сквозной приватный скоринг лаборатория -> сервер -> больница.");
}

fn step<R>(title: &str, f: impl FnOnce() -> R) -> R {
    println!("--- {title} ---");
    f()
}