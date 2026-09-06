//! Бенчмарк приватного скоринга (этап A2): лаба → сервер → больница.
//!
//! Запуск (Release!):
//!   $env:PATH = "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\<ver>\bin\Hostx64\x64;$env:PATH"
//!   cargo run --release --example bench
//!
//! Замеряет KeyGen/ReKeyGen/Encrypt/EvalMultPlain/EvalAdd/EvalAddPlain/ReEncrypt/Decrypt
//! для N = 10 / 100 / 1000 / 10000 пациентов (чанками по batch 8192 слотов) и
//! точность непрерывного скора и бинарного порога относительно открытой модели.

use std::time::Instant;

use hepre::scoring::{compute_score, decrypt_scores, encrypt_features, transfer_score, ScoringModel};
use hepre::{CkksParams, Context, PublicKey, SecretKey};

const BATCH: usize = 8192;
const SIZES: [usize; 4] = [10, 100, 1000, 10000];

// Нормированные фичи: age, bmi, glucose, family_hist (0/1), hba1c.
fn model() -> ScoringModel {
    ScoringModel { weights: vec![0.03, 0.08, 0.14, 0.10, 0.22], intercept: -1.7, threshold: 0.0 }
}

fn dataset(n: usize) -> Vec<Vec<f64>> {
    (0..n)
        .map(|i| {
            let i = i as f64;
            vec![
                0.5 + 0.3 * (i % 80.0) / 80.0,
                0.25 + 0.05 * (i % 20.0) / 20.0,
                0.6 + 0.3 * (i % 13.0) / 13.0,
                if (i as usize + 7) % 2 == 0 { 1.0 } else { 0.0 },
                0.05 + 0.03 * (i % 11.0) / 11.0,
            ]
        })
        .collect()
}

struct Timings {
    encrypt_ms: f64,
    score_ms: f64,
    pre_ms: f64,
    decrypt_ms: f64,
    max_abs_err: f64,
    mismatches: usize,
}

fn run_size(
    cc: &Context,
    lab_pk: &PublicKey,
    hosp_sk: &SecretKey,
    rekey: &hepre::EvalKey,
    n: usize,
) -> Timings {
    let m = model();
    let xs = dataset(n);

    let mut encrypt_ms = 0.0;
    let mut score_ms = 0.0;
    let mut pre_ms = 0.0;
    let mut decrypt_ms = 0.0;
    let mut max_abs_err = 0.0f64;
    let mut mismatches = 0usize;

    for chunk in xs.chunks(BATCH) {
        let t = Instant::now();
        let feats = encrypt_features(cc, lab_pk, &m, chunk).unwrap();
        encrypt_ms += t.elapsed().as_secs_f64() * 1000.0;

        let t = Instant::now();
        let score = compute_score(cc, &feats, &m).unwrap();
        score_ms += t.elapsed().as_secs_f64() * 1000.0;

        let t = Instant::now();
        let transferred = transfer_score(cc, rekey, &score).unwrap();
        pre_ms += t.elapsed().as_secs_f64() * 1000.0;

        let t = Instant::now();
        let got = decrypt_scores(cc, hosp_sk, &transferred, chunk.len()).unwrap();
        decrypt_ms += t.elapsed().as_secs_f64() * 1000.0;

        for (g, x) in got.iter().zip(chunk.iter()) {
            let expected = m.score_plain(x);
            max_abs_err = max_abs_err.max((g - expected).abs());
            if (*g >= m.threshold) != m.predict_plain(x) {
                mismatches += 1;
            }
        }
    }

    Timings { encrypt_ms, score_ms, pre_ms, decrypt_ms, max_abs_err, mismatches }
}

fn main() {
    println!("=== Private diabetes-risk scoring: lab -> server(FHE) -> hospital(PRE) ===");
    println!("batch (CKKS slots) = {BATCH}, features = 5, additive linear scorer.");

    let cc = Context::new_ckks(CkksParams { batch_size: 8192, multiplicative_depth: 3, scaling_mod_size: 40 })
        .expect("ctx_new_ckks");

    // Лаборатория и больница — один раз. ReKeyGen (lab_sk -> hosp_pk) тоже один раз.
    let t = Instant::now();
    let lab = cc.keygen().unwrap();
    let keygen_ms = t.elapsed().as_secs_f64() * 1000.0;
    let lab_pk = lab.public_key().unwrap();
    let lab_sk = lab.secret_key().unwrap();

    let hosp = cc.keygen().unwrap();
    let hosp_pk = hosp.public_key().unwrap();
    let hosp_sk = hosp.secret_key().unwrap();

    // Прогрев первого вызова (инициализация глобальных реестров OpenFHE).
    let warm = dataset(4);
    let feats0 = encrypt_features(&cc, &lab_pk, &model(), &warm).unwrap();
    let s0 = compute_score(&cc, &feats0, &model()).unwrap();
    let _ = transfer_score(&cc, &cc.rekeygen(&lab_sk, &hosp_pk).unwrap(), &s0).unwrap();

    let t = Instant::now();
    let rekey = cc.rekeygen(&lab_sk, &hosp_pk).unwrap();
    let rekeygen_ms = t.elapsed().as_secs_f64() * 1000.0;

    println!("KeyGen(lab) = {:.2} ms; ReKeyGen(lab_sk -> hosp_pk) = {:.2} ms.", keygen_ms, rekeygen_ms);
    println!();
    println!(
        "{:<6} {:>10} {:>10} {:>10} {:>10} | {:>9} {:>7}",
        "N", "Encrypt", "Score", "PRE", "Decrypt", "max|err|", "binMis"
    );
    println!(
        "{:-<6} {:-<10} {:-<10} {:-<10} {:-<10} | {:-<9} {:-<7}",
        "", "", "", "", "", "", ""
    );

    for n in SIZES {
        let r = run_size(&cc, &lab_pk, &hosp_sk, &rekey, n);
        println!(
            "{:<6} {:>8.2}ms {:>8.2}ms {:>8.2}ms {:>8.2}ms | {:>9.2e} {:>7}",
            n, r.encrypt_ms, r.score_ms, r.pre_ms, r.decrypt_ms, r.max_abs_err, r.mismatches
        );
        if r.mismatches > 0 {
            println!("  !! {} threshold mismatches for N={n}", r.mismatches);
        }
    }

    println!();
    println!("Notes:");
    println!("  * KeyGen/ReKeyGen measured once (not per patient); Encrypt/Score/PRE/Decrypt = full cohort wall-time.");
    println!("  * Ciphertexts encrypted under lab pk; server computes score; PRE transfers to hospital;");
    println!("    hospital decrypts with its own sk and applies the threshold in the clear.");
    println!("  * Run with `--release` for realistic timings.");
}