use hepre::{CkksParams, Context};

#[test]
fn ciphertext_serialization_roundtrip() {
    let cc = Context::new_ckks(CkksParams {
        batch_size: 8192,
        multiplicative_depth: 2,
        scaling_mod_size: 40,
    })
    .unwrap();
    let keys = cc.keygen().unwrap();
    let pk = keys.public_key().unwrap();
    let sk = keys.secret_key().unwrap();

    let values = [3.0, -1.0, 0.25, 8.0];
    let pt = cc.make_ckks_plaintext(&values).unwrap();
    let ct = cc.encrypt(&pk, &pt).unwrap();

    let bytes = ct.serialize().unwrap();
    assert!(!bytes.is_empty());

    let ct2 = cc.deserialize_ciphertext(&bytes).unwrap();
    let got = cc.decrypt_ckks(&sk, &ct2, 4).unwrap();
    for i in 0..4 {
        assert!((got[i] - values[i]).abs() < 1e-2, "slot {i}: got {}, expected {}", got[i], values[i]);
    }
}

#[test]
fn secret_key_serialization_roundtrip() {
    let cc = Context::new_ckks(CkksParams {
        batch_size: 8192,
        multiplicative_depth: 2,
        scaling_mod_size: 40,
    })
    .unwrap();
    let keys = cc.keygen().unwrap();
    let pk = keys.public_key().unwrap();
    let sk = keys.secret_key().unwrap();

    let sk_bytes = sk.serialize().unwrap();
    assert!(!sk_bytes.is_empty());
    let sk2 = cc.deserialize_secret_key(&sk_bytes).unwrap();

    let values = [5.0, -2.0, 1.5, 0.0];
    let ct = cc.encrypt(&pk, &cc.make_ckks_plaintext(&values).unwrap()).unwrap();
    let got = cc.decrypt_ckks(&sk2, &ct, 4).unwrap();
    for i in 0..4 {
        assert!((got[i] - values[i]).abs() < 1e-2, "slot {i}: got {}, expected {}", got[i], values[i]);
    }
}

#[test]
fn wrong_key_fails_or_garbage() {
    let cc = Context::new_ckks(CkksParams {
        batch_size: 8192,
        multiplicative_depth: 2,
        scaling_mod_size: 40,
    })
    .unwrap();
    let k1 = cc.keygen().unwrap();
    let k2 = cc.keygen().unwrap();

    let pt = cc.make_ckks_plaintext(&[1.0, 2.0, 3.0, 4.0]).unwrap();
    let ct = cc.encrypt(&k1.public_key().unwrap(), &pt).unwrap();
    // чужой ключ должен дать мусор или явную ошибку, но не исходные значения
    match cc.decrypt_ckks(&k2.secret_key().unwrap(), &ct, 4) {
        Err(_) => {} // OpenFHE отвергает дешифрование (слишком высокий шум)
        Ok(got) => {
            let mismatch = got.iter().zip([1.0f64, 2.0, 3.0, 4.0]).any(|(g, e)| (*g - e).abs() > 0.5);
            assert!(mismatch, "wrong key unexpectedly produced the plaintext");
        }
    }
}