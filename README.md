# HEPRE

A research prototype for privacy-preserving type-2-diabetes risk scoring under
homomorphic encryption (OpenFHE CKKS + BFV) with proxy re-encryption (PRE). The
mathematical core is C++ (OpenFHE); the computation layer is a Rust wrapper built
on a `cxx` bridge. Design and study protocol: `docs/STUDY_PROTOCOL.md`.

> The role CLIs (`lab`/`server`/`hospital`) are the implemented code. The scenario
> evaluated by the accompanying manuscript is bilateral (laboratory ↔ external
> assessor, e.g. an insurer): the assessor's environment plays the computing role,
> and the laboratory performs the proxy re-encryption of the result
> (`docs/THREAT_MODEL.md`).

## Layout

- `vendor/openfhe-development` — OpenFHE 1.5.1 sources (commit `ed361af`) with local patches (see below).
- `vendor/openfhe-clang-build` / `openfhe-clang-debug-build` — Release/Debug clang-cl builds.
- `vendor/openfhe-install` — install prefix (Release libraries; the Debug build is diagnostic only and not installed).
- `cpp/` — C++ FFI shim `openfhe-shim.cc` / `openfhe-shim.h`.
- `src/` — the `hepre` Rust crate: `lib.rs` (public API, RAII), `ffi.rs` (cxx bridge),
  `scoring.rs` (stage A2: private scoring).
- `src/bin/` — per-role CLIs: `hospital`, `lab`, `server` (see “Usage example”);
  `src/demo_common.rs` — shared module (handoff file format, inline JSON).
- `build.rs` — bridge build: compiles the shim via cxx-build + clang-cl and links the static OpenFHE libraries.
- `tests/` — integration tests (`ckks.rs`, `bfv.rs`, `pre.rs`, `serialize.rs`, `score.rs`,
  `roles_handoff.rs` — a cross-process scenario).
- `examples/bench.rs` — end-to-end scoring benchmark (stage A2).
- `scripts/` — OpenFHE configuration/build commands and smoke-test linking.

## Research layer (publication)

A complete reproducible study pipeline is built on top of the cryptographic
prototype: a synthetic cohort with a documented ground truth, a reference logistic
model, a homomorphic logistic-regression circuit, and the statistical analysis.
Everything is computed **inside this repository**, without external statistical
packages.

```powershell
# one command: build, tests, and the full study run
pwsh scripts\repro.ps1
```

Or step by step:

```powershell
cargo build --release --bins
.\target\release\study.exe --write-default-config configs\study.json   # regenerate the configuration
.\target\release\study.exe --config configs\study.json                 # run the protocol

# reproducibility check: compare two runs by the exact-invariant list
python scripts\compare_runs.py results\20260925-042702_4659cac3 results\20261012-083347_4659cac3

# structural check of CITATION.cff and ci.yml (no external dependencies)
python scripts\validate_metadata.py
```

The repository ships the study runs. The canonical one is
`results/20261013-045947_4659cac3/` (the manuscript cites its numbers);
`results/20261014-132941_acd70ff4/` is an **appendix run** that extends the
scalability test to 120,000 profiles (`configs/study_scale_120k.json`; the rest of
the protocol is unchanged), and `results/20261014-131713_acd70ff4/` carries the
per-patient `probabilities.csv` artifact (open vs encrypted probability). The pair
`results/20260925-042702_4659cac3/` and `results/20261012-083347_4659cac3/` is the
reproducibility check run by the command above: every exact invariant defined by the
script matches, including the `cohort.csv` digest; only CKKS-noise quantities and
timings differ. Runs from `20261013-*` onwards also contain the sensitivity control
of the disagreement metric (`negative_control` in `results.json`). Details:
`docs/REPRODUCIBILITY.md`, §3.1.

Every run is written to `results/<run-id>/` together with a SHA-256 manifest and an
environment fingerprint. Artifact reference:

| Module | Purpose |
|---|---|
| `src/cohort.rs` | synthetic cohort: DGP with an explicit `η`, prevalence calibrated by bisection, train/validation split |
| `src/rng.rs`, `src/detmath.rs` | deterministic PRNG and elementary math without the platform `libm` |
| `src/logistic.rs` | reference logistic regression (IRLS + L2, standardization) |
| `src/poly.rs` | odd polynomial approximation of the sigmoid and a **verified decision-sign preservation** check |
| `src/he.rs` | homomorphic circuit: logit, polynomial sigmoid, PRE |
| `src/stats.rs` | AUC/DeLong, Wilson, Clopper–Pearson, kappa, McNemar, Brier, calibration, bootstrap |
| `src/study.rs`, `src/bin/study.rs` | the whole protocol and its CLI; sensitivity control (`negative_control`) |
| `src/hash.rs`, `src/env.rs` | SHA-256 and the environment fingerprint for reproducibility |

Documentation: `docs/STUDY_PROTOCOL.md` (design and statistical plan),
`docs/DATA_FLOW.md` (data flow, Figure 1, and a trace of a single value),
`docs/THREAT_MODEL.md`, `docs/DATA_DICTIONARY.md`, `docs/REPRODUCIBILITY.md`,
`docs/REPORTING_GUIDELINES.md`. Technical details of the protocol that accompany
the manuscript are collected in `appendix_A.md`.

## License and citation

The project is distributed under the **BSD-2-Clause** license (`LICENSE`). If you
use HEPRE in academic work, please cite it via `CITATION.cff` (on GitHub, the
“Cite this repository” button). This is a **research prototype**: do not use it
with real patient data and do not apply it to clinical decisions (`SECURITY.md`).

**Main methodological result.** The sigmoid approximation is odd (hence exactly
`p(0) = 1/2`) and is verified to **preserve the sign relative to the decision
threshold**. These two properties imply that the binary classification is identical
to the plaintext rule `η ≥ 0` for any approximation error: the error shifts only the
reported probability, never the decision. Monotonicity and non-negativity of the
coefficients are **measured, not enforced** — on a wide interval the sigmoid is
almost a step, and forcing non-negativity raises the probability error from
4.1·10⁻³ to ≈10⁻¹.

## Environment (Windows)

- Visual Studio 2022 Community with the **`Microsoft.VisualStudio.Component.VC.Llvm.Clang`**
  component (clang-cl 19.1.5 + lld-link): `C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\Llvm\x64\`.
- CMake, NMake (from `vcvars64.bat`), Rust (MSVC toolchain `x86_64-pc-windows-msvc`).
- **MSVC (`cl.exe`) is officially unsupported by OpenFHE; we use clang-cl** (same MS ABI).
- OpenMP/NTL/GMP are disabled: `-DWITH_OPENMP=OFF -DWITH_NTL=OFF`. Key configuration:
  `HAVE_INT128=TRUE`, `NATIVEINT=64`, `MATHBACKEND=4` (uint64 + `__int128`).
- The MSVC runtime must be available when running the built binaries and tests (see “Running the tests”).

## Building OpenFHE

Release configuration (from `cfg-openfhe-clang.cmd`):

```bat
call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
cmake -S vendor\openfhe-development -B vendor\openfhe-clang-build -G "NMake Makefiles" ^
  -DCMAKE_C_COMPILER=clang-cl -DCMAKE_CXX_COMPILER=clang-cl ^
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreadedDLL ^
  -DCMAKE_SYSTEM_VERSION=10.0.26100.0 ^
  -DBUILD_STATIC=ON -DBUILD_SHARED=OFF -DBUILD_UNITTESTS=OFF -DBUILD_EXAMPLES=OFF -DBUILD_BENCHMARKS=OFF ^
  -DWITH_OPENMP=OFF -DWITH_NTL=OFF -DBUILD_EXTRAS=OFF ^
  -DCMAKE_INSTALL_PREFIX=vendor\openfhe-install
```

Debug is the same with `-B vendor\openfhe-clang-debug-build`, `-DCMAKE_BUILD_TYPE=Debug`,
`-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreadedDebugDLL`.

Build/install (must run from the `vcvars64` environment):

```bat
cmake --build vendor\openfhe-clang-build         --config Release
cmake --install vendor\openfhe-clang-build       --config Release
```

Note: on rebuild, `cmake --install` restores the current state of the prefix. Debug
libraries (`vendor\openfhe-clang-debug-build\lib`) are built for diagnostics only and
are not installed.

## Vendored patches (`vendor/openfhe-development`, on top of commit `ed361af`)

| File | Change / reason |
|---|---|
| `CMakeLists.txt` | MSVC branch: `/DOPENFHE_VERSION /D_USE_MATH_DEFINES`; `/O2 /DNDEBUG` in Release instead of `-O0`; do not add `-Wno-unknown-pragmas` |
| `src/binfhe/lib/rgsw-acc.cpp` | `__builtin_ctz` for MSVC x64 via `_BitScanForward64` |
| `src/core/include/math/hal/intnat/ubintnat.h` | `Mul128` for MSVC x64 via `_umul128` (no `__int128`) |
| `src/core/include/math/math-hal.h` | Removed `#error "MSVC COMPILER IS NOT SUPPORTED"` |
| `src/core/include/math/hal/bigintfxd/ubintfxd.h`, `bigintntl/ubintntl.h` | `__attribute__((deprecated))` → `[[deprecated]]` at the beginning of the declaration (clang-cl rejects the trailing form) |
| `src/pke/include/encoding/packedencoding.h`, `cryptocontext.h` | Same for `SetParams` and `ReKeyGen` |
| `src/core/lib/utils/demangle.cpp` | `cxxabi.h` only for `__GNUC__` or clang without `_MSC_VER` |
| `src/pke/include/cryptocontextfactory.h` | `template<> inline std::vector<...> AllContexts = {};` (instead of an undefined static variable) |
| `src/pke/lib/cryptocontextfactory.cpp` | Moved the `AllContexts` definition (see above) |
| `src/pke/lib/scheme/ckksrns/ckksrns-parametergeneration.cpp` | `AUXMODSIZE = 60` only with `HAVE_INT128`, otherwise `57` |
| `src/pke/lib/ciphertext-impl.cpp` | **Critical for CKKS.** `CiphertextImpl<DCRTPoly>::SetLevel` uses a non-owning `std::dynamic_pointer_cast<const CryptoParametersCKKSRNS>` (the correct code restored). The tree previously contained a variant with a second owning `shared_ptr` built from a raw pointer — it destroyed `CryptoParametersCKKSRNS` prematurely and caused heap corruption (`0xC0000374`) in CKKS `Encrypt`. |

The patches are applied to the source files directly. The full set is also kept in
`patches/openfhe-1.5.1-msvc.patch` (base revision `ed361af`) and is applied
automatically by `scripts/setup-openfhe.ps1` on a clean build. To verify that the
patch reproduces the current tree exactly (a zero exit code means it applies
cleanly):

```powershell
git -C vendor\openfhe-development apply --reverse --check patches\openfhe-1.5.1-msvc.patch
```

## Linking consumers (smoke tests and the Rust wrapper)

- The libraries are static: `OPENFHEpke_static.lib`, `OPENFHEcore_static.lib`, `OPENFHEbinfhe_static.lib` (under `lib/`).
- `clang_rt.builtins-x86_64.lib` must be linked (in `%LLVM%\lib\clang\19\lib\windows`) —
  otherwise you get `__modti3`, `__floatuntidf`, `__fixunsdfti` errors (`__int128` operations).
- The CRT must match the libraries: Release → `/MD`, Debug → `/MDd` (otherwise LNK2038).
- An example of clang-cl linking is `scripts/build-clang.cmd`.

## Sanity check (stage A0, pure C++)

Tests are built with `scripts/build-clang.cmd <src> <exe>` and run directly:

- `smoke-ckks-eval.cpp` — CKKS depth 2, batch 8192, Encrypt → EvalMult → Rescale → EvalAdd → Decrypt: ✅ SMOKE OK.
- `smoke-bfv.cpp` — BFV (integer), full pipeline: ✅ SMOKE OK.
- Key test parameters: `SetMultiplicativeDepth`, `SetScalingModSize(40)`, `SetBatchSize`.

See the external temporary copies `smoke-*.cpp` (the canonical versions live in the
Rust crate's `tests/`).

# Stage A1: the `hepre` Rust wrapper

## Architecture

```
Rust (src/lib.rs)  ──►  cxx bridge (src/ffi.rs)  ──►  C++ shim (cpp/openfhe-shim.cc)  ──►  OpenFHE
   public API            #[cxx::bridge(namespace = "hepre::ffi")]   namespace hepre::ffi
   Result<T, Error>      functions declared as unsafe extern "C++"
```

- The bridge is built by `build.rs` via `cxx_build::bridge("src/ffi.rs")` (cxx 1.0): it compiles the
  shim with clang-cl (`/MD /std:c++17 /EHsc /D_USE_MATH_DEFINES`) and links
  `OPENFHE{bfinfhe,core,pke}_static` + `clang_rt.builtins-x86_64`.
- Overridable environment variables: `OPENFHE_HOME` (OpenFHE prefix),
  `CLANGCL` (path to clang-cl.exe), `CLANG_RT_DIR` (the clang_rt.builtins directory).

### Handles (RAII)

- Every OpenFHE object is represented by a `usize` handle — a pointer to a heap-allocated “box”
  `std::shared_ptr<void>`. The original deleter is preserved in the `shared_ptr` control block,
  so a single `Drop` correctly destroys any type.
- Typed wrappers: `Store<T>(shared_ptr<T>)` → `static_pointer_cast<void>`;
  `Get<T>(handle)` → the reverse `static_pointer_cast`; for `KeyPair<DCRTPoly>` — `StoreValue`.
- Rust-side lifetimes via the `handle!` macro (`Context`, `KeyPair`, `PublicKey`, `SecretKey`, `EvalKey`,
  `Plaintext`, `Ciphertext`); release happens in `Drop`.

### Error handling (important when debugging)

- cxxbridge generates `noexcept` wrappers **without try/catch**: a C++ exception crossing the boundary
  leads to `std::terminate`/`abort` (symptom: `0xC0000409`, the debugger stack ending in
  `ucrtbase!terminate` → `abort`). cxx does **not** convert exceptions into Rust panics automatically.
- Therefore every FFI shim function catches exceptions itself (the `guard<>` template):
  - success: `guard` **clears** `g_last_error` (thread_local) and returns the result;
  - failure: the message (`e.what()` or “unknown C++ exception”) is stored in `g_last_error`,
    and the function returns a “zero” value — `0` for handles, an empty container for `Vec`.
- The Rust side (`src/lib.rs`) checks the sentinel (`ok_handle`/`ok_vec`) and on failure reads
  `ffi::ffi::take_last_error()` → `Err(Error)`.
- Consequence: decrypting with a wrong key in CKKS no longer **kills the process** — OpenFHE throws
  `CKKSPackedEncoding::Decode` “The decryption failed because the approximation error is too high...”
  (`ckkspackedencoding.cpp:452`), and the shim returns an `Err` carrying that text.

### Serialization headers

- `ciphertext-ser.h`, `cryptocontext-ser.h`, `key-ser.h`, `bfvrns-ser.h`, `ckksrns-ser.h`, `rns-ser.h`
  use an **unqualified `DCRTPoly`**, so they must be included strictly **after**
  `using namespace lbcrypto;` (otherwise: `unknown type name 'DCRTPoly'`).

### PRE: required configuration parameters

Lattice-based proxy re-encryption in OpenFHE requires **both** settings (both crashes below were
reproduced and analysed from stacks in this work):

1. `cc->Enable(PRE)` — otherwise `ReKeyGen` throws a `VerifyPREEnabled` exception (unhandled →
   `abort`, `0xC0000409`). Scheme flags, `pke/include/constants-defs.h:41-50`:
   `PKE=0x01, KEYSWITCH=0x02, PRE=0x04, LEVELEDSHE=0x08, ADVANCEDSHE=0x10, MULTIPARTY=0x20, FHE=0x40`.
2. `parameters.SetPREMode(INDCPA)` — otherwise `KeySwitchHYBRID` fails with
   **undefined behaviour** (access violation `0xC0000005` in `ElemParams<...>::GetRingDimension`):
   the public key stays in the **Q basis**, while `KeySwitchHYBRID::KeySwitchGenInternal` (the public
   variant, `keyswitch-hybrid.cpp:132+`) indexes up to `sizeQP`; `GetElementAtIndex` is an unchecked
   `operator[]`. `GetParamsPK()` returns the QP basis only when
   `m_PREMode != NOT_SET` (`rns-cryptoparameters.h:265-271`).
   The official examples always set `SetPREMode`: `pre-buffer.cpp`, `pre-hra-secure.cpp`.

Stage A1 hard-codes the configuration in `ctx_new_ckks` / `ctx_new_bfv` (`SetPREMode(INDCPA)`) and
includes `PKE | KEYSWITCH | PRE | LEVELEDSHE | ADVANCEDSHE` in `EnableAll`.

### Thread safety

- OpenFHE keeps **global static registries** (the `CryptoContextFactory` context registry and others)
  without synchronization: creating/destroying contexts concurrently from different threads of one
  process causes races (symptoms: `GetContextForPointer(): Cannot find context...` or a hang in a
  parallel test run).
- The shim serializes **all** FFI entry points with a single global `std::mutex` (the `LOCK()` macro in
  `guard<>` and in the `*_free` / void operations). A Rust application may use several threads — the
  OpenFHE calls simply queue up.

### Diagnostics

- The Debug build (`vendor\openfhe-clang-debug-build`) is useful for stacks, but clang-cl **ignores `/Zi`**
  (no PDB is produced; no debug info in the objects), so function names come from the **public symbols**
  of the exe (our Rust tests link with `/DEBUG` and export the shim function names as external symbols).
- A working debugging loop (cdb, symbols from a symbol server):
  `srv*<local-symbol-cache>*https://msdl.microsoft.com/download/symbols`.

## Running the tests

The MSVC runtime must be on `PATH` (otherwise the run fails with `0xC0000135`: `MSVCP140.dll`,
`VCRUNTIME140.dll`, `VCRUNTIME140_1.dll` not found). Example:

```powershell
$rt = "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207\bin\Hostx64\x64"
$env:PATH = "$rt;$env:PATH"
cargo test
```

Expected result — **15/15 tests green** (stable also in a parallel run, thanks to the mutex):

| Test | Count | What it checks |
|---|---|---|
| `tests/ckks.rs` | 4 | roundtrip; EvalMult→Rescale→EvalAdd; `ring_dimension()==16384` (batch 8192 ⇒ N=2N_slots); public key after serialization |
| `tests/bfv.rs` | 2 | roundtrip + EvalMult/EvalAdd (modulo t); secret-key serialization |
| `tests/pre.rs` | 2 | `rekeygen` + `reencrypt` → the recipient decrypts; rekey-key serialization |
| `tests/serialize.rs` | 3 | ciphertext roundtrip; secret-key roundtrip; **wrong key** — `Err` or garbage, but never the original plaintext |
| `tests/score.rs` | 3 | end-to-end scoring lab→server→PRE→hospital; wrong key; 200 patients in one batch |
| `tests/roles_handoff.rs` | 1 | **cross-process** run of the `lab`/`server`/`hospital` utilities through files: keys and ciphertexts serialize correctly between processes |

## Known limitations of the prototype

- A single global mutex — full serialization of all OpenFHE calls (the price of thread safety).
- The error message is thread_local; it is “consumed” by reading it, which is the prototype solution (no queues).
- No `panic=abort` compatibility: errors are returned as `Result`, not panics.

# Stage A2: end-to-end private scoring

## Scenario (roles)

- **Laboratory**: owns the patient features (`N` rows × `f` features) and encrypts them.
- **Server**: evaluates the continuous score `score_i = Σ_j w_j·x_ij + b` homomorphically
  (`eval_mult_plain` + `eval_add` + `eval_add_plain`) without `Rescale` (an additive circuit),
  then proxy-re-encrypts the result to the hospital key. It sees neither the features nor the score.
- **Hospital**: decrypts the score with its own key and applies the `threshold` in the clear.

## Usage example (`src/bin`: hospital, lab, server)

Three separate role CLIs — one per role — exchange only files in the `handoff/` directory
(keys and ciphertexts, OpenFHE binary format). No role holds another role's secrets: the server has
no decryption keys and sees only ciphertexts.

The dataset (JSON) and the model (public) are the same as in the A2 tests:
`[hba1c, glucose, bmi, age, family]`, threshold `-1.45`.

The `--patients`/`--model` parameters accept **either** a path to a JSON file (most reliable)
**or** an inline JSON string. Files are recommended: shell quoting never enters the JSON, and the
parser skips a BOM (PowerShell 5.1 `Set-Content`).

**Preparation** (from the repository root; MSVC runtime on `PATH`, see “Running the tests”):

```powershell
cargo build --release --bins

# JSON into files, once (by hand or in an editor; each command's payload is a single line):
Set-Content handoff\patients.json -Value '[{"name":"P1","features":[0.50,0.40,0.20,0.30,0.00]},{"name":"P2","features":[0.80,0.70,0.50,0.60,1.00]},{"name":"P3","features":[0.10,0.20,0.10,0.10,0.00]},{"name":"P4","features":[0.70,0.90,0.70,0.80,1.00]},{"name":"P5","features":[0.30,0.60,0.30,0.50,0.00]}]'
Set-Content handoff\model.json -Value '{"weights":[0.22,0.14,0.08,0.03,0.10],"intercept":-1.70,"threshold":-1.45}'
```

**1. The hospital generates keys** (`handoff/hosp.pk.bin` → to the laboratory; `hosp.sk.bin` stays with the hospital):

```powershell
cargo run --release --bin hospital -- keygen
```

```
--- Hospital: key generation ---
   Public key  : handoff\hosp.pk.bin (2049.9 KB)
   Secret key  : handoff\hosp.sk.bin (641.3 KB)  — kept by the hospital only
   The public key is passed to the laboratory for ReKeyGen.
```

**2. The laboratory encrypts the patient features and produces the re-encryption key:**

```powershell
cargo run --release --bin lab -- encrypt --patients handoff\patients.json --model handoff\model.json --hospital-pk handoff\hosp.pk.bin
```

```
--- Laboratory: encrypting patient features ---
   Patients: 5, features per patient: 5
   Ciphertexts produced: 5 (feature -> ciphertext, patient -> slot; batches of 8192).
   features.cts.bin : handoff\features.cts.bin (6408.5 KB)
   rekey.ek.bin     : handoff\rekey.ek.bin (4098.8 KB)
   The server receives ONLY these files: no patient features and no decryption keys.
```

**3. The server evaluates the score homomorphically and re-encrypts it to the hospital key (PRE):**

```powershell
cargo run --release --bin server -- score --model handoff\model.json
```

```
--- Server: homomorphic score evaluation and PRE ---
   Patients: 5, batches: 1 (slots per batch: 8192).
   score = Σ_j w_j·x_j + b: EvalMultPlain + EvalAdd on ciphertexts; result -> ReEncrypt (PRE).
   Time: 277.6 ms.
   features.cts.bin read : handoff\features.cts.bin (6408.5 KB)
   rekey.ek.bin    read  : handoff\rekey.ek.bin (4098.8 KB)
   scores.ct.bin   written: handoff\scores.ct.bin (1025.7 KB)
   The server holds no decryption keys and never saw the patient features in the clear.
```

**4. The hospital decrypts the scores and applies the threshold in the clear**
(`--patients` is only for the demonstrative accuracy check; in the real protocol the hospital never
sees the patient features):

```powershell
cargo run --release --bin hospital -- decrypt --model handoff\model.json --patients handoff\patients.json
```

```
--- Hospital: score decryption and risk threshold ---

   Pat.      Score (HE)  Risk >= -1.45?
   P1          -1.5090              no
   P2          -1.2680              YES
   P3          -1.6390              no
   P4          -1.2400              YES
   P5          -1.5110              no

   Total: patients = 5, at risk = 2
   Check (demonstrative): max|HE score - plaintext score| = 4.2e-9; threshold mismatches: 0
   Check passed: max|err| < 1e-2, no threshold mismatches.
```

**Alternative: inline JSON.** `--patients`/`--model` can be passed as strings.
Important: the `$patients` and `$model` assignments must be on **separate lines**
(do not concatenate them — PowerShell would merge them into one broken literal), and the values
themselves should preferably be single-line:

```powershell
$patients = '[{"name":"P1","features":[0.50,0.40,0.20,0.30,0.00]},{"name":"P2","features":[0.80,0.70,0.50,0.60,1.00]},{"name":"P3","features":[0.10,0.20,0.10,0.10,0.00]},{"name":"P4","features":[0.70,0.90,0.70,0.80,1.00]},{"name":"P5","features":[0.30,0.60,0.30,0.50,0.00]}]'
$model = '{"weights":[0.22,0.14,0.08,0.03,0.10],"intercept":-1.70,"threshold":-1.45}'
```

and use `--patients $patients --model $model` instead of file paths — the output is the same.

The scores and verdicts are stable; `max|err|` is about 10⁻⁹ (CKKS noise is random).
The `handoff/` directory is generated and is not part of the repository. For large cohorts use the
benchmark `cargo run --release --example bench` (N = 10/100/1000/10000, see below).

## The `scoring` module

- `ScoringModel { weights, intercept, threshold }` with an open check
  `score_plain` / `predict_plain` (for accuracy comparison in tests and the benchmark).
- `encrypt_features(cc, pk, model, &[Vec<f64>])` — SIMD packing: patient → slot,
  feature → a separate ciphertext (`EvalMultPlain` by the weight `w_j` replicated across slots).
- `compute_score` — `Σ_j w_j·ct_j + b` as a single ciphertext per batch of slots (batch 8192).
- `transfer_score(cc, rekey, score)` — `ReEncrypt` (PRE).
- `decrypt_scores(cc, sk, score, k)` — decrypts the first `k` slots as the hospital.

## A2 tests (`tests/score.rs`, 3 tests)

| Test | What it checks |
|---|---|
| `end_to_end_score_and_pre_to_hospital` | lab → server → PRE → hospital: continuous scores match `score_plain`; the threshold matches `predict_plain` |
| `wrong_secret_key_cannot_read_transferred_score` | a wrong key yields no plausible score (garbage or `Err` — fail-closed `Decode`) |
| `multiple_patients_in_one_batch` | 200 patients in one batch (slots 0..200) |

## Benchmark

```powershell
$rt = "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207\bin\Hostx64\x64"
$env:PATH = "$rt;$env:PATH"
cargo run --release --example bench
```

`std::time` measurements for N = 10 / 100 / 1000 / 10000 (chunks of 8192 slots; KeyGen/ReKeyGen are
done once per build). Example results (Release, Windows x64):

```
KeyGen(lab) = 28.27 ms; ReKeyGen(lab_sk -> hosp_pk) = 37.89 ms.
N        Encrypt    Score      PRE    Decrypt |  max|err|  binMis
10      116.94ms   82.64ms   21.26ms   39.22ms |   4.17e-9       0
100     106.24ms   71.93ms   19.26ms   33.65ms |   5.12e-9       0
1000    112.54ms   74.90ms   18.75ms   39.34ms |   6.08e-9       0
10000   220.72ms  155.15ms   38.83ms   69.92ms |   8.55e-9       0
```

Notes:

- Up to 8192 patients the cost is almost independent of N (one SIMD packing per feature);
  N = 10000 needs 2 batches, hence ≈2× the time.
- `max|err|` is the maximum absolute error of the continuous score against the plaintext model
  (generously below 10⁻⁶, in practice ~10⁻⁹); `binMis` is the number of binary-threshold
  discrepancies (0 for all sizes).
