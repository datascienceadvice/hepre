use hepre::{Ciphertext, CkksParams, Context, EvalKey, SecretKey};

const TOL: f64 = 1e-2;

fn make_ctx() -> Context {
    Context::new_ckks(CkksParams {
        batch_size: 8192,
        multiplicative_depth: 3,
        scaling_mod_size: 40,
    })
    .unwrap()
}

fn decrypt(cc: &Context, sk: &SecretKey, ct: &Ciphertext) -> Vec<f64> {
    cc.decrypt_ckks(sk, ct, 4).unwrap()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < TOL
}

#[test]
fn pre_reencrypt_recipient_can_decrypt() {
    let cc = make_ctx();

    // лаборатория (источник данных)
    let alice = cc.keygen().unwrap();
    let alice_pk = alice.public_key().unwrap();
    let alice_sk = alice.secret_key().unwrap();

    // сервер (получатель результата)
    let bob = cc.keygen().unwrap();
    let bob_pk = bob.public_key().unwrap();
    let bob_sk = bob.secret_key().unwrap();

    // ре-ключ alice -> bob, выдаёт владелец alice (не отдаёт серверу свой секret)
    let ek = cc.rekeygen(&alice_sk, &bob_pk).unwrap();

    let values = [1.5, -2.25, 3.0, 10.5];
    let pt = cc.make_ckks_plaintext(&values).unwrap();
    let ct = cc.encrypt(&alice_pk, &pt).unwrap();

    // сервер перешифровывает и расшифровывает своим ключом
    let ct2 = cc.reencrypt(&ek, &ct).unwrap();
    let got = decrypt(&cc, &bob_sk, &ct2);
    for i in 0..4 {
        assert!(close(got[i], values[i]), "slot {i}: got {}, expected {}", got[i], values[i]);
    }
}

#[test]
fn pre_evalkey_serialization_roundtrip() {
    let cc = make_ctx();

    let alice = cc.keygen().unwrap();
    let alice_pk = alice.public_key().unwrap();
    let alice_sk = alice.secret_key().unwrap();
    let bob = cc.keygen().unwrap();
    let bob_pk = bob.public_key().unwrap();
    let bob_sk = bob.secret_key().unwrap();

    let ek = cc.rekeygen(&alice_sk, &bob_pk).unwrap();
    let ek_bytes = ek.serialize().unwrap();
    let ek2: EvalKey = cc.deserialize_eval_key(&ek_bytes).unwrap();

    let values = [7.0, -1.0, 0.5, 4.0];
    let ct = cc.encrypt(&alice_pk, &cc.make_ckks_plaintext(&values).unwrap()).unwrap();
    let ct_bytes = ct.serialize().unwrap();
    let ct2 = cc.deserialize_ciphertext(&ct_bytes).unwrap();

    let ct3 = cc.reencrypt(&ek2, &ct2).unwrap();
    let got = decrypt(&cc, &bob_sk, &ct3);
    for i in 0..4 {
        assert!(close(got[i], values[i]), "slot {i}: got {}, expected {}", got[i], values[i]);
    }
}