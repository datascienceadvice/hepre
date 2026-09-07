# HEPRE

Прототип системы для приватного скоринга риска диабета 2-го типа на гомоморфном шифровании (OpenFHE CKKS + BFV)
с прокси-перешифрованием (PRE). Математическое ядро — C++ (OpenFHE), обёртка для вычислений — Rust (cxx bridge),
см. `design.txt` (ТЗ и план статьи Q1–Q2).

## Структура
- `vendor/openfhe-development` — исходники OpenFHE 1.5.1 (commit `ed361af`) с локальными патчами (см. ниже).
- `vendor/openfhe-clang-build` / `openfhe-clang-debug-build` — Release/Debug сборки clang-cl.
- `vendor/openfhe-install` — установочный префикс (содержит Release-библиотеки; Debug для диагностики не установлен).
- `cpp/` — C++-шим (FFI-слой) `openfhe-shim.cc` / `openfhe-shim.h`.
- `src/` — Rust-крэйт `hepre`: `lib.rs` (публичный API, RAII), `ffi.rs` (cxx bridge),
  `scoring.rs` (этап A2: приватный скоринг).
- `build.rs` — сборка моста: компиляция shim через cxx-build + clang-cl и линковка статических библиотек OpenFHE.
- `tests/` — интеграционные тесты (`ckks.rs`, `bfv.rs`, `pre.rs`, `serialize.rs`, `score.rs`).
- `examples/bench.rs` — бенчмарк сквозного скоринга (этап A2).
- `examples/demo.rs` — воспроизводимый end-to-end пример использования (см. раздел «Пример использования»).
- `scripts/` — команды конфигурации/сборки OpenFHE и линковки smoke-тестов.

## Окружение (Windows)
- Visual Studio 2022 Community с компонентом **`Microsoft.VisualStudio.Component.VC.Llvm.Clang`**
  (clang-cl 19.1.5 + lld-link): `C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\Llvm\x64\`.
- CMake, NMake (из vcvars64.bat), Rust (MSVC toolchain `x86_64-pc-windows-msvc`).
- **MSVC (cl.exe) официально не поддерживается OpenFHE; используем clang-cl** (тот же MS ABI).
- OpenMP/NTL/GMP отключены: `-DWITH_OPENMP=OFF -DWITH_NTL=OFF`. Ключевая конфигурация:
  `HAVE_INT128=TRUE`, `NATIVEINT=64`, `MATHBACKEND=4` (uint64 + `__int128`).
- Для запуска собранных exe и тестов MSVC-рантайм должен быть доступен (см. «Запуск тестов»).

## Сборка OpenFHE
Конфигурация Release (из `cfg-openfhe-clang.cmd`):
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
Debug — то же с `-B vendor\openfhe-clang-debug-build`, `-DCMAKE_BUILD_TYPE=Debug`,
`-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreadedDebugDLL`.

Сборка/установка (обязательно из среды vcvars64):
```bat
cmake --build vendor\openfhe-clang-build         --config Release
cmake --install vendor\openfhe-clang-build       --config Release
```
Примечание: при пересборке команда `cmake --install` восстанавливает актуальное состояние префикса.
Debug-библиотеки (`vendor\openfhe-clang-debug-build\lib`) собираются только для диагностики и не устанавливаются.

## Вендорные патчи (vendor/openfhe-development, поверх commit ed361af)
| Файл | Изменение / причина |
|---|---|
| `CMakeLists.txt` | Ветка MSVC: `/DOPENFHE_VERSION /D_USE_MATH_DEFINES`; `/O2 /DNDEBUG` в Release вместо `-O0`; не добавлять `-Wno-unknown-pragmas` |
| `src/binfhe/lib/rgsw-acc.cpp` | `__builtin_ctz` для MSVC x64 через `_BitScanForward64` |
| `src/core/include/math/hal/intnat/ubintnat.h` | `Mul128` для MSVC x64 через `_umul128` (без `__int128`) |
| `src/core/include/math/math-hal.h` | Убран `#error "MSVC COMPILER IS NOT SUPPORTED"` |
| `src/core/include/math/hal/bigintfxd/ubintfxd.h`, `bigintntl/ubintntl.h` | `__attribute__((deprecated))` → `[[deprecated]]` в начале объявления (clang-cl не принимает хвостовую форму) |
| `src/pke/include/encoding/packedencoding.h`, `cryptocontext.h` | То же для `SetParams` и `ReKeyGen` |
| `src/core/lib/utils/demangle.cpp` | `cxxabi.h` только для `__GNUC__` или clang без `_MSC_VER` |
| `src/pke/include/cryptocontextfactory.h` | `template<> inline std::vector<...> AllContexts = {};` (а не неопределённая статическая переменная) |
| `src/pke/lib/cryptocontextfactory.cpp` | Вынос определения `AllContexts` (см. выше) |
| `src/pke/lib/scheme/ckksrns/ckksrns-parametergeneration.cpp` | `AUXMODSIZE = 60` только при `HAVE_INT128`, иначе `57` |
| `src/pke/lib/ciphertext-impl.cpp` | **Критично для CKKS.** `CiphertextImpl<DCRTPoly>::SetLevel` использует не-владеющий `std::dynamic_pointer_cast<const CryptoParametersCKKSRNS>` (восстановлен корректный код). Ранее в дереве находился вариант с вторым владеющим `shared_ptr` из сырого указателя — он преждевременно удалял `CryptoParametersCKKSRNS` и вызывал heap corruption (`0xC0000374`) в CKKS `Encrypt`. |

Патчи применяются к файлам исходников напрямую; отдельный diff/patch-файл не поддерживается (дерево уже изменено).

## Линковка потребителей (smoke-тесты и Rust-обёртка)
- Библиотеки статические: `OPENFHEpke_static.lib`, `OPENFHEcore_static.lib`, `OPENFHEbinfhe_static.lib` (префикс `lib/`).
- Обязательно подключить `clang_rt.builtins-x86_64.lib` (в `%LLVM%\lib\clang\19\lib\windows`)
  — иначе ошибки `__modti3`, `__floatuntidf`, `__fixunsdfti` (операции над `__int128`).
- CRT должен совпадать с библиотеками: Release → `/MD`, Debug → `/MDd` (иначе LNK2038).
- Пример линковки clang-cl — `scripts/build-clang.cmd`.

## Проверка работоспособности (этап A0, чистый C++)
Тесты собираются `scripts/build-clang.cmd <src> <exe>` и запускаются напрямую:
- `smoke-ckks-eval.cpp` — CKKS depth 2, batch 8192, Encrypt → EvalMult → Rescale → EvalAdd → Decrypt: ✅ SMOKE OK.
- `smoke-bfv.cpp` — BFV (integer), полный конвейер: ✅ SMOKE OK.
- Ключевые параметры тестов: `SetMultiplicativeDepth`, `SetScalingModSize(40)`, `SetBatchSize`.

См. `C:\Temp\opencode\smoke-*.cpp` (внешние временные копии; канонические версии — в `tests/` Rust-крэйта).

# Этап A1: Rust-обёртка `hepre`

## Архитектура
```
Rust (src/lib.rs)  ──►  cxx bridge (src/ffi.rs)  ──►  C++ shim (cpp/openfhe-shim.cc)  ──►  OpenFHE
   публичный API          #[cxx::bridge(namespace = "hepre::ffi")]   namespace hepre::ffi
   Result<T, Error>       функции объявлены как unsafe extern "C++"
```
- Мост собирается `build.rs` через `cxx_build::bridge("src/ffi.rs")` (cxx 1.0): компиляция shim
  clang-cl (`/MD /std:c++17 /EHsc /D_USE_MATH_DEFINES`) и линковка
  `OPENFHE{bfinfhe,core,pke}_static` + `clang_rt.builtins-x86_64`.
- Переопределяемые переменные окружения: `OPENFHE_HOME` (префикс OpenFHE),
  `CLANGCL` (путь к clang-cl.exe), `CLANG_RT_DIR` (каталог clang_rt.builtins).

### Ручки (RAII)
- Все объекты OpenFHE представлены хэндлом `usize` — указателем на выделенную в куче «коробку»
  `std::shared_ptr<void>`. Исходный делетер сохраняется в управляющем блоке `shared_ptr`,
  поэтому один `Drop` корректно удаляет любой тип.
- Типизированные обёртки: `Store<T>(shared_ptr<T>)` → `static_pointer_cast<void>`;
  `Get<T>(handle)` → обратный `static_pointer_cast`; для `KeyPair<DCRTPoly>` — `StoreValue`.
- В Rust типожизни через макрос `handle!` (`Context`, `KeyPair`, `PublicKey`, `SecretKey`, `EvalKey`,
  `Plaintext`, `Ciphertext`); освобождение — в `Drop`.

### Обработка ошибок (важно понимать при отладке)
- cxxbridge генерирует `noexcept`-обёртки **без try/catch**: C++-исключение, пересекающее границу,
  приводит к `std::terminate`/`abort` (симптом: `0xC0000409`, в отладчике стек оканчивается
  `ucrtbase!terminate` → `abort`). cxx **не** конвертирует исключения в Rust-паники автоматически.
- Поэтому каждая FFI-функция shim сама перехватывает исключения (шаблон `guard<>`):
  - успех: `guard` **очищает** `g_last_error` (thread_local) и возвращает результат;
  - провал: сообщение (`e.what()` или «unknown C++ exception») сохраняется в `g_last_error`,
    функция возвращает «нулевое» значение — `0` для хэндлов, пустой контейнер для `Vec`.
- Rust-сторона (`src/lib.rs`) проверяет сентинель (`ok_handle`/`ok_vec`) и при провале читает
  `ffi::ffi::take_last_error()` → `Err(Error)`.
- Следствие: дешифрование чужим ключом в CKKS больше **не убивает процесс** — OpenFHE кидает
  `CKKSPackedEncoding::Decode` «The decryption failed because the approximation error is too high...»
  (`ckkspackedencoding.cpp:452`), shim возвращает `Err` с этим текстом.

### Заголовки сериализации
- `ciphertext-ser.h`, `cryptocontext-ser.h`, `key-ser.h`, `bfvrns-ser.h`, `ckksrns-ser.h`, `rns-ser.h`
  используют **незаквалифицированный `DCRTPoly`**, поэтому подключаются строго **после**
  `using namespace lbcrypto;` (иначе `unknown type name 'DCRTPoly'`).

### PRE: обязательные параметры конфигурации
Решёточное прокси-перешифрование в OpenFHE требует **обеих** настроек (оба краха ниже были
воспроизведены и разобраны по стеку в этой работе):
1. `cc->Enable(PRE)` — иначе `ReKeyGen` бросает `VerifyPREEnabled`-исключение (необработанное →
   `abort`, `0xC0000409`). Флаги схемы, `pke/include/constants-defs.h:41-50`:
   `PKE=0x01, KEYSWITCH=0x02, PRE=0x04, LEVELEDSHE=0x08, ADVANCEDSHE=0x10, MULTIPARTY=0x20, FHE=0x40`.
2. `parameters.SetPREMode(INDCPA)` (или `NOISE_FLOODING_HRA`) — иначе `KeySwitchHYBRID` падает
   **неопределённым поведением** (AV `0xC0000005` в `ElemParams<...>::GetRingDimension`):
   публичный ключ остаётся в **Q-базисе**, а `KeySwitchHYBRID::KeySwitchGenInternal` (public-вариант,
   `keyswitch-hybrid.cpp:132+`) индексирует до `sizeQP`; `GetElementAtIndex` — это `operator[]`
   без проверок границ. `GetParamsPK()` отдаёт QP-базис только при
   `m_PREMode != NOT_SET` (`rns-cryptoparameters.h:265-271`).
   Официальные примеры всегда ставят `SetPREMode`: `pre-buffer.cpp`, `pre-hra-secure.cpp`.

Слой A1 жестко задаёт конфигурацию в `ctx_new_ckks` / `ctx_new_bfv` (`SetPREMode(INDCPA)`)
и включает в `EnableAll` набор `PKE | KEYSWITCH | PRE | LEVELEDSHE | ADVANCEDSHE`.

### Потокобезопасность
- OpenFHE держит **глобальные статические реестры** (реестр контекстов `CryptoContextFactory`
  и др.) без синхронизации: одновременное создание/уничтожение контекстов из разных потоков
  одного процесса порождает гонки (симптомы: `GetContextForPointer(): Cannot find context...`
  или зависание в параллельном прогоне тестов).
- Шим сериализует **все** входы в FFI единым глобальным `std::mutex` (макрос `LOCK()` и в
  `guard<>`, и в функциях `*_free` / void-операциях). Rust-приложение может использовать несколько
  потоков — вызовы к OpenFHE просто будут выстраиваться по очереди.

### Диагностика
- Debug-сборка (`vendor\openfhe-clang-debug-build`) полезна для стека, но clang-cl **игнорирует `/Zi`**
  (PDB не создаётся; отладочная информация в объектах отсутствует), поэтому имена функций берутся
  из **публичных символов** exe (наши Rust-тесты линкуются с `/DEBUG` и экспортируют имена
  shim-функций как внешние символы).
- Рабочий цикл отладки (cdb, символы из symbol server):
  `srv*C:\Temp\opencode\symbols*https://msdl.microsoft.com/download/symbols`.

## Запуск тестов
MSVC-рантайм должен быть в `PATH` (иначе запуск падает с `0xC0000135`: не найдены
`MSVCP140.dll`, `VCRUNTIME140.dll`, `VCRUNTIME140_1.dll`). Пример:
```powershell
$rt = "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207\bin\Hostx64\x64"
$env:PATH = "$rt;$env:PATH"
cargo test
```
Ожидаемый результат — **14/14 тестов зелёные** (стабильно и при параллельном прогоне, благодаря мьютексу):
| Тест | Кол-во | Что проверяет |
|---|---|---|
| `tests/ckks.rs` | 4 | roundtrip; EvalMult→Rescale→EvalAdd; `ring_dimension()==16384` (batch 8192 ⇒ N=2N_slots); публичный ключ после сериализации |
| `tests/bfv.rs` | 2 | roundtrip + EvalMult/EvalAdd (по модулю t); сериализация секретного ключа |
| `tests/pre.rs` | 2 | `rekeygen` + `reencrypt` → получатель расшифровывает; сериализация rekey-ключа |
| `tests/serialize.rs` | 3 | roundtrip шифротекста; roundtrip секретного ключа; **wrong key** — `Err` либо мусор, но не исходный текст |
| `tests/score.rs` | 3 | сквозной скоринг лаба→сервер→PRE→больница; чужой ключ; 200 пациентов в одном пакете |

## Известные ограничения прототипа
- Один глобальный мьютекс — полная сериализация всех OpenFHE-вызовов (цена потокобезопасности).
- Сообщение об ошибке — thread_local; «съедается» чтением, но прототипное решение (без очередей).
- Нет `panic=abort`-совместимости: ошибки возвращаются как `Result`, а не паники.

# Этап A2: сквозной приватный скоринг

## Сценарий (роли)
- **Лаборатория**: владеет фичами пациентов (`N` строк × `f` признаков), шифрует их.
- **Сервер**: гомоморфно считает непрерывный скор `score_i = Σ_j w_j·x_ij + b`
  (`eval_mult_plain` + `eval_add` + `eval_add_plain`) без `Rescale` (аддитивный контур),
  затем PRE-перешифровывает результат на ключ больницы. Не видит ни фичи, ни скор.
- **Больница**: расшифровывает скор своим ключом и применяет порог `threshold` в открытом виде.

## Пример использования (examples/demo.rs, end-to-end)
Воспроизводимый прогон всего конвейера на 5 «пациентах»: команда → вывод. Демо
показывает каждый шаг протокола и что видит каждая сторона.

1. **Терминал** (запуск в Release; MSVC-рантайм должен быть в `PATH`, см. «Запуск тестов»):
```powershell
cargo run --release --example demo
```
2. **Получаем** пошаговый вывод (скоры и вердикты стабильны; `max|err|` — примерный,
   порядок ~10⁻⁹, т.к. шум CKKS случаен):
```
=== Приватный скоринг риска диабета 2-го типа (CKKS + PRE) ===
Параметры CKKS: batch = 8192 слотов, multiplicative_depth = 3, scaling = 40 бит.

--- 1. Лаборатория: ключи и шифрование фичей пациентов ---
   5 пациента(ов), 5 признака(ов) на пациента
   Зашифровано 5 шифротекстов (признак -> шифротекст, пациент -> слот).
   Суммарный размер зашифрованных фичей: 6562335 байт.
   Сервер видит только шифротексты — значения фичей недоступны.
--- 2. Сервер: гомоморфный расчёт скора параллельно для всех пациентов ---
   Скор: score_i = Σ_j w_j·x_ij + b  (EvalMultPlain x5 + EvalAdd)
   Результат лежит в шифротексте — сервер не знает самих скоров.
--- 3. Больница: ключи и ключ перешифрования (PRE) ---
   Больница генерирует свою пару ключей.
   Лаборатория формирует ReKeyGen(sk_lab -> pk_hospital).
--- 4. Сервер: прокси-перешифрование скора под ключ больницы ---
   ReEncrypt выполнен: расшифровать скор теперь может только больница.
--- 5. Больница: расшифровка скора и применение порога в открытом виде ---

   Пац.      Скор (HE) Скор (откр.)   max|err|   Риск >= -1.45?
   P1          -1.5090      -1.5090   2.9e-10   нет
   P2          -1.2680      -1.2680    1.4e-9   ДА
   P3          -1.6390      -1.6390    1.7e-9   нет
   P4          -1.2400      -1.2400    2.2e-9   ДА
   P5          -1.5110      -1.5110    3.0e-9   нет

   Итог: max|HE скор - открытый скор| = 3.0e-9
   Несовпадений бинарного порога: 0 (ожидается 0).
--- 6. Проверка: чужой ключ (сервера) не расшифровывает PRE-скор ---
   чужой ключ -> Err (fail-closed Decode) — недоступно. OK

Готово: сквозной приватный скоринг лаборатория -> сервер -> больница.
```
3. **Затем** эти же шаги можно посчитать на больших когортах бенчмарком
   `cargo run --release --example bench` (N = 10/100/1000/10000, см. ниже).

## Модуль `scoring`
- `ScoringModel { weights, intercept, threshold }` с открытой проверкой
  `score_plain` / `predict_plain` (для сверки точности в тестах и бенчмарке).
- `encrypt_features(cc, pk, model, &[Vec<f64>])` — SIMD-упаковка: пациент → слот,
  признак → отдельный шифротекст (`EvalMultPlain` множителем `w_j`, повторённым по слотам).
- `compute_score` — `Σ_j w_j·ct_j + b` одним шифротекстом на пакет слотов (batch 8192).
- `transfer_score(cc, rekey, score)` — `ReEncrypt` (PRE).
- `decrypt_scores(cc, sk, score, k)` — расшифровка первых `k` слотов больницей.

## Тесты A2 (`tests/score.rs`, 3 теста)
| Тест | Что проверяет |
|---|---|
| `end_to_end_score_and_pre_to_hospital` | лаба → сервер → PRE → больница: непрерывные скоры совпадают с `score_plain`; порог совпадает с `predict_plain` |
| `wrong_secret_key_cannot_read_transferred_score` | чужой ключ не даёт правдоподобного скора (мусор либо `Err` — fail-closed `Decode`) |
| `multiple_patients_in_one_batch` | 200 пациентов в одном пакете (слоты 0..200) |

## Бенчмарк
```powershell
$rt = "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207\bin\Hostx64\x64"
$env:PATH = "$rt;$env:PATH"
cargo run --release --example bench
```
Замеры `std::time` для N = 10 / 100 / 1000 / 10000 (чанки по 8192 слота; KeyGen/ReKeyGen
один раз на сборку). Пример результатов (Release, Windows x64):
```
KeyGen(lab) = 28.27 ms; ReKeyGen(lab_sk -> hosp_pk) = 37.89 ms.
N        Encrypt    Score      PRE    Decrypt |  max|err|  binMis
10      116.94ms   82.64ms   21.26ms   39.22ms |   4.17e-9       0
100     106.24ms   71.93ms   19.26ms   33.65ms |   5.12e-9       0
1000    112.54ms   74.90ms   18.75ms   39.34ms |   6.08e-9       0
10000   220.72ms  155.15ms   38.83ms   69.92ms |   8.55e-9       0
```
Примечания:
- До 8192 пациентов стоимость почти не зависит от N (одна SIMD-упаковка на признак);
  N=10000 укладывается в 2 пакета, поэтому время ~2×.
- `max|err|` — максимальная абсолютная ошибка непрерывного скора относительно открытой модели
  (< 10⁻⁶ с запасом, фактически ~10⁻⁹); `binMis` — расхождения бинарного порога (0 для всех размеров).