#include "openfhe-shim.h"

#include "openfhe/core/openfhecore.h"
#include "openfhe.h"

#include <memory>
#include <mutex>
#include <sstream>
#include <stdexcept>
#include <string>
#include <utility>
#include <vector>

using namespace lbcrypto;

// Заголовки cereal-сериализации OpenFHE используют незаквалифицированные
// типы (например, DCRTPoly), поэтому их можно включать только после
// `using namespace lbcrypto;`.
#include "openfhe/pke/ciphertext-ser.h"
#include "openfhe/pke/cryptocontext-ser.h"
#include "openfhe/pke/key/key-ser.h"
#include "openfhe/pke/scheme/bfvrns/bfvrns-ser.h"
#include "openfhe/pke/scheme/ckksrns/ckksrns-ser.h"
#include "openfhe/pke/schemerns/rns-ser.h"

namespace {

// OpenFHE использует глобальные статические реестры контекстов без
// синхронизации, поэтому все входы в FFI сериализуются единым мьютексом.
std::mutex g_ffi_mutex;

// Лок сквирается в начале каждой публичной FFI-функции.
#define LOCK() std::lock_guard<std::mutex> g_lock(g_ffi_mutex)

// HANDLE SYSTEM ------------------------------------------------------------
// Хэндл — выделенная в куче "коробка" с type-erased shared_ptr<void>.
// Исходная широкая удаляющая функция (делетер) остаётся в управляющем блоке,
// поэтому один Drop() корректно удаляет объект любого типа.

std::size_t StoreVoid(std::shared_ptr<void> p) {
    auto* box = new std::shared_ptr<void>(std::move(p));
    return reinterpret_cast<std::size_t>(box);
}

template <typename T>
std::size_t Store(std::shared_ptr<T> p) {
    return StoreVoid(std::static_pointer_cast<void>(std::move(p)));
}

std::size_t StoreValue(KeyPair<DCRTPoly> kp) {
    return StoreVoid(std::make_shared<KeyPair<DCRTPoly>>(std::move(kp)));
}

std::shared_ptr<void>& Box(std::size_t h) {
    if (h == 0)
        throw std::runtime_error("null handle");
    return *reinterpret_cast<std::shared_ptr<void>*>(h);
}

template <typename T>
std::shared_ptr<T> Get(std::size_t h) {
    return std::static_pointer_cast<T>(Box(h));
}

void Drop(std::size_t h) {
    if (h != 0)
        delete reinterpret_cast<std::shared_ptr<void>*>(h);
}

// ERROR HANDLING -----------------------------------------------------------
// cxxbridge-обёртки помечаются `noexcept` и не ловят C++-исключения, поэтому
// каждая FFI-функция сама перехватывает исключения и возвращает "нулевое"
// значение (0 / пустой контейнер); текст ошибки сохраняется и возвращается
// по запросу через hepre::ffi::take_last_error.

thread_local std::string g_last_error;

template <typename R, typename F>
R guard(F&& f, R def) {
    LOCK();
    try {
        R r = std::forward<F>(f)();
        g_last_error.clear();
        return r;
    }
    catch (const std::exception& e) {
        g_last_error = e.what();
    }
    catch (...) {
        g_last_error = "unknown C++ exception";
    }
    return def;
}

void EnableAll(CryptoContext<DCRTPoly> cc) {
    cc->Enable(PKE);
    cc->Enable(KEYSWITCH);
    cc->Enable(PRE);
    cc->Enable(LEVELEDSHE);
    cc->Enable(ADVANCEDSHE);
}

template <typename T>
rust::Vec<std::uint8_t> ToBytes(const T& obj) {
    std::stringstream ss(std::ios::in | std::ios::out | std::ios::binary);
    Serial::Serialize(obj, ss, SerType::BINARY);
    const std::string& s = ss.str();
    rust::Vec<std::uint8_t> out;
    out.reserve(s.size());
    for (char c : s)
        out.push_back(static_cast<std::uint8_t>(c));
    return out;
}

template <typename T>
std::size_t FromBytes(const std::string& s) {
    std::istringstream ss(s, std::ios::in | std::ios::binary);
    T obj;
    Serial::Deserialize(obj, ss, SerType::BINARY);
    if (obj == nullptr)
        throw std::runtime_error("deserialization produced null object");
    return Store(obj);
}

}  // namespace

namespace hepre {
namespace ffi {

rust::String take_last_error() {
    // глобальное состояние не потокобезопасно, но для прототипа достаточно
    return rust::String(g_last_error.c_str());
}

// --- context ---------------------------------------------------------------

std::size_t ctx_new_ckks(std::uint32_t batch, std::uint32_t depth, std::uint32_t scaling_mod_size) {
    return guard<std::size_t>(
        [&]() {
            CCParams<CryptoContextCKKSRNS> parameters;
            parameters.SetMultiplicativeDepth(depth);
            parameters.SetScalingModSize(scaling_mod_size);
            parameters.SetBatchSize(batch);
            parameters.SetPREMode(INDCPA);
            return Store(GenCryptoContext(parameters));
        },
        0);
}

std::size_t ctx_new_bfv(std::uint32_t batch, std::uint32_t depth, std::uint32_t plaintext_modulus) {
    return guard<std::size_t>(
        [&]() {
            CCParams<CryptoContextBFVRNS> parameters;
            parameters.SetMultiplicativeDepth(depth);
            parameters.SetPlaintextModulus(plaintext_modulus);
            parameters.SetBatchSize(batch);
            parameters.SetPREMode(INDCPA);
            return Store(GenCryptoContext(parameters));
        },
        0);
}

void ctx_free(std::size_t h) {
    LOCK();
    Drop(h);
}

std::uint32_t ctx_ring_dim(std::size_t h) {
    return guard<std::uint32_t>([&]() { return Get<CryptoContextImpl<DCRTPoly>>(h)->GetRingDimension(); }, 0);
}

void ctx_enable_all(std::size_t h) {
    LOCK();
    try {
        EnableAll(Get<CryptoContextImpl<DCRTPoly>>(h));
        g_last_error.clear();
    }
    catch (const std::exception& e) {
        g_last_error = e.what();
    }
    catch (...) {
        g_last_error = "unknown C++ exception";
    }
}

// --- keys ------------------------------------------------------------------

std::size_t keygen(std::size_t ctx) {
    return guard<std::size_t>(
        [&]() {
            auto cc   = Get<CryptoContextImpl<DCRTPoly>>(ctx);
            auto keys = cc->KeyGen();
            return StoreValue(std::move(keys));
        },
        0);
}

void kp_free(std::size_t h) {
    LOCK();
    Drop(h);
}

std::size_t kp_public(std::size_t h) {
    return guard<std::size_t>(
        [&]() {
            auto kp = Get<KeyPair<DCRTPoly>>(h);
            return Store(kp->publicKey);
        },
        0);
}

std::size_t kp_secret(std::size_t h) {
    return guard<std::size_t>(
        [&]() {
            auto kp = Get<KeyPair<DCRTPoly>>(h);
            return Store(kp->secretKey);
        },
        0);
}

void key_free(std::size_t h) {
    LOCK();
    Drop(h);
}

void evmult_keygen(std::size_t ctx, std::size_t sk) {
    LOCK();
    try {
        auto cc = Get<CryptoContextImpl<DCRTPoly>>(ctx);
        cc->EvalMultKeyGen(Get<PrivateKeyImpl<DCRTPoly>>(sk));
        g_last_error.clear();
    }
    catch (const std::exception& e) {
        g_last_error = e.what();
    }
    catch (...) {
        g_last_error = "unknown C++ exception";
    }
}

std::size_t rekeygen(std::size_t ctx, std::size_t from_sk, std::size_t to_pk) {
    return guard<std::size_t>(
        [&]() {
            auto cc = Get<CryptoContextImpl<DCRTPoly>>(ctx);
            return Store(cc->ReKeyGen(Get<PrivateKeyImpl<DCRTPoly>>(from_sk), Get<PublicKeyImpl<DCRTPoly>>(to_pk)));
        },
        0);
}

void evalkey_free(std::size_t h) {
    LOCK();
    Drop(h);
}

// --- plaintexts ------------------------------------------------------------

std::size_t ckks_plain(std::size_t ctx, rust::Slice<const double> vals) {
    return guard<std::size_t>(
        [&]() {
            auto cc                 = Get<CryptoContextImpl<DCRTPoly>>(ctx);
            std::vector<double> v(vals.begin(), vals.end());
            return Store(cc->MakeCKKSPackedPlaintext(std::move(v)));
        },
        0);
}

std::size_t bfv_plain(std::size_t ctx, rust::Slice<const std::int64_t> vals) {
    return guard<std::size_t>(
        [&]() {
            auto cc = Get<CryptoContextImpl<DCRTPoly>>(ctx);
            std::vector<std::int64_t> v(vals.begin(), vals.end());
            return Store(cc->MakePackedPlaintext(static_cast<const std::vector<std::int64_t>&>(v)));
        },
        0);
}

void plain_free(std::size_t h) {
    LOCK();
    Drop(h);
}

// --- encrypt / decrypt -----------------------------------------------------

std::size_t encrypt(std::size_t ctx, std::size_t pk, std::size_t pt) {
    return guard<std::size_t>(
        [&]() {
            auto cc     = Get<CryptoContextImpl<DCRTPoly>>(ctx);
            auto cipher = cc->Encrypt(Get<PublicKeyImpl<DCRTPoly>>(pk), Get<PlaintextImpl>(pt));
            if (cipher == nullptr)
                throw std::runtime_error("encryption returned null ciphertext");
            return Store(cipher);
        },
        0);
}

void ct_free(std::size_t h) {
    LOCK();
    Drop(h);
}

rust::Vec<double> decrypt_ckks(std::size_t ctx, std::size_t sk, std::size_t ct, std::uint32_t len) {
    return guard<rust::Vec<double>>(
        [&]() {
            auto cc = Get<CryptoContextImpl<DCRTPoly>>(ctx);
            Plaintext result;
            cc->Decrypt(Get<PrivateKeyImpl<DCRTPoly>>(sk), Get<CiphertextImpl<DCRTPoly>>(ct), &result);
            if (result == nullptr)
                throw std::runtime_error("decryption returned null plaintext");
            if (len > 0)
                result->SetLength(len);
            const auto& values = result->GetCKKSPackedValue();
            rust::Vec<double> out;
            out.reserve(values.size());
            for (const auto& v : values)
                out.push_back(v.real());
            return out;
        },
        rust::Vec<double>());
}

rust::Vec<std::int64_t> decrypt_bfv(std::size_t ctx, std::size_t sk, std::size_t ct, std::uint32_t len) {
    return guard<rust::Vec<std::int64_t>>(
        [&]() {
            auto cc = Get<CryptoContextImpl<DCRTPoly>>(ctx);
            Plaintext result;
            cc->Decrypt(Get<PrivateKeyImpl<DCRTPoly>>(sk), Get<CiphertextImpl<DCRTPoly>>(ct), &result);
            if (result == nullptr)
                throw std::runtime_error("decryption returned null plaintext");
            if (len > 0)
                result->SetLength(len);
            const auto& values = result->GetPackedValue();
            rust::Vec<std::int64_t> out;
            out.reserve(values.size());
            for (auto v : values)
                out.push_back(v);
            return out;
        },
        rust::Vec<std::int64_t>());
}

// --- evaluation ------------------------------------------------------------

std::size_t eval_add(std::size_t ctx, std::size_t a, std::size_t b) {
    return guard<std::size_t>(
        [&]() { return Store(Get<CryptoContextImpl<DCRTPoly>>(ctx)->EvalAdd(Get<CiphertextImpl<DCRTPoly>>(a),
                                                                             Get<CiphertextImpl<DCRTPoly>>(b))); },
        0);
}

std::size_t eval_mult(std::size_t ctx, std::size_t a, std::size_t b) {
    return guard<std::size_t>(
        [&]() { return Store(Get<CryptoContextImpl<DCRTPoly>>(ctx)->EvalMult(Get<CiphertextImpl<DCRTPoly>>(a),
                                                                             Get<CiphertextImpl<DCRTPoly>>(b))); },
        0);
}

std::size_t eval_mult_plain(std::size_t ctx, std::size_t a, std::size_t pt) {
    return guard<std::size_t>(
        [&]() { return Store(Get<CryptoContextImpl<DCRTPoly>>(ctx)->EvalMult(
            Get<CiphertextImpl<DCRTPoly>>(a), Get<PlaintextImpl>(pt))); },
        0);
}

std::size_t eval_add_plain(std::size_t ctx, std::size_t a, std::size_t pt) {
    return guard<std::size_t>(
        [&]() {
            auto p = Get<PlaintextImpl>(pt);
            return Store(Get<CryptoContextImpl<DCRTPoly>>(ctx)->EvalAdd(
                Get<CiphertextImpl<DCRTPoly>>(a), p));
        },
        0);
}

std::size_t rescale(std::size_t ctx, std::size_t ct) {
    return guard<std::size_t>(
        [&]() { return Store(Get<CryptoContextImpl<DCRTPoly>>(ctx)->Rescale(Get<CiphertextImpl<DCRTPoly>>(ct))); }, 0);
}

// --- proxy re-encryption ---------------------------------------------------

std::size_t reencrypt(std::size_t ctx, std::size_t ek, std::size_t ct) {
    return guard<std::size_t>(
        [&]() {
            auto cc = Get<CryptoContextImpl<DCRTPoly>>(ctx);
            auto c2 = cc->ReEncrypt(Get<CiphertextImpl<DCRTPoly>>(ct), Get<EvalKeyImpl<DCRTPoly>>(ek));
            if (c2 == nullptr)
                throw std::runtime_error("re-encryption returned null ciphertext");
            return Store(c2);
        },
        0);
}

// --- serialization ---------------------------------------------------------

rust::Vec<std::uint8_t> ser_ct(std::size_t ctx, std::size_t ct) {
    (void)ctx;
    return guard<rust::Vec<std::uint8_t>>([&]() { return ToBytes(Get<CiphertextImpl<DCRTPoly>>(ct)); },
                                          rust::Vec<std::uint8_t>());
}

std::size_t deser_ct(std::size_t ctx, rust::Slice<const std::uint8_t> bytes) {
    (void)ctx;
    return guard<std::size_t>(
        [&]() {
            std::string s(reinterpret_cast<const char*>(bytes.data()), bytes.size());
            return FromBytes<Ciphertext<DCRTPoly>>(s);
        },
        0);
}

rust::Vec<std::uint8_t> ser_pk(std::size_t ctx, std::size_t pk) {
    (void)ctx;
    return guard<rust::Vec<std::uint8_t>>([&]() { return ToBytes(Get<PublicKeyImpl<DCRTPoly>>(pk)); },
                                          rust::Vec<std::uint8_t>());
}

std::size_t deser_pk(std::size_t ctx, rust::Slice<const std::uint8_t> bytes) {
    (void)ctx;
    return guard<std::size_t>(
        [&]() {
            std::string s(reinterpret_cast<const char*>(bytes.data()), bytes.size());
            return FromBytes<PublicKey<DCRTPoly>>(s);
        },
        0);
}

rust::Vec<std::uint8_t> ser_sk(std::size_t ctx, std::size_t sk) {
    (void)ctx;
    return guard<rust::Vec<std::uint8_t>>([&]() { return ToBytes(Get<PrivateKeyImpl<DCRTPoly>>(sk)); },
                                          rust::Vec<std::uint8_t>());
}

std::size_t deser_sk(std::size_t ctx, rust::Slice<const std::uint8_t> bytes) {
    (void)ctx;
    return guard<std::size_t>(
        [&]() {
            std::string s(reinterpret_cast<const char*>(bytes.data()), bytes.size());
            return FromBytes<PrivateKey<DCRTPoly>>(s);
        },
        0);
}

rust::Vec<std::uint8_t> ser_rekey(std::size_t ctx, std::size_t ek) {
    (void)ctx;
    return guard<rust::Vec<std::uint8_t>>([&]() { return ToBytes(Get<EvalKeyImpl<DCRTPoly>>(ek)); },
                                          rust::Vec<std::uint8_t>());
}

std::size_t deser_rekey(std::size_t ctx, rust::Slice<const std::uint8_t> bytes) {
    (void)ctx;
    return guard<std::size_t>(
        [&]() {
            std::string s(reinterpret_cast<const char*>(bytes.data()), bytes.size());
            return FromBytes<EvalKey<DCRTPoly>>(s);
        },
        0);
}

}  // namespace ffi
}  // namespace hepre