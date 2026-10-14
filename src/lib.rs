//! `hepre` — безопасная Rust-обёртка над OpenFHE (CKKS/BFV + PRE) через cxx bridge.
//!
//! Слой A1: контексты, ключи, шифрование/дешифрование, гомоморфные операции
//! (EvalAdd/EvalMult/Rescale), прокси-перешифрование (ReKeyGen/ReEncrypt) и
//! бинарная сериализация ключей/шифротекстов.
//!
//! Все FFI-функции сами перехватывают C++-исключения: провал сообщается
//! нулевым хэндлом / пустым контейнером, а текст ошибки читается через
//! `error_pop()`.

// Научно-воспроизводимый крейт: ряд style-линтов сознательно разрешён,
// чтобы не переписывать числовые алгоритмы под форму clippy.
#![allow(clippy::excessive_precision)] // литералы с точностью зафиксированы намеренно
#![allow(clippy::needless_range_loop)] // индексы в циклах читаются как в исходном алгоритме
#![allow(clippy::manual_range_contains)] // явные сравнения диапазонов прозрачнее
#![allow(clippy::manual_memcpy)] // циклы копирования понятны без unsafe::copy_from_slice
#![allow(clippy::inconsistent_digit_grouping)] // разделители цифр — по исходным данным
#![allow(clippy::module_inception)] // соглашение имён модулей сохранено
#![allow(clippy::too_many_arguments)] // сигнатуры числовых конвейеров объёмны сознательно
#![allow(clippy::manual_div_ceil)]
// явная формула деления с округлением документирует алгоритм
//! `take_last_error` и возвращается как [`Error`].

pub(crate) mod ffi;

// --- криптографический слой -------------------------------------------------
pub mod he;
pub mod scoring;

// --- научный слой исследования ---------------------------------------------
// Модули ниже не зависят от OpenFHE и покрыты тестами с эталонными
// значениями: генерация когорты, логистическая регрессия, аппроксимация
// сигмоиды, статистика и воспроизводимость.
pub mod cohort;
pub mod detmath;
pub mod env;
pub mod hash;
pub mod logistic;
pub mod poly;
pub mod rng;
pub mod stats;
pub mod study;

/// Ошибка слоя (сообщение из C++-исключения).
#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

type Result<T> = std::result::Result<T, Error>;

fn last_error_or_fallback() -> String {
    let s = ffi::ffi::take_last_error();
    if s.is_empty() {
        "FFI call failed".to_string()
    } else {
        s
    }
}

/// Ненулевой хэндл либо текст последней C++-ошибки.
fn ok_handle(raw: usize) -> Result<usize> {
    if raw != 0 {
        Ok(raw)
    } else {
        Err(Error(last_error_or_fallback()))
    }
}

/// Непустой вектор либо текст последней C++-ошибки.
fn ok_vec<T>(v: Vec<T>) -> Result<Vec<T>> {
    if !v.is_empty() {
        Ok(v)
    } else {
        Err(Error(last_error_or_fallback()))
    }
}

// ---------------------------------------------------------------------------
// Конфигурация контекста
// ---------------------------------------------------------------------------

/// Параметры создания CKKS-контекста.
#[derive(Clone, Copy, Debug)]
pub struct CkksParams {
    pub batch_size: u32,
    pub multiplicative_depth: u32,
    /// Точность масштабирования (биты): 30–60, обычно 40.
    pub scaling_mod_size: u32,
}

impl Default for CkksParams {
    fn default() -> Self {
        Self {
            batch_size: 8192,
            multiplicative_depth: 2,
            scaling_mod_size: 40,
        }
    }
}

/// Параметры создания BFV-контекста.
#[derive(Clone, Copy, Debug)]
pub struct BfvParams {
    pub batch_size: u32,
    pub multiplicative_depth: u32,
    pub plaintext_modulus: u32,
}

impl Default for BfvParams {
    fn default() -> Self {
        Self {
            batch_size: 8192,
            multiplicative_depth: 2,
            plaintext_modulus: 65537,
        }
    }
}

// ---------------------------------------------------------------------------
// Типожители (RAII)
// ---------------------------------------------------------------------------

macro_rules! handle {
    ($name:ident, $free:ident) => {
        pub struct $name {
            raw: usize,
        }

        impl $name {
            pub(crate) fn from_raw(raw: usize) -> Self {
                Self { raw }
            }
        }

        impl Drop for $name {
            fn drop(&mut self) {
                ffi::ffi::$free(self.raw);
            }
        }
    };
}

handle!(Context, ctx_free);
handle!(KeyPair, kp_free);
handle!(PublicKey, key_free);
handle!(SecretKey, key_free);
handle!(EvalKey, evalkey_free);
handle!(Plaintext, plain_free);
handle!(Ciphertext, ct_free);

// ---------------------------------------------------------------------------
// Контекст
// ---------------------------------------------------------------------------

impl Context {
    pub fn new_ckks(p: CkksParams) -> Result<Self> {
        let raw = ok_handle(ffi::ffi::ctx_new_ckks(
            p.batch_size,
            p.multiplicative_depth,
            p.scaling_mod_size,
        ))?;
        Self::finish_creation(raw)
    }

    pub fn new_bfv(p: BfvParams) -> Result<Self> {
        let raw = ok_handle(ffi::ffi::ctx_new_bfv(
            p.batch_size,
            p.multiplicative_depth,
            p.plaintext_modulus,
        ))?;
        Self::finish_creation(raw)
    }

    fn finish_creation(raw: usize) -> Result<Self> {
        ffi::ffi::ctx_enable_all(raw);
        let err = ffi::ffi::take_last_error();
        if err.is_empty() {
            Ok(Context::from_raw(raw))
        } else {
            ffi::ffi::ctx_free(raw);
            Err(Error(err))
        }
    }

    pub fn ring_dimension(&self) -> u32 {
        ffi::ffi::ctx_ring_dim(self.raw)
    }

    // --- keys ---------------------------------------------------------------

    pub fn keygen(&self) -> Result<KeyPair> {
        ok_handle(ffi::ffi::keygen(self.raw)).map(KeyPair::from_raw)
    }

    pub fn evmult_keygen(&self, sk: &SecretKey) -> Result<()> {
        ffi::ffi::evmult_keygen(self.raw, sk.raw);
        let err = ffi::ffi::take_last_error();
        if err.is_empty() {
            Ok(())
        } else {
            Err(Error(err))
        }
    }

    /// ReKeyGen для PRE: ключ перешифрования от `from_sk` к публичному ключу `to_pk`.
    pub fn rekeygen(&self, from_sk: &SecretKey, to_pk: &PublicKey) -> Result<EvalKey> {
        ok_handle(ffi::ffi::rekeygen(self.raw, from_sk.raw, to_pk.raw)).map(EvalKey::from_raw)
    }

    // --- plaintext ----------------------------------------------------------

    pub fn make_ckks_plaintext(&self, values: &[f64]) -> Result<Plaintext> {
        ok_handle(ffi::ffi::ckks_plain(self.raw, values)).map(Plaintext::from_raw)
    }

    pub fn make_bfv_plaintext(&self, values: &[i64]) -> Result<Plaintext> {
        ok_handle(ffi::ffi::bfv_plain(self.raw, values)).map(Plaintext::from_raw)
    }

    // --- encrypt / decrypt --------------------------------------------------

    pub fn encrypt(&self, pk: &PublicKey, pt: &Plaintext) -> Result<Ciphertext> {
        ok_handle(ffi::ffi::encrypt(self.raw, pk.raw, pt.raw)).map(Ciphertext::from_raw)
    }

    pub fn decrypt_ckks(&self, sk: &SecretKey, ct: &Ciphertext, len: u32) -> Result<Vec<f64>> {
        ok_vec(ffi::ffi::decrypt_ckks(self.raw, sk.raw, ct.raw, len))
    }

    pub fn decrypt_bfv(&self, sk: &SecretKey, ct: &Ciphertext, len: u32) -> Result<Vec<i64>> {
        ok_vec(ffi::ffi::decrypt_bfv(self.raw, sk.raw, ct.raw, len))
    }

    // --- evaluation ---------------------------------------------------------

    pub fn eval_add(&self, a: &Ciphertext, b: &Ciphertext) -> Result<Ciphertext> {
        ok_handle(ffi::ffi::eval_add(self.raw, a.raw, b.raw)).map(Ciphertext::from_raw)
    }

    pub fn eval_add_plain(&self, ct: &Ciphertext, pt: &Plaintext) -> Result<Ciphertext> {
        ok_handle(ffi::ffi::eval_add_plain(self.raw, ct.raw, pt.raw)).map(Ciphertext::from_raw)
    }

    pub fn eval_mult(&self, a: &Ciphertext, b: &Ciphertext) -> Result<Ciphertext> {
        ok_handle(ffi::ffi::eval_mult(self.raw, a.raw, b.raw)).map(Ciphertext::from_raw)
    }

    pub fn eval_mult_plain(&self, ct: &Ciphertext, pt: &Plaintext) -> Result<Ciphertext> {
        ok_handle(ffi::ffi::eval_mult_plain(self.raw, ct.raw, pt.raw)).map(Ciphertext::from_raw)
    }

    pub fn rescale(&self, ct: &Ciphertext) -> Result<Ciphertext> {
        ok_handle(ffi::ffi::rescale(self.raw, ct.raw)).map(Ciphertext::from_raw)
    }

    // --- PRE ----------------------------------------------------------------

    pub fn reencrypt(&self, ek: &EvalKey, ct: &Ciphertext) -> Result<Ciphertext> {
        ok_handle(ffi::ffi::reencrypt(self.raw, ek.raw, ct.raw)).map(Ciphertext::from_raw)
    }

    // --- serialization ------------------------------------------------------

    pub fn deserialize_ciphertext(&self, bytes: &[u8]) -> Result<Ciphertext> {
        ok_handle(ffi::ffi::deser_ct(self.raw, bytes)).map(Ciphertext::from_raw)
    }

    pub fn deserialize_public_key(&self, bytes: &[u8]) -> Result<PublicKey> {
        ok_handle(ffi::ffi::deser_pk(self.raw, bytes)).map(PublicKey::from_raw)
    }

    pub fn deserialize_secret_key(&self, bytes: &[u8]) -> Result<SecretKey> {
        ok_handle(ffi::ffi::deser_sk(self.raw, bytes)).map(SecretKey::from_raw)
    }

    pub fn deserialize_eval_key(&self, bytes: &[u8]) -> Result<EvalKey> {
        ok_handle(ffi::ffi::deser_rekey(self.raw, bytes)).map(EvalKey::from_raw)
    }
}

// ---------------------------------------------------------------------------
// Ключи
// ---------------------------------------------------------------------------

impl KeyPair {
    pub fn public_key(&self) -> Result<PublicKey> {
        ok_handle(ffi::ffi::kp_public(self.raw)).map(PublicKey::from_raw)
    }

    pub fn secret_key(&self) -> Result<SecretKey> {
        ok_handle(ffi::ffi::kp_secret(self.raw)).map(SecretKey::from_raw)
    }
}

impl PublicKey {
    pub fn serialize(&self) -> Result<Vec<u8>> {
        ok_vec(ffi::ffi::ser_pk(0, self.raw))
    }
}

impl SecretKey {
    pub fn serialize(&self) -> Result<Vec<u8>> {
        ok_vec(ffi::ffi::ser_sk(0, self.raw))
    }
}

impl EvalKey {
    pub fn serialize(&self) -> Result<Vec<u8>> {
        ok_vec(ffi::ffi::ser_rekey(0, self.raw))
    }
}

impl Ciphertext {
    pub fn serialize(&self) -> Result<Vec<u8>> {
        ok_vec(ffi::ffi::ser_ct(0, self.raw))
    }
}
