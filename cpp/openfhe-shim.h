#pragma once

#include "rust/cxx.h"
#include <cstddef>
#include <cstdint>

namespace hepre {
namespace ffi {

// Последняя ошибка (пустая строка — ошибок нет); см. openfhe-shim.cc.
rust::String take_last_error();

// --- context ---------------------------------------------------------------
std::size_t ctx_new_ckks(std::uint32_t batch, std::uint32_t depth, std::uint32_t scaling_mod_size);
std::size_t ctx_new_bfv(std::uint32_t batch, std::uint32_t depth, std::uint32_t plaintext_modulus);
void        ctx_free(std::size_t h);
std::uint32_t ctx_ring_dim(std::size_t h);
void        ctx_enable_all(std::size_t h);

// --- keys ------------------------------------------------------------------
std::size_t keygen(std::size_t ctx);
void        kp_free(std::size_t h);
std::size_t kp_public(std::size_t h);
std::size_t kp_secret(std::size_t h);
void        key_free(std::size_t h);
void        evmult_keygen(std::size_t ctx, std::size_t sk);
std::size_t rekeygen(std::size_t ctx, std::size_t from_sk, std::size_t to_pk);
void        evalkey_free(std::size_t h);

// --- plaintexts ------------------------------------------------------------
std::size_t ckks_plain(std::size_t ctx, rust::Slice<const double> vals);
std::size_t bfv_plain(std::size_t ctx, rust::Slice<const std::int64_t> vals);
void        plain_free(std::size_t h);

// --- encrypt / decrypt -----------------------------------------------------
std::size_t        encrypt(std::size_t ctx, std::size_t pk, std::size_t pt);
void               ct_free(std::size_t h);
rust::Vec<double>  decrypt_ckks(std::size_t ctx, std::size_t sk, std::size_t ct, std::uint32_t len);
rust::Vec<std::int64_t> decrypt_bfv(std::size_t ctx, std::size_t sk, std::size_t ct, std::uint32_t len);

// --- evaluation ------------------------------------------------------------
std::size_t eval_add(std::size_t ctx, std::size_t a, std::size_t b);
std::size_t eval_add_plain(std::size_t ctx, std::size_t a, std::size_t pt);
std::size_t eval_mult(std::size_t ctx, std::size_t a, std::size_t b);
std::size_t eval_mult_plain(std::size_t ctx, std::size_t a, std::size_t pt);
std::size_t rescale(std::size_t ctx, std::size_t ct);

// --- proxy re-encryption ---------------------------------------------------
std::size_t reencrypt(std::size_t ctx, std::size_t ek, std::size_t ct);

// --- serialization ---------------------------------------------------------
rust::Vec<std::uint8_t> ser_ct(std::size_t ctx, std::size_t ct);
std::size_t             deser_ct(std::size_t ctx, rust::Slice<const std::uint8_t> bytes);
rust::Vec<std::uint8_t> ser_pk(std::size_t ctx, std::size_t pk);
std::size_t             deser_pk(std::size_t ctx, rust::Slice<const std::uint8_t> bytes);
rust::Vec<std::uint8_t> ser_sk(std::size_t ctx, std::size_t sk);
std::size_t             deser_sk(std::size_t ctx, rust::Slice<const std::uint8_t> bytes);
rust::Vec<std::uint8_t> ser_rekey(std::size_t ctx, std::size_t ek);
std::size_t             deser_rekey(std::size_t ctx, rust::Slice<const std::uint8_t> bytes);

}  // namespace ffi
}  // namespace hepre