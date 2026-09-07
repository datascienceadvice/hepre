//! Утилита роли «Сервер»: гомоморфно считает скор по зашифрованным фичам
//! и перешифровывает результат под ключ больницы (PRE). Не имеет ни данных,
//! ни ключа расшифровки.
//!
//!   server score --model '<json>' [--dir handoff]
//!       читает handoff/features.cts.bin + handoff/rekey.ek.bin
//!       -> handoff/scores.ct.bin

#[path = "../demo_common.rs"]
mod common;

use std::path::PathBuf;
use std::time::Instant;

use common::{blobs_to_ciphertexts, err_exit, make_context, parse_model, read_handoff, resolve_dir, write_handoff, KIND_FEATURES, KIND_SCORES};
use hepre::scoring::{compute_score, transfer_score, EncodedFeatures, ScoringModel};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("Использование:");
        eprintln!("  server score --model '<json>' [--dir DIR]");
        std::process::exit(2);
    }
    if args[0] == "help" || args[0] == "--help" || args[0] == "-h" {
        eprintln!("Использование:");
        eprintln!("  server score --model '<json>' [--dir DIR]");
        std::process::exit(0);
    }
    if args[0] != "score" {
        err_exit(format!("server: неизвестная подкоманда «{}» (доступна: score)", args[0]));
    }

    let rest = &args[1..];
    let model_str = arg_after(rest, "--model").unwrap_or_else(|| err_exit("server score: требуется --model '<json>'"));
    let dir = arg_after(rest, "--dir").map(|s| resolve_dir(&s)).unwrap_or_else(|| PathBuf::from("handoff"));
    let model = parse_model(&model_str).unwrap_or_else(|e| err_exit(e));

    let cc = make_context();
    println!("--- Сервер: гомоморфный расчёт скора и PRE ---");

    let feats_path = dir.join("features.cts.bin");
    let rekey_path = dir.join("rekey.ek.bin");
    let (kind, n, f, feats_blobs) = read_handoff(&feats_path).unwrap_or_else(|e| err_exit(e));
    if kind != KIND_FEATURES {
        err_exit(format!("{}: не features-контейнер (kind={kind})", feats_path.display()));
    }
    if f as usize != model.weights.len() {
        err_exit(format!(
            "{}: признаков в контейнере ({f}) != длины модели ({})",
            feats_path.display(),
            model.weights.len()
        ));
    }

    let rekey_bytes = std::fs::read(&rekey_path).unwrap_or_else(|e| err_exit(format!("чтение {}: {e}", rekey_path.display())));
    let rekey = cc.deserialize_eval_key(&rekey_bytes).unwrap_or_else(|e| err_exit(format!("deserialize_eval_key: {e}")));

    // Шифротексты в контейнере идут пакетами по f штук (BATCH пациентов на пакет).
    let chunks = (n as usize + common::BATCH - 1) / common::BATCH;
    let expected_cts = chunks * f as usize;
    if feats_blobs.len() != expected_cts {
        err_exit(format!(
            "{}: ожидал {expected_cts} шифротекстов, получил {}",
            feats_path.display(),
            feats_blobs.len()
        ));
    }

    let sm = ScoringModel {
        weights: model.weights.clone(),
        intercept: model.intercept,
        threshold: model.threshold,
    };

    let t = Instant::now();
    let mut score_blobs: Vec<Vec<u8>> = Vec::with_capacity(chunks);
    for c in 0..chunks {
        let n_in_chunk = ((c + 1) * common::BATCH).min(n as usize) - c * common::BATCH;
        let feat_slice: Vec<Vec<u8>> = feats_blobs[c * f as usize..(c + 1) * f as usize].to_vec();
        let cts = blobs_to_ciphertexts(&cc, &feat_slice).unwrap_or_else(|e| err_exit(e));

        // Модель публична: сервер сам собирает plaintext интерцепта.
        let feats = EncodedFeatures {
            cts,
            n: n_in_chunk,
            intercept_pt: Some(cc.make_ckks_plaintext(&vec![model.intercept; n_in_chunk]).unwrap()),
        };

        let score = compute_score(&cc, &feats, &sm).unwrap_or_else(|e| err_exit(format!("compute_score: {e}")));
        let transferred = transfer_score(&cc, &rekey, &score).unwrap_or_else(|e| err_exit(format!("transfer_score: {e}")));
        score_blobs.push(transferred.serialize().unwrap_or_else(|e| err_exit(e)));
    }
    let score_ms = t.elapsed().as_secs_f64() * 1000.0;

    let scores_path = dir.join("scores.ct.bin");
    write_handoff(&scores_path, KIND_SCORES, n as usize, 0, &score_blobs).unwrap_or_else(|e| err_exit(e));

    let feats_bytes: usize = feats_blobs.iter().map(|b| b.len()).sum();
    let scores_bytes: usize = score_blobs.iter().map(|b| b.len()).sum();
    println!("   Пациентов: {n}, пакетов: {chunks} (слотов на пакет: {}).", common::BATCH);
    println!("   score = Σ_j w_j·x_j + b: EvalMultPlain + EvalAdd на шифротекстах; результат -> ReEncrypt (PRE).");
    println!("   Время: {score_ms:.1} ms.");
    println!("   features.cts.bin прочитан : {} ({})", feats_path.display(), common::kb(feats_bytes));
    println!("   rekey.ek.bin    прочитан  : {} ({})", rekey_path.display(), common::kb(rekey_bytes.len()));
    println!("   scores.ct.bin   записан   : {} ({})", scores_path.display(), common::kb(scores_bytes));
    println!("   Сервер не имеет ключей расшифровки и не видел фичи пациентов в открытом виде.");
}

fn arg_after(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}