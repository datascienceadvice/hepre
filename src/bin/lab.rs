//! Утилита роли «Лаборатория»: владеет фичами пациентов, шифрует их и
//! формирует ключ перешифрования для больницы.
//!
//!   lab encrypt --patients '<json>' --model '<json>' --hospital-pk <file> [--dir handoff]
//!       -> handoff/features.cts.bin (зашифрованные фичи для сервера)
//!       -> handoff/rekey.ek.bin    (key-перешифрования lab_sk -> hosp_pk)

#[path = "../demo_common.rs"]
mod common;

use std::path::PathBuf;

use common::{ciphertexts_to_blobs, err_exit, make_context, parse_model, parse_patients, resolve_dir, resolve_json_arg, write_handoff, KIND_FEATURES};
use hepre::scoring::{encrypt_features, ScoringModel};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("Использование:");
        eprintln!("  lab encrypt --patients '<json или путь к файлу>' --model '<json или путь к файлу>' --hospital-pk <file> [--dir DIR]");
        std::process::exit(2);
    }
    if args[0] == "help" || args[0] == "--help" || args[0] == "-h" {
        eprintln!("Использование:");
        eprintln!("  lab encrypt --patients '<json или путь к файлу>' --model '<json или путь к файлу>' --hospital-pk <file> [--dir DIR]");
        std::process::exit(0);
    }
    if args[0] != "encrypt" {
        err_exit(format!("lab: неизвестная подкоманда «{}» (доступна: encrypt)", args[0]));
    }

    let rest = &args[1..];
    let patients_raw = arg_after(rest, "--patients").unwrap_or_else(|| err_exit("lab encrypt: требуется --patients '<json или файл>'"));
    let model_raw = arg_after(rest, "--model").unwrap_or_else(|| err_exit("lab encrypt: требуется --model '<json или файл>'"));
    let hosp_pk = arg_after(rest, "--hospital-pk").unwrap_or_else(|| err_exit("lab encrypt: требуется --hospital-pk <file>"));
    let dir = arg_after(rest, "--dir").map(|s| resolve_dir(&s)).unwrap_or_else(|| PathBuf::from("handoff"));

    let patients_str = resolve_json_arg("--patients", &patients_raw).unwrap_or_else(|e| err_exit(e));
    let model_str = resolve_json_arg("--model", &model_raw).unwrap_or_else(|e| err_exit(e));
    let model = parse_model(&model_str).unwrap_or_else(|e| err_exit(e));
    let patients = parse_patients(&patients_str).unwrap_or_else(|e| err_exit(e));
    let xs: Vec<Vec<f64>> = patients.iter().map(|p| p.features.clone()).collect();

    let cc = make_context();
    println!("--- Лаборатория: шифрование фичей пациентов ---");
    println!("   Пациентов: {}, признаков на пациента: {}", patients.len(), model.weights.len());

    let hosp_pk_path = PathBuf::from(hosp_pk);
    let hosp_pk_bytes = std::fs::read(&hosp_pk_path)
        .unwrap_or_else(|e| err_exit(format!("чтение {}: {e}", hosp_pk_path.display())));
    let hosp_pk = cc.deserialize_public_key(&hosp_pk_bytes).unwrap_or_else(|e| err_exit(e));

    let keys = cc.keygen().unwrap_or_else(|e| err_exit(format!("keygen: {e}")));
    let lab_pk = keys.public_key().unwrap_or_else(|e| err_exit(e));
    let lab_sk = keys.secret_key().unwrap_or_else(|e| err_exit(e));

    // Шифрование фичей пакетами по BATCH пациентов.
    let sm = ScoringModel {
        weights: model.weights.clone(),
        intercept: model.intercept,
        threshold: model.threshold,
    };
    let mut all_blobs: Vec<Vec<u8>> = Vec::new();
    let mut blobs_encrypted = 0usize;
    for chunk in xs.chunks(common::BATCH) {
        let feats = encrypt_features(&cc, &lab_pk, &sm, chunk).unwrap_or_else(|e| err_exit(e));
        let blobs = ciphertexts_to_blobs(&feats.cts.iter().collect::<Vec<_>>()).unwrap_or_else(|e| err_exit(e));
        blobs_encrypted += blobs.len();
        all_blobs.extend(blobs);
    }

    let feats_path = dir.join("features.cts.bin");
    write_handoff(&feats_path, KIND_FEATURES, patients.len(), model.weights.len(), &all_blobs)
        .unwrap_or_else(|e| err_exit(e));

    // PRE: ключ перешифрования lab_sk -> hosp_pk.
    let rekey = cc.rekeygen(&lab_sk, &hosp_pk).unwrap_or_else(|e| err_exit(format!("rekeygen: {e}")));
    let rekey_bytes = rekey.serialize().unwrap_or_else(|e| err_exit(e));
    let rekey_path = dir.join("rekey.ek.bin");
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| err_exit(format!("create_dir_all: {e}")));
    std::fs::write(&rekey_path, &rekey_bytes).unwrap_or_else(|e| err_exit(format!("запись {}: {e}", rekey_path.display())));

    let feats_bytes: usize = all_blobs.iter().map(|b| b.len()).sum();
    println!("   Зашифровано шифротекстов: {blobs_encrypted} (признак -> шифротекст, пациент -> слот; пакеты по {}).", common::BATCH);
    println!("   features.cts.bin : {} ({})", feats_path.display(), common::kb(feats_bytes));
    println!("   rekey.ek.bin     : {} ({})", rekey_path.display(), common::kb(rekey_bytes.len()));
    println!("   Сервер получает ТОЛЬКО эти файлы: без фичей пациентов и без ключей расшифровки.");
}

fn arg_after(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}