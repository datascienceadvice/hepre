//! Утилита роли «Больница»: генерация ключей и расшифровка скоров.
//!
//! Подкоманды:
//!   hospital keygen  [--dir handoff]
//!       -> handoff/hosp.pk.bin, handoff/hosp.sk.bin
//!   hospital decrypt --model '<json>' [--patients '<json>'] [--dir handoff]
//!       читает hosp.sk.bin + scores.ct.bin, печатает скоры и вердикты риска.
//!       --patients задаётся ТОЛЬКО для демонстрационной сверки точности
//!       (в реальном протоколе больница видит фичи пациентов).

#[path = "../demo_common.rs"]
mod common;

use std::path::PathBuf;

use common::{
    err_exit, make_context, parse_model, parse_patients, read_handoff, resolve_dir,
    resolve_json_arg, KIND_SCORES,
};
use hepre::scoring::decrypt_scores;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        print_usage();
        std::process::exit(2);
    }

    match args[0].as_str() {
        "keygen" => cmd_keygen(&args[1..]),
        "decrypt" => cmd_decrypt(&args[1..]),
        "help" | "--help" | "-h" => print_usage(),
        other => err_exit(format!("неизвестная подкоманда «{other}»")),
    }
}

fn print_usage() {
    eprintln!("Использование:");
    eprintln!("  hospital keygen  [--dir DIR]");
    eprintln!("  hospital decrypt --model '<json или путь к файлу>' [--patients '<json или путь к файлу>'] [--dir DIR]");
}

fn dir_flag(args: &[String]) -> PathBuf {
    args.iter()
        .position(|a| a == "--dir")
        .and_then(|i| args.get(i + 1))
        .map(|s| resolve_dir(s))
        .unwrap_or_else(|| PathBuf::from("handoff"))
}

fn cmd_keygen(args: &[String]) {
    let flags_dir = dir_flag(args);
    let cc = make_context();
    println!("--- Больница: генерация ключей ---");

    let keys = cc
        .keygen()
        .unwrap_or_else(|e| err_exit(format!("keygen: {e}")));
    let pk = keys.public_key().unwrap_or_else(|e| err_exit(e));
    let sk = keys.secret_key().unwrap_or_else(|e| err_exit(e));

    let pk_path = flags_dir.join("hosp.pk.bin");
    let sk_path = flags_dir.join("hosp.sk.bin");
    std::fs::create_dir_all(&flags_dir)
        .unwrap_or_else(|e| err_exit(format!("create_dir_all: {e}")));
    let pk_bytes = pk.serialize().unwrap_or_else(|e| err_exit(e));
    let sk_bytes = sk.serialize().unwrap_or_else(|e| err_exit(e));
    std::fs::write(&pk_path, &pk_bytes)
        .unwrap_or_else(|e| err_exit(format!("запись {}: {e}", pk_path.display())));
    std::fs::write(&sk_path, &sk_bytes)
        .unwrap_or_else(|e| err_exit(format!("запись {}: {e}", sk_path.display())));

    println!(
        "   Публичный ключ  : {} ({})",
        pk_path.display(),
        common::kb(pk_bytes.len())
    );
    println!(
        "   Секретный ключ  : {} ({})  — хранится только у больницы",
        sk_path.display(),
        common::kb(sk_bytes.len())
    );
    println!("   Публичный ключ передаётся лаборатории для ReKeyGen.");
}

fn cmd_decrypt(args: &[String]) {
    let dir = match args.iter().position(|a| a == "--dir") {
        Some(idx) if idx + 1 < args.len() => resolve_dir(&args[idx + 1]),
        _ => PathBuf::from("handoff"),
    };
    let model_raw = arg_after(args, "--model")
        .unwrap_or_else(|| err_exit("decrypt: требуется --model '<json или файл>'"));
    let patients_raw = arg_after(args, "--patients");
    let model_str = resolve_json_arg("--model", &model_raw).unwrap_or_else(|e| err_exit(e));
    let model = parse_model(&model_str).unwrap_or_else(|e| err_exit(e));

    let cc = make_context();
    println!("--- Больница: расшифровка скоров и порог риска ---");

    let sk_path = dir.join("hosp.sk.bin");
    let scores_path = dir.join("scores.ct.bin");
    let sk_bytes = std::fs::read(&sk_path)
        .unwrap_or_else(|e| err_exit(format!("чтение {}: {e}", sk_path.display())));
    let sk = cc
        .deserialize_secret_key(&sk_bytes)
        .unwrap_or_else(|e| err_exit(format!("deserialize_sk: {e}")));

    let (kind, n, _f, blobs) = read_handoff(&scores_path).unwrap_or_else(|e| err_exit(e));
    if kind != KIND_SCORES {
        err_exit(format!(
            "{}: не scores-контейнер (kind={kind})",
            scores_path.display()
        ));
    }
    let cts = common::blobs_to_ciphertexts(&cc, &blobs).unwrap_or_else(|e| err_exit(e));

    // Расшифровка по чанкам.
    let mut scores: Vec<f64> = Vec::with_capacity(n as usize);
    let mut remaining = n as usize;
    for ct in &cts {
        let k = remaining.min(common::BATCH);
        let got = decrypt_scores(&cc, &sk, ct, k).unwrap_or_else(|e| err_exit(e));
        scores.extend(got.into_iter().take(k));
        remaining -= k;
    }

    println!();
    println!(
        "   {:<6} {:>12} {:>16}",
        "Пац.",
        "Скор (HE)",
        format!("Риск >= {:.2}?", model.threshold)
    );
    let mut n_risk = 0usize;
    for (i, s) in scores.iter().enumerate() {
        let risk = *s >= model.threshold;
        if risk {
            n_risk += 1;
        }
        let name = format!("P{}", i + 1);
        println!(
            "   {:<6} {:>12.4} {:>16}",
            name,
            s,
            if risk { "ДА" } else { "нет" }
        );
    }
    println!();
    println!("   Итог: пациентов = {}, риск у {}", n, n_risk);

    // Демонстрационная сверка точности (только если больнице передан --patients).
    if let Some(ps) = patients_raw {
        let ps = resolve_json_arg("--patients", &ps).unwrap_or_else(|e| err_exit(e));
        let patients = parse_patients(&ps).unwrap_or_else(|e| err_exit(e));
        let mut max_err = 0.0f64;
        let mut mismatches = 0usize;
        for (i, (s, p)) in scores.iter().zip(patients.iter()).enumerate() {
            let want = model.score_plain(&p.features);
            max_err = max_err.max((*s - want).abs());
            if (*s >= model.threshold) != (want >= model.threshold) {
                mismatches += 1;
            }
            let _ = i;
        }
        println!(
            "   Сверка (демонстрационная): max|HE скор - открытый скор| = {max_err:.1e}; несовпадений порога: {mismatches}"
        );
        if mismatches > 0 || max_err > 1e-2 {
            err_exit(format!(
                "сверка не сошлась: max|err|={max_err:.1e}, mismatches={mismatches}"
            ));
        }
        println!("   Сверка пройдена: max|err| < 1e-2, расхождений с порогом нет.");
    }
}

fn arg_after(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}
