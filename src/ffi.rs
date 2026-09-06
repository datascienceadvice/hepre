#[cxx::bridge(namespace = "hepre::ffi")]
pub mod ffi {
    unsafe extern "C++" {
        include!("openfhe-shim.h");

        // error reporting
        fn take_last_error() -> String;

        // context
        fn ctx_new_ckks(batch: u32, depth: u32, scaling_mod_size: u32) -> usize;
        fn ctx_new_bfv(batch: u32, depth: u32, plaintext_modulus: u32) -> usize;
        fn ctx_free(h: usize);
        fn ctx_ring_dim(h: usize) -> u32;
        fn ctx_enable_all(h: usize);

        // keys
        fn keygen(ctx: usize) -> usize;
        fn kp_free(h: usize);
        fn kp_public(h: usize) -> usize;
        fn kp_secret(h: usize) -> usize;
        fn key_free(h: usize);
        fn evmult_keygen(ctx: usize, sk: usize);
        fn rekeygen(ctx: usize, from_sk: usize, to_pk: usize) -> usize;
        fn evalkey_free(h: usize);

        // plaintexts
        fn ckks_plain(ctx: usize, vals: &[f64]) -> usize;
        fn bfv_plain(ctx: usize, vals: &[i64]) -> usize;
        fn plain_free(h: usize);

        // encrypt / decrypt
        fn encrypt(ctx: usize, pk: usize, pt: usize) -> usize;
        fn ct_free(h: usize);
        fn decrypt_ckks(ctx: usize, sk: usize, ct: usize, len: u32) -> Vec<f64>;
        fn decrypt_bfv(ctx: usize, sk: usize, ct: usize, len: u32) -> Vec<i64>;

        // evaluation
        fn eval_add(ctx: usize, a: usize, b: usize) -> usize;
        fn eval_add_plain(ctx: usize, a: usize, pt: usize) -> usize;
        fn eval_mult(ctx: usize, a: usize, b: usize) -> usize;
        fn eval_mult_plain(ctx: usize, a: usize, pt: usize) -> usize;
        fn rescale(ctx: usize, ct: usize) -> usize;

        // proxy re-encryption
        fn reencrypt(ctx: usize, ek: usize, ct: usize) -> usize;

        // serialization
        fn ser_ct(ctx: usize, ct: usize) -> Vec<u8>;
        fn deser_ct(ctx: usize, bytes: &[u8]) -> usize;
        fn ser_pk(ctx: usize, pk: usize) -> Vec<u8>;
        fn deser_pk(ctx: usize, bytes: &[u8]) -> usize;
        fn ser_sk(ctx: usize, sk: usize) -> Vec<u8>;
        fn deser_sk(ctx: usize, bytes: &[u8]) -> usize;
        fn ser_rekey(ctx: usize, ek: usize) -> Vec<u8>;
        fn deser_rekey(ctx: usize, bytes: &[u8]) -> usize;
    }
}