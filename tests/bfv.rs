use hepre::{BfvParams, Context};

const T: i64 = 65537;

fn make_ctx() -> Context {
    Context::new_bfv(BfvParams {
        batch_size: 8192,
        multiplicative_depth: 2,
        plaintext_modulus: T as u32,
    })
    .unwrap()
}

#[test]
fn bfv_roundtrip_and_eval() {
    let cc = make_ctx();
    let keys = cc.keygen().unwrap();
    cc.evmult_keygen(&keys.secret_key().unwrap()).unwrap();
    let pk = keys.public_key().unwrap();
    let sk = keys.secret_key().unwrap();

    let a: Vec<i64> = (1..=4).map(|i| i * 3).collect();
    let b: Vec<i64> = (1..=4).collect();
    let c: Vec<i64> = (1..=4).map(|i| i * 7 % 20).collect();

    let pt = cc.make_bfv_plaintext(&a).unwrap();
    let ct = cc.encrypt(&pk, &pt).unwrap();
    let got = cc.decrypt_bfv(&sk, &ct, 4).unwrap();
    assert_eq!(got, a, "roundtrip mismatch");

    let ca = cc
        .encrypt(&pk, &cc.make_bfv_plaintext(&a).unwrap())
        .unwrap();
    let cb = cc
        .encrypt(&pk, &cc.make_bfv_plaintext(&b).unwrap())
        .unwrap();
    let cc_ = cc
        .encrypt(&pk, &cc.make_bfv_plaintext(&c).unwrap())
        .unwrap();

    let cm = cc.eval_mult(&ca, &cb).unwrap();
    let cad = cc.eval_add(&cm, &cc_).unwrap();

    let got = cc.decrypt_bfv(&sk, &cad, 4).unwrap();
    for i in 0..4 {
        let exp = (a[i] * b[i] + c[i]).rem_euclid(T);
        assert_eq!(got[i], exp, "slot {i}: got {}, expected {exp}", got[i]);
    }
}

#[test]
fn bfv_secret_key_roundtrip_serialization() {
    let cc = make_ctx();
    let keys = cc.keygen().unwrap();
    let pk = keys.public_key().unwrap();
    let sk = keys.secret_key().unwrap();

    let sk_bytes = sk.serialize().unwrap();
    let sk2 = cc.deserialize_secret_key(&sk_bytes).unwrap();

    let a: Vec<i64> = (10..=13).collect();
    let ct = cc
        .encrypt(&pk, &cc.make_bfv_plaintext(&a).unwrap())
        .unwrap();
    let got = cc.decrypt_bfv(&sk2, &ct, 4).unwrap();
    assert_eq!(got, a);
}
