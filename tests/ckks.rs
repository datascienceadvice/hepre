use hepre::{Ciphertext, CkksParams, Context, PublicKey, SecretKey};

const TOL: f64 = 1e-2;

fn make_ctx() -> Context {
    Context::new_ckks(CkksParams {
        batch_size: 8192,
        multiplicative_depth: 3,
        scaling_mod_size: 40,
    })
    .unwrap()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < TOL
}

#[test]
fn encrypt_decrypt_roundtrip() {
    let cc = make_ctx();
    let keys = cc.keygen().unwrap();
    let pk = keys.public_key().unwrap();
    let sk = keys.secret_key().unwrap();

    let values = [1.25, -2.5, 3.75, 10.0];
    let pt = cc.make_ckks_plaintext(&values).unwrap();
    let ct = cc.encrypt(&pk, &pt).unwrap();

    let got = ctx_decrypt(&cc, &sk, &ct);
    for (g, e) in got.iter().zip(values.iter()) {
        assert!(close(*g, *e), "got {g}, expected {e}");
    }
}

// CKKS pad (rank): обёртка не хранит число значений; для теста берём 4.
fn ctx_decrypt(cc: &Context, sk: &SecretKey, ct: &Ciphertext) -> Vec<f64> {
    cc.decrypt_ckks(sk, ct, 4).unwrap()
}

#[test]
fn eval_mult_rescale_add() {
    let cc = make_ctx();
    let keys = cc.keygen().unwrap();
    cc.evmult_keygen(&keys.secret_key().unwrap()).unwrap();
    let pk = keys.public_key().unwrap();
    let sk = keys.secret_key().unwrap();

    let a = [1.0, 2.0, 3.0, 4.0];
    let b = [5.0, 6.0, 7.0, 8.0];
    let c = [0.5, 1.5, 2.5, 3.5];

    let ca = cc
        .encrypt(&pk, &cc.make_ckks_plaintext(&a).unwrap())
        .unwrap();
    let cb = cc
        .encrypt(&pk, &cc.make_ckks_plaintext(&b).unwrap())
        .unwrap();
    let cc_ = cc
        .encrypt(&pk, &cc.make_ckks_plaintext(&c).unwrap())
        .unwrap();

    let cm = cc.eval_mult(&ca, &cb).unwrap();
    let cr = cc.rescale(&cm).unwrap();
    let cad = cc.eval_add(&cr, &cc_).unwrap();

    let got = ctx_decrypt(&cc, &sk, &cad);
    for i in 0..4 {
        let exp = a[i] * b[i] + c[i];
        assert!(
            close(got[i], exp),
            "slot {i}: got {}, expected {exp}",
            got[i]
        );
    }
}

#[test]
fn ring_dimension_positive() {
    let cc = make_ctx();
    assert!(cc.ring_dimension() > 0);
    // batch 8192 -> ring 16384
    assert_eq!(cc.ring_dimension(), 16384);
}

#[test]
fn public_key_usable_after_serialize() {
    let cc = make_ctx();
    let keys = cc.keygen().unwrap();
    let pk = keys.public_key().unwrap();
    let sk = keys.secret_key().unwrap();
    let bytes = pk.serialize().unwrap();

    let pk2 = cc.deserialize_public_key(&bytes).unwrap();
    let values = [2.0, -1.0, 0.0, 7.5];
    let pt = cc.make_ckks_plaintext(&values).unwrap();
    let ct = cc.encrypt(&pk2, &pt).unwrap();
    let got = ctx_decrypt(&cc, &sk, &ct);
    for i in 0..4 {
        assert!(
            close(got[i], values[i]),
            "slot {i}: got {}, expected {}",
            got[i],
            values[i]
        );
    }
    assert!(PublicKey::serialize(&pk2).is_ok());
}
