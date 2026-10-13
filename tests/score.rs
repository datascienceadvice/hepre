//! Этап A2 — сквозной приватный скоринг: лаба → сервер → больница (PRE).

use hepre::scoring::{
    compute_score, decrypt_scores, encrypt_features, scoring_context, transfer_score, EncodedFeatures,
    ScoringModel,
};
use hepre::{CkksParams, Context};

// Небольшая синтетическая модель "риска диабета": 3 признака.
fn model() -> ScoringModel {
    ScoringModel {
        weights: vec![0.4, -0.2, 0.7],
        intercept: 0.1,
        threshold: 0.5,
    }
}

fn dataset() -> Vec<Vec<f64>> {
    vec![
        vec![0.5, 0.1, 0.8],
        vec![0.9, 0.7, 0.2],
        vec![-0.3, 0.4, 0.6],
        vec![1.0, 0.0, 1.0],
        vec![0.0, 0.5, 0.0],
    ]
}

fn context() -> Context {
    scoring_context().unwrap()
}

#[test]
fn end_to_end_score_and_pre_to_hospital() {
    let cc = context();
    let m = model();
    let lab = cc.keygen().unwrap();
    let lab_pk = lab.public_key().unwrap();
    let lab_sk = lab.secret_key().unwrap();

    // 1) Лаборатория шифрует фичи пациентов.
    let feats: EncodedFeatures =
        encrypt_features(&cc, &lab_pk, &m, &dataset()).expect("encrypt_features");

    // 2) Сервер гомоморфно считает скор (не видит ни фичи, ни результат).
    let score = compute_score(&cc, &feats, &m).expect("compute_score");

    // 3) Больница генерирует свою пару ключей.
    let hosp = cc.keygen().unwrap();
    let hosp_pk = hosp.public_key().unwrap();
    let _hosp_sk = hosp.secret_key().unwrap();

    // Лаборатория выпускает ключ перешифрования со своего sk на pk больницы.
    let rekey = cc.rekeygen(&lab_sk, &hosp_pk).expect("rekeygen");

    // 4) Сервер перешифровывает скор под ключ больницы.
    let transferred = transfer_score(&cc, &rekey, &score).expect("transfer_score");

    // 5) Больница расшифровывает скоры и применяет порог в открытом виде.
    let got = decrypt_scores(&cc, &_hosp_sk, &transferred, feats.n).unwrap();

    let xs = dataset();
    for (i, (g, x)) in got.iter().zip(xs.iter()).enumerate() {
        let expected = m.score_plain(x);
        let close = (g - expected).abs() < 0.1;
        assert!(close, "row {i}: got {g}, expected {expected}");
    }

    // Порог: больница решает рисковать/не рисковать.
    let binary: Vec<bool> = got.iter().map(|s| *s >= m.threshold).collect();
    let expected_binary: Vec<bool> = xs.iter().map(|x| m.predict_plain(x)).collect();
    assert_eq!(binary, expected_binary, "thresholded predictions match");
}

#[test]
fn wrong_secret_key_cannot_read_transferred_score() {
    let cc = context();
    let m = model();
    let lab = cc.keygen().unwrap();
    let lab_pk = lab.public_key().unwrap();
    let lab_sk = lab.secret_key().unwrap();

    let feats = encrypt_features(&cc, &lab_pk, &m, &dataset()).unwrap();
    let score = compute_score(&cc, &feats, &m).unwrap();

    let hosp = cc.keygen().unwrap();
    let hosp_pk = hosp.public_key().unwrap();
    let _hosp_sk = hosp.secret_key().unwrap();

    let rekey = cc.rekeygen(&lab_sk, &hosp_pk).unwrap();
    let transferred = transfer_score(&cc, &rekey, &score).unwrap();

    // Расшифровка чужим серверным ключом должна дать мусор (не исходные скоры);
    // OpenFHE Decode() при высоком уровне ошибки может и бросить исключение → Err.
    let server_keys = cc.keygen().unwrap();
    let server_sk = server_keys.secret_key().unwrap();
    match decrypt_scores(&cc, &server_sk, &transferred, feats.n) {
        Ok(got) => {
            for (i, (g, x)) in got.iter().zip(dataset().iter()).enumerate() {
                let expected = m.score_plain(x);
                let close = (g - expected).abs() < 0.1;
                assert!(!close, "row {i}: wrong key produced a plausible score {g}");
            }
        }
        Err(_e) => { /* fail-closed (Decode error) — тоже приемлемо */ }
    }
}

#[test]
fn multiple_patients_in_one_batch() {
    let cc = Context::new_ckks(CkksParams {
        batch_size: 8192,
        multiplicative_depth: 3,
        scaling_mod_size: 40,
    })
    .unwrap();
    let m = model();
    let keys = cc.keygen().unwrap();
    let pk = keys.public_key().unwrap();

    // 200 пациентов в одном пакете (слоты 0..200).
    let xs: Vec<Vec<f64>> = (0..200)
        .map(|i| vec![0.01 * i as f64, 0.5 - 0.001 * i as f64, 0.01 * i as f64])
        .collect();
    let feats = encrypt_features(&cc, &pk, &m, &xs).unwrap();
    let score = compute_score(&cc, &feats, &m).unwrap();
    let got = decrypt_scores(&cc, &keys.secret_key().unwrap(), &score, 200).unwrap();
    for (i, (g, x)) in got.iter().zip(xs.iter()).enumerate() {
        let expected = m.score_plain(x);
        assert!((g - expected).abs() < 0.2, "row {i}: got {g}, expected {expected}");
    }
}