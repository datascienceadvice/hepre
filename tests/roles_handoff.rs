//! Сквозной интеграционный тест: три раздельные утилиты ролей
//! (`lab`, `server`, `hospital`) как отдельные процессы, обменивающиеся
//! только файлами в каталоге handoff.
//!
//! Проверяет кросс-процессную сериализацию ключей/шифротекстов и корректные
//! вердикты порога на 5 «пациентах».

use std::path::PathBuf;
use std::process::Command;

fn bin(name: &str) -> &'static str {
    match name {
        "lab" => env!("CARGO_BIN_EXE_lab"),
        "server" => env!("CARGO_BIN_EXE_server"),
        "hospital" => env!("CARGO_BIN_EXE_hospital"),
        other => panic!("unknown bin {other}"),
    }
}

fn run(dir: &std::path::Path, bin_name: &str, args: &[&str]) -> std::process::Output {
    let out = Command::new(bin(bin_name))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("запуск {bin_name}: {e}"));

    if !out.status.success() {
        eprintln!("--- stderr {bin_name} ---\n{}", String::from_utf8_lossy(&out.stderr));
        panic!("{bin_name} упал с кодом {}", out.status);
    }
    out
}

fn json_model() -> &'static str {
    "{\"weights\":[0.22,0.14,0.08,0.03,0.10],\"intercept\":-1.70,\"threshold\":-1.45}"
}

fn json_patients() -> &'static str {
    "[\
     {\"name\":\"P1\",\"features\":[0.50,0.40,0.20,0.30,0.00]},\
     {\"name\":\"P2\",\"features\":[0.80,0.70,0.50,0.60,1.00]},\
     {\"name\":\"P3\",\"features\":[0.10,0.20,0.10,0.10,0.00]},\
     {\"name\":\"P4\",\"features\":[0.70,0.90,0.70,0.80,1.00]},\
     {\"name\":\"P5\",\"features\":[0.30,0.60,0.30,0.50,0.00]}]"
}

#[test]
fn role_utilities_end_to_end() {
    // Рабочий каталог — внутри каталога сборки, а не в системном TEMP:
    // он гарантированно доступен на запись рядом с проектом, исключён из
    // репозитория и вычищается `cargo clean`. Cargo задаёт
    // CARGO_TARGET_TMPDIR именно для интеграционных тестов.
    let base: PathBuf = match std::env::var_os("CARGO_TARGET_TMPDIR") {
        Some(v) => PathBuf::from(v),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp"),
    };
    let dir: PathBuf = base.join(format!("hepre_roles_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // 1. Больница генерирует ключи.
    run(&dir, "hospital", &["keygen"]);

    // Проверяем, что на диске появились файлы handoff.
    for f in ["hosp.pk.bin", "hosp.sk.bin"] {
        assert!(dir.join("handoff").join(f).is_file(), "нет файла {f}");
    }

    // 2. Лаборатория шифрует фичи и формирует ReKeyGen.
    //    JSON передаём путями к файлам (покрываем файловый режим resolve_json_arg);
    //    в шагах ниже — inline-строками (покрываем inline-режим).
    let model_file = dir.join("model.json");
    let patients_file = dir.join("patients.json");
    // Пишем с UTF-8 BOM (как PowerShell 5.1 Set-Content -Encoding utf8) — парсер обязан это пережить.
    let bom = "\u{feff}";
    std::fs::write(&model_file, format!("{bom}{}", json_model())).unwrap();
    std::fs::write(&patients_file, format!("{bom}{}", json_patients())).unwrap();
    let dirs = dir.join("handoff");
    let hosp_pk = dirs.join("hosp.pk.bin");
    let out = run(
        &dir,
        "lab",
        &[
            "encrypt",
            "--patients",
            patients_file.to_str().unwrap(),
            "--model",
            model_file.to_str().unwrap(),
            "--hospital-pk",
            hosp_pk.to_str().unwrap(),
        ],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("features.cts.bin"), "lab не записал features: {stdout}");
    assert!(stdout.contains("rekey.ek.bin"), "lab не записал rekey: {stdout}");
    for f in ["features.cts.bin", "rekey.ek.bin"] {
        assert!(dirs.join(f).is_file(), "нет файла {f}");
    }

    // 3. Сервер считает скор гомоморфно и делает PRE.
    let out = run(&dir, "server", &["score", "--model", json_model()]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("scores.ct.bin"), "server не записал scores: {stdout}");
    assert!(!stdout.contains("ОШИБКА"), "server вывел ошибку: {stdout}");
    assert!(dirs.join("scores.ct.bin").is_file(), "нет файла scores.ct.bin");

    // 4. Больница расшифровывает (со сверкой — она знает фичи в демо) и печатает вердикты.
    let out = run(
        &dir,
        "hospital",
        &[
            "decrypt",
            "--model",
            model_file.to_str().unwrap(),
            "--patients",
            patients_file.to_str().unwrap(),
        ],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);

    // Скоры близки к открытой модели, вердикты для порога -1.45:
    // P1 ~= -1.509 (нет), P2 ~= -1.268 (ДА), P3 ~= -1.639 (нет), P4 ~= -1.240 (ДА), P5 ~= -1.511 (нет).
    assert!(stdout.contains("Сверка пройдена"), "сверка не прошла: {stdout}");

    let mut rows: Vec<(bool, f64)> = Vec::new();
    for line in stdout.lines() {
        let t = line.trim();
        let parts: Vec<&str> = t.split_whitespace().collect();
        if t.starts_with("P") && parts.len() >= 3 {
            let num = parts[1].replace(',', ".");
            let v: f64 = num.parse().unwrap_or_else(|e| panic!("не число {num}: {e}"));
            let verdict = parts.last().unwrap() == &"ДА";
            rows.push((verdict, v));
        }
    }
    assert_eq!(rows.len(), 5, "ожидали 5 строк с результатами: {stdout}");

    let must_risk = [false, true, false, true, false];
    let want: [f64; 5] = [-1.509, -1.268, -1.639, -1.240, -1.511];
    for (i, ((risk, v), want_v)) in rows.iter().zip(want.iter()).enumerate() {
        assert_eq!(*risk, must_risk[i], "вердикт P{} неверен", i + 1);
        assert!((*v - want_v).abs() < 1e-3, "скор P{}={v}, хотим {want_v}", i + 1);
    }

    let _ = std::fs::remove_dir_all(&dir);
}