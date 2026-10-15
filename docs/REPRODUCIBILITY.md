# Воспроизводимость (Reproducibility) — HEPRE

Документ описывает, как повторить сборку и получить **все** числа, о которых отчитывается проект.
Всё, что уже опубликовано, снабжено ссылкой на источник; числа, которые ещё не получены, здесь
**не выдумываются** — указано, в каком артефакте они появятся (`results/<run-id>/…`).

> **Состояние дерева (обновлено после первого полного прогона).** Всё перечисленное ниже уже
> в дереве и проверено: скрипты `scripts/setup-openfhe.ps1` и `scripts/repro.ps1`, `Makefile`,
> CI-workflow, раннер исследования (`src/study.rs`, CLI `src/bin/study.rs`), гомоморфный контур
> логистической регрессии (`src/he.rs`), отпечаток окружения (`src/env.rs`) и модули
> детерминированной генерации/статистики (`src/rng.rs`, `src/detmath.rs`, `src/hash.rs`,
> `src/cohort.rs`, `src/stats.rs`, `src/logistic.rs`, `src/poly.rs`), вендорный патч
> `patches/openfhe-1.5.1-msvc.patch`, конфигурация `configs/study.json` и каталог `results/`
> с эталонным прогоном.
>
> **Проверено прогоном.** Модульные тесты библиотеки — **71 из 71 зелёные**; интеграционные
> тесты A1/A2 — **15 из 15 зелёные**. Полный прогон исследования выполнен в release-профиле и
> дал каталог `results/20261015-052701_c4a6a8f1/` со всеми числами, на которые ссылается
> рукопись (эталонный прогон; контроль чувствительности метрики — `negative_control`);
> повторная генерация когорты, модели и многочлена совпала **побитово**.
>
> Модульные тесты новых модулей находятся **внутри самих модулей** (блоки `#[cfg(test)] mod tests`),
> а не в отдельных файлах `tests/*.rs`: так они имеют доступ к приватным функциям
> (например, к решателю линейных систем и к проверкам монотонности). Интеграционные тесты
> `tests/*.rs` по-прежнему проверяют публичный API.

---

## 1. Что именно воспроизводится

| Класс | Воспроизводимость | Где фиксируется |
|---|---|---|
| Сборка (OpenFHE 1.5.1 + шим + Rust-крэйт) | Полная при совпадении версий инструментов (§2) | §3–§5 |
| Корректность (15 интеграционных тестов) | Полная | §7, шаг 9 |
| Синтетическая когорта и модель | **Бит-в-бит** при фиксированном seed (`CohortConfig::seed`) | `results/<run-id>/cohort.csv`, `model.json` |
| Числовые результаты (скор, вероятность, ошибка, метрики) | Воспроизводимы в пределах шума CKKS и с точностью f64-арифметики | `results/<run-id>/results.json` |
| Шифротексты/ключи (побайтово) | **Не воспроизводимы** (PRNG OpenFHE) | — |
| Тайминги | **Не воспроизводимы абсолютно**; сравнимы только относительно | `results/<run-id>/results.json` + `environment.json` |

---

## 2. Требуемое программное обеспечение

| Компонент | Версия / требование | Где зафиксировано в репозитории |
|---|---|---|
| ОС | Windows x64 | `README.md` §«Окружение (Windows)» |
| Visual Studio 2022 Community | Компонент **`Microsoft.VisualStudio.Component.VC.Llvm.Clang`** → clang-cl **19.1.5** + lld-link; каталог `C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\Llvm\x64\` | `README.md` §«Окружение»; `build.rs` (значения по умолчанию `CLANGCL`, `CLANG_RT_DIR`) |
| MSVC runtime | Например, `…\VC\Tools\MSVC\14.44.35207\bin\Hostx64\x64` | `README.md` §«Запуск тестов» |
| CMake | ≥ 3.16.3 | `vendor/openfhe-development/CMakeLists.txt`: `cmake_minimum_required(VERSION 3.16.3)` |
| Генератор сборки | **NMake Makefiles**, запуск из среды `vcvars64.bat` | `README.md`; `scripts/cfg-openfhe-clang.cmd` |
| Компилятор C/C++ | **clang-cl** (MSVC `cl.exe` официально не поддерживается OpenFHE) | `README.md` §«Окружение» |
| SDK target | `-DCMAKE_SYSTEM_VERSION=10.0.26100.0` | `scripts/cfg-openfhe-clang.cmd`, `scripts/setup-openfhe.ps1` |
| Rust | **1.97.0**, toolchain `x86_64-pc-windows-msvc` | Заявлено для опубликованных чисел. В репозитории **нет** `rust-toolchain.toml`, поэтому версия фиксируется только манифестом прогона (§6.2). Замечание: CI ставит `dtolnay/rust-toolchain@stable`, то есть **не** пинит 1.97.0 — расхождение, которое стоит устранить |
| Зависимости Rust | Пинованы `Cargo.lock`: `cxx`/`cxx-build` 1.0.200, `cc` 1.4.5, `serde`/`serde_core`/`serde_derive` 1.0.229, `serde_json` 1.0.151, `link-cplusplus` 1.0.12 | `Cargo.lock` (закоммичен намеренно, см. комментарий в `.gitignore`) |
| OpenFHE | **1.5.1**, коммит `ed361af22049007db2107e7c69bcff209e8c420d` (ветка `main`); версия подтверждается `OPENFHE_VERSION_{MAJOR,MINOR,PATCH} = 1/5/1` | `README.md` §«Структура»; `vendor/openfhe-development/CMakeLists.txt`; `scripts/setup-openfhe.ps1` (`$OpenFheCommit`) |
| Ревизия HEPRE | Ветка `master`, на момент написания `4659cac3e24537fb99fb3a25c4df98df28ff4f3f` | `.git/HEAD` |
| Для скриптов `*.ps1` | PowerShell 7 (`pwsh`); `Makefile` требует GNU make | `scripts/*.ps1`, `Makefile` |
| CI | `.github/workflows/ci.yml`: отдельные задания `checks` (fmt + clippy, ubuntu) и `windows-build` (windows-latest, clang-cl 19.1.5 через choco, `cargo test --release`) | `.github/workflows/ci.yml` |

Примечание: команда `git` в среде подготовки этого документа была недоступна (отказ доступа к
`git.exe`), поэтому ревизии прочитаны напрямую из `.git/HEAD`. Runner обязан получать ревизию через
`git rev-parse HEAD` и записывать её в манифест.

---

## 3. Вендорный OpenFHE: обязательный внешний шаг

`vendor/` **не входит в репозиторий** — он git-ignored как тяжёлый внешний артефакт
(`.gitignore`, строки 9–12: `/vendor/`). Поэтому читатель обязан получить исходники сам.
Штатный путь — скрипт `scripts/setup-openfhe.ps1`:

```powershell
pwsh -NoProfile -File scripts/setup-openfhe.ps1
```

Скрипт выполняет (и это следует считать нормативной процедурой):

1. **Fail fast**: если `vendor\openfhe-install\lib` уже существует, скрипт останавливается с
   подсказкой (идемпотентный ручной запуск, а не повторная пересборка).
2. `git init/fetch --depth 1` и `git checkout --detach` для коммита
   **`ed361af22049007db2107e7c69bcff209e8c420d`**, затем проверка `git rev-parse HEAD` на точное
   совпадение.
3. Применение локального патча **`patches\openfhe-1.5.1-msvc.patch`** (`git apply --check`, затем
   `git apply`). При отсутствии файла скрипт **падает**; обходной ключ `-AllowMissingPatch` только
   предупреждает и продолжает.
4. Импорт окружения `vcvars64.bat` (путь ищется через `vswhere.exe`) в текущий процесс PowerShell.
5. Конфигурация, сборка и установка с **точно теми же** флагами CMake, что в `README.md`
   (§«Сборка OpenFHE») и в `scripts/cfg-openfhe-clang.cmd`:

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

Сборка/установка (эквивалент шагов внутри скрипта):

```bat
cmake --build vendor\openfhe-clang-build   --config Release
cmake --install vendor\openfhe-clang-build --config Release
```

Debug-вариант — те же флаги с `-B vendor\openfhe-clang-debug-build`, `-DCMAKE_BUILD_TYPE=Debug`,
`-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreadedDebugDLL`; Debug-библиотеки **не устанавливаются**
(только для диагностики): `scripts/cfg-openfhe-clang-debug.cmd`,
`scripts/build-openfhe-clang-debug.cmd`.

Ключевые производные параметры конфигурации (проверяются сборкой): `HAVE_INT128=TRUE`,
`NATIVEINT=64`, `MATHBACKEND=4` (`README.md` §«Окружение»).

### 3.1 Пробелы воспроизводимости (важно для ревьюера)

1. **Патч-файл присутствует, но `README.md` утверждает обратное.** `patches/openfhe-1.5.1-msvc.patch`
   есть в дереве (~10 КБ; в заголовке указаны базовый коммит
   `ed361af22049007db2107e7c69bcff209e8c420d`, способ генерации `git diff HEAD --ignore-submodules=all`
   и команда применения `git apply patches/openfhe-1.5.1-msvc.patch`). Он применяется автоматически
   шагом 3 `scripts/setup-openfhe.ps1` (с `git apply --check` перед применением) и учитывается в
   ключе кэша CI (`hashFiles('patches/*.patch', …)`).
   При этом `README.md` в разделе «Вендорные патчи» по-прежнему говорит: «Патчи применяются к файлам
   исходников напрямую; отдельный diff/patch-файл не поддерживается (дерево уже изменено)» — это
   устаревшая формулировка, которую стоит синхронизировать с фактическим состоянием. Таблица тех же
   11 правок в `README.md` остаётся полезной как описание «зачем»: `CMakeLists.txt`;
   `src/binfhe/lib/rgsw-acc.cpp`; `src/core/include/math/hal/intnat/ubintnat.h`;
   `src/core/include/math/math-hal.h`; `src/core/include/math/hal/bigintfxd/ubintfxd.h`,
   `bigintntl/ubintntl.h`; `src/pke/include/encoding/packedencoding.h`, `cryptocontext.h`;
   `src/core/lib/utils/demangle.cpp`; `src/pke/include/cryptocontextfactory.h`;
   `src/pke/lib/cryptocontextfactory.cpp`;
   `src/pke/lib/scheme/ckksrns/ckksrns-parametergeneration.cpp`;
   `src/pke/lib/ciphertext-impl.cpp`.
2. **Конфигурация исследования есть** — `configs/study.json` зафиксирована в дереве и
   воспроизводит эталонный прогон. Файл создаётся/пересоздаётся командой
   `cargo run --release --bin study -- --write-default-config configs/study.json`;
   без `--config` раннер использует встроенную `default_study_config()`.
3. **Каталог `results/` есть.** В репозитории лежат прогоны исследования; ключевые
   описаны ниже:

   | Прогон | Что это |
   |---|---|
   | `results/20260925-042702_4659cac3/` | исторический прогон, используемый для проверки воспроизводимости (пара с повторным) |
   | `results/20261012-083347_4659cac3/` | повторный прогон из пары сравнения |
   | `results/20261015-052701_c4a6a8f1/` | **эталонный прогон; на его числа ссылается рукопись**: единый конвейер — контроль чувствительности (`negative_control`) и масштабируемость до 120 000 профилей |

   Единый прогон выполняет всё одним запуском (конфигурация `configs/study.json`):
   валидационная выборка модели остаётся 4 000 наблюдений, а тест масштабируемости
   измеряется на пуле до 120 000 профилей:

   ```powershell
   .\target\release\study.exe --config configs\study.json
   ```

   Измеренные значения (медиана 3 повторов, машина статьи): 1 000 — 2,3 с;
   8 000 — 2,2 с (один пакет); 40 000 — 10,9 с (5 пакетов); 80 000 — 21,9 с (10);
   120 000 — 32,7 с (15). Время на пакет постоянно ≈ 2,2 с, то есть рост линейный;
   экстраполяция даёт ≈ 55 с на 200 000 и ≈ 109 с на 400 000 профилей (объём
   зашифрованных показателей ≈ 825 и 1 617 МБ соответственно — на машине с 6 ГБ RAM
   это предел, ограничитель именно память, а не время).

   Пара `20260925 / 20261012-083347` оставлена намеренно: их сравнение и есть
   проверка воспроизводимости. Выполняется она скриптом

   ```powershell
   python scripts/compare_runs.py results/20260925-042702_4659cac3 results/20261012-083347_4659cac3
   ```

   **Наблюдённый результат сравнения:** совпали все **точно-обязательные**
   величины, перечень которых задаёт скрипт, включая хэш `cohort.csv`
   (когорта побитово идентична), коэффициенты модели, коэффициенты многочлена,
   все клинические метрики, число расхождений решений, вывод о согласии с
   эталоном и все объёмы ключей и шифротекстов. Различаются только:

   * величины режима A, состоящие целиком из шума CKKS: `max|Δp|` 5,75·10⁻⁹ против
     5,52·10⁻⁹ — обе пренебрежимо малы (проверяется абсолютным пределом 10⁻⁶);
   * `max|Δp|` режима B в шестой значащей цифре (4,1301·10⁻³ против 4,1301·10⁻³):
     там доминирует детерминированная погрешность аппроксимации, а шум лишь шевелит
     младшие разряды (проверяется относительным пределом 10⁻³);
   * тайминги: второй прогон прошёл примерно на 30 % быстрее (2,21 с против 3,03 с
     на пакет) из-за загрузки машины. Инвариантны число пакетов и отсутствие
     расхождений знака — их и проверяет скрипт, а не абсолютное время.

   Прежние черновые прогоны удалены, чтобы в репозитории не накапливался мусор;
   два описанных выше набора сохраняются.
4. **Модульные тесты лежат внутри модулей**, а не в отдельных файлах: блоки
   `#[cfg(test)] mod tests` в `src/{rng,detmath,hash,cohort,stats,logistic,poly,env,he,study}.rs`.
   Это сделано намеренно — так тестируются приватные функции (решатель линейных систем,
   проверки монотонности и знака, внутренние помощники), которые не должны попадать в
   публичный API. Все **71** модульный тест и **15** интеграционных тестов проходят.
   Отдельного файла `tests/*.rs` для каждой темы нет и не требуется.
   Тесты, которым нужен рабочий каталог, пишут его внутрь `target/` (см.
   `study::tests::test_output_root` и `tests/roles_handoff.rs`), а не в системный
   `TEMP`: каталог сборки гарантированно доступен на запись рядом с проектом,
   исключён из репозитория и вычищается `cargo clean`.

---

## 4. MSVC-рантайм в `PATH` (обязательное условие запуска)

Без MSVC-рантайма в `PATH` запуск собранных exe и тестов падает с `0xC0000135` (не найдены
`MSVCP140.dll`, `VCRUNTIME140.dll`, `VCRUNTIME140_1.dll`). Пример из `README.md`:

```powershell
$rt = "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207\bin\Hostx64\x64"
$env:PATH = "$rt;$env:PATH"
cargo test
```

`scripts/repro.ps1` и CI делают это автоматически: определяют последнюю версию MSVC через
`vswhere.exe`, собирают путь `VC\Tools\MSVC\<версия>\bin\Hostx64\x64` и добавляют его в `PATH`.
Ожидаемый результат тестов — **15/15** зелёных, стабильно в том числе при параллельном прогоне
(благодаря единому мьютексу в шиме).

---

## 5. Сборка Rust-обёртки

`build.rs` компилирует шим `cpp/openfhe-shim.cc` через `cxx_build::bridge("src/ffi.rs")` с флагами
`/MD /std:c++17 /EHsc /D_USE_MATH_DEFINES` и линкует `OPENFHEbinfhe_static`, `OPENFHEcore_static`,
`OPENFHEpke_static` + `clang_rt.builtins-x86_64`.

| Переменная окружения | Значение по умолчанию в `build.rs` |
|---|---|
| `OPENFHE_HOME` | `D:/lair/hepre/vendor/openfhe-install` |
| `CLANGCL` | `C:/Program Files/Microsoft Visual Studio/2022/Community/VC/Tools/Llvm/x64/bin/clang-cl.exe` |
| `CLANG_RT_DIR` | `C:/Program Files/Microsoft Visual Studio/2022/Community/VC/Tools/Llvm/x64/lib/clang/19/lib/windows` |

Требование согласованности CRT: Release → `/MD`, Debug → `/MDd`, иначе `LNK2038`.
`clang_rt.builtins-x86_64.lib` обязательна, иначе неразрешённые `__modti3`, `__floatuntidf`,
`__fixunsdfti` (операции над `__int128`).

Основные команды (см. также `Makefile`):

| Цель `Makefile` | Команда |
|---|---|
| `build` | `cargo build` |
| `build-release` | `cargo build --release --bins` |
| `test` / `test-release` | `cargo test` / `cargo test --release` (ожидается 15/15) |
| `bench` | `cargo run --release --example bench` |
| `study` | `cargo run --release --bin study -- --config configs/study.json` |
| `study` (создать конфигурацию) | `cargo run --release --bin study -- --write-default-config configs/study.json` |
| `study` (другой каталог результатов) | `cargo run --release --bin study -- --config configs/study.json --out results` |
| `repro` | `pwsh -NoProfile -File scripts/repro.ps1` |

CLI раннера (`src/bin/study.rs`): `--config <файл>`, `--out <каталог>` (по умолчанию `results`),
`--write-default-config <файл>`, `--help`. Без `--config` используется встроенная
`default_study_config()`, а существующий `configs/study.json` подхватывается автоматически.

---

## 6. Политика детерминизма

### 6.1 Что пинится (реализовано)

- **Seed.** Единственный источник воспроизводимости — `CohortConfig::seed` (значение по умолчанию
  `20_260_907` в `default_t2dm_config()`), записывается в `results/<run-id>/manifest.json`.
- **Целочисленный PRNG.** `src/rng.rs`: `xoshiro256**` с инициализацией состояния через
  `SplitMix64`; только арифметика `u64`, поэтому последовательность **побитово одинакова на любой
  платформе**. Преобразование в `f64` берёт старшие 53 бита (`>> 11`) — ровно представимые значения
  в `[0, 1)`.
- **Именованные подпотоки.** `Rng::substream(seed, name)` (имя хэшируется FNV-1a и XOR-ится с
  семенем) разводит калибровку распространённости, генерацию ковариат, исходов и разбиение; порядок
  вызовов не влияет на результат.
- **Детерминированная математика.** `src/detmath.rs` реализует `exp`, `ln`, `cos`, `powi`, `logit`,
  `sigmoid` через `+ − × ÷` и точное масштабирование степени двойки (`to_bits`/`from_bits`);
  `sqrt` берётся из `std`, потому что по IEEE-754 он обязан быть корректно округлённым. Причина
  прямо указана в модуле: `f64::exp`/`f64::ln` не специфицированы IEEE-754 и могут отличаться на
  последних битах между платформами и версиями libm.
- **Автономный SHA-256.** `src/hash.rs` — собственная реализация FIPS 180-4 (`sha256_hex`,
  `sha256_file`, инкрементальный `Sha256`), чтобы сборка была офлайн, а хэш артефакта проверялся
  независимо; корректность — по тестовым векторам NIST в модульных тестах.
- **Отсутствие внешних зависимостей для статистики.** `src/stats.rs` реализован на `std` +
  `detmath` (AUC по Манну—Уитни с интервалом ДеЛонга, интервалы Вильсона и Клоппера—Пирсона, точный
  тест МакНемара, каппа Коэна, MCC, F1, мера Бриера, калибровка, процентильный бутстрап с явным
  семенем).
- **Самопроверка детерминизма в раннере.** `src/study.rs` повторно генерирует когорту и сравнивает
  результат: `DeterminismCheck { cohort_sha256_first, cohort_sha256_second, cohort_bit_identical,
  model_bit_identical, polynomial_bit_identical }` попадает в `results.json`. То есть факт
  бит-воспроизводимости фиксируется **в артефакте прогона**, а не только в тестах.
- **Отпечаток окружения без внешних вызовов.** `src/env.rs` форматирует время собственной
  реализацией `civil_from_days` (алгоритм Х. Хиннанта) и читает ревизию/ветку git **напрямую из
  `.git`**, не запуская `git`; версия компилятора передаётся из `build.rs` через
  `cargo:rustc-env=HEPRE_RUSTC_VERSION=…` (вместе с `HEPRE_OPENFHE_HOME`). Это удобно для
  воспроизведения в средах, где `git` недоступен.

### 6.2 Манифест и отпечаток среды

`manifest.json` — машиночитаемая запись прогона (структура `StudyResults` + список артефактов);
`environment.json` — отпечаток среды (`EnvironmentFingerprint`). Фактические поля:

| Поле (артефакт) | Смысл |
|---|---|
| `run_id` | UTC-метка запуска (`env::run_id`), входит в имя каталога (§8) |
| `started_utc`, `finished_utc`, `duration_seconds` | Границы и длительность прогона |
| `study_name`, `design` | Имя исследования и описание дизайна из конфигурации |
| `config` / `config.json` | Полная конфигурация `StudyConfig` (в т.ч. `cohort.seed`) |
| `environment` / `environment.json` | `rustc_version`, `arch`, `os`, `os_version`, `cpu_count`, `openfhe_home`, `crate_version`, `git_revision`, `git_branch`, `started_unix`, `started_utc` |
| `cohort` | `CohortDiagnostics` (реализованная распространённость, статистика логита, число итераций калибровки, `feature_summary`) |
| `model` | `ModelSummary` (коэффициенты, стандартизация, сходимость, `n_train`, `n_events`) |
| `polynomial` | `PolynomialSummary` + развёртка «точность против глубины» |
| `clinical_performance` | Метрики на валидационной выборке с интервалами |
| `keys_and_volume` | `KeyAndVolumeReport`: размеры ключей и шифротекстов |
| `results.json` (артефакт) | Полный `StudyResults` |
| `manifest.json` (артефакт) | Перечень всех артефактов с SHA-256 |
| `artifacts[] {file, sha256, bytes}` | Контрольная сумма (`src/hash.rs`) и размер каждого артефакта |
| `determinism` | `DeterminismCheck` — совпадение SHA-256 когорты при повторной генерации, бит-идентичность модели и многочлена |
| `sample_size` | `SampleSizeJustification`: `validation_n`, `equivalence_margin`, `alpha`, верхняя граница 95 % ДИ при нуле расхождений, допустимое число расхождений, вывод |

Отпечаток среды намеренно включает только то, что влияет на интерпретацию замеров; конкретные
значения появятся в `results/<run-id>/environment.json` после прогона.

### 6.3 Бит-воспроизводимость когорты

Требование «не использовать платформенно-зависимые трансцендентные функции libm в генерации
когорты» **выполнено в коде**: `src/rng.rs` использует `detmath::sqrt`/`detmath::ln` (полярный метод
Марсальи) вместо `f64::ln`, а `src/cohort.rs` считает вероятности через `detmath::sigmoid`. Поэтому
`cohort.csv` при том же seed воспроизводится бит-в-бит; это проверяется модульными тестами
`generation_is_deterministic` (сравнение `f64::to_bits()` для `eta_true` и полного CSV),
`different_seed_changes_data`, `prevalence_is_close_to_target`,
`split_partitions_the_pool_without_overlap`, `features_respect_declared_bounds`,
`eta_and_probability_are_consistent_with_truth`, `csv_has_header_and_all_rows`,
`diagnostics_are_populated` (модуль объявлен в `src/lib.rs`), а независимо — полем `determinism`
в `results.json` (§6.1).

### 6.4 Что детерминировано, а что нет

- `cohort.csv`, `model.json` — **бит-в-бит** при том же seed и той же версии генератора.
- Погрешность гомоморфного контура (`max|err|`) — воспроизводима по порядку величины, но не
  побитово: `README.md` прямо отмечает, что `max|err|` «ориентировочно ~10⁻⁹ (шум CKKS случаен)».
  Поэтому `results.json` обязан содержать сами метрики ошибки, а не предполагать побайтовое
  равенство выходов.
- Шифротексты, ключи и `rekey` — **не** воспроизводимы побайтово (PRNG OpenFHE).
- Тайминги — машинно-зависимы; публиковать их следует как **относительные стоимости** вместе с
  `environment.json`, с оговоркой «Release-замеры `std::time`, зависят от машины и загрузки».

---

## 7. Чек-лист ревьюера (пошагово)

| Шаг | Действие | Ожидаемый результат |
|---|---|---|
| 1 | Установить VS 2022 Community + компонент `Microsoft.VisualStudio.Component.VC.Llvm.Clang` | `clang-cl` 19.1.5 доступен в `…\VC\Tools\Llvm\x64\bin` |
| 2 | Установить CMake и убедиться, что `vcvars64.bat` даёт NMake | `cmake --version` ≥ 3.16.3; `nmake` в `PATH` после `vcvars64` |
| 3 | Установить Rust 1.97.0 MSVC: `rustup toolchain install 1.97.0-x86_64-pc-windows-msvc` | `rustc -V` печатает 1.97.0, target `x86_64-pc-windows-msvc` |
| 4 | Проверить наличие `patches/openfhe-1.5.1-msvc.patch` (в дереве есть, ~10 КБ; см. §3.1, п. 1) | файл применяется к закреплённому коммиту через `git apply --check` |
| 5 | `pwsh -NoProfile -File scripts/setup-openfhe.ps1` | OpenFHE 1.5.1 на коммите `ed361af…`, префикс `vendor\openfhe-install\lib` |
| 6 | При необходимости выставить `OPENFHE_HOME`, `CLANGCL`, `CLANG_RT_DIR` (§5) | `cargo build` не падает на линковке |
| 7 | Добавить MSVC-рантайм в `PATH` (автоматически в `repro.ps1`/CI) | — |
| 8 | `cargo build --release --bins` | собраны `lab`, `server`, `hospital` |
| 9 | `cargo test --release` | **15/15** зелёных (4 CKKS + 2 BFV + 2 PRE + 3 serialize + 3 score + 1 roles_handoff) |
| 10 | `cargo run --release --example bench` | таблица N = 10/100/1000/10000; KeyGen/ReKeyGen; `max\|err\|`; `binMis` |
| 11 | Прогнать демо трёх ролей (`README.md` §«Пример использования») | файлы handoff, скоры P1–P5, «Сверка пройдена» |
| 12 | Создать конфигурацию: `cargo run --release --bin study -- --write-default-config configs/study.json` | появился `configs/study.json` (иначе `repro.ps1` падает, §3.1) |
| 13 | `make repro` (`scripts/repro.ps1`) | проходят сборка, тесты и раннер; появляется `results/<run-id>/` |
| 14 | Проверить артефакты и digest-ы | `manifest.json`, `environment.json`, `config.json`, `cohort.csv`, `model.json`, `results.json`, `*.csv`; SHA-256 совпадают, `determinism.cohort_bit_identical = true` |
| 15 | Сравнить числа с `README.md` | тесты совпадают; тайминги **отличаются** (ожидаемо, §6.4) |

---

## 8. Целевая раскладка `results/` и таблица «величина → команда → артефакт»

### 8.1 Раскладка

`scripts/repro.ps1` проверяет наличие обязательного минимума в последнем каталоге прогона
(`manifest.json`, `environment.json`, `cohort.csv`, `model.json`, `results.json`); раннер
(`src/study.rs`) дополнительно записывает:

```
results/
└── <run-id>/
    ├── manifest.json                  # перечень артефактов с SHA-256, отпечаток, детерминизм
    ├── environment.json               # EnvironmentFingerprint: Rust/арх/ОС/CPU/OpenFHE/git
    ├── config.json                    # полная конфигурация StudyConfig (включая cohort.seed)
    ├── cohort.csv                     # синтетическая когорта (столбцы — docs/DATA_DICTIONARY.md §4.2)
    ├── model.json                     # обученная модель: веса, интерцепт, стандартизация, порог
    ├── roc_curve.csv                  # ROC-кривая на валидационной выборке
    ├── calibration_deciles.csv        # калибровочные децили
    ├── polynomial_sweep.csv           # «точность против глубины» для аппроксимации сигмоиды
    ├── polynomial_coefficients.csv    # коэффициенты выбранного многочлена
    ├── scalability.csv                # замеры по размерам когорты (медиана по повторам)
    └── results.json                   # полный StudyResults (все посчитанные величины)
```

`results.json` перечисляет посчитанные артефакты, но не сам себя; полный перечень, включая
`results.json`, `environment.json` и `config.json`, содержится в `manifest.json` (комментарий в
`src/study.rs`).

`<run-id>` кодирует UTC-метку запуска и короткую ревизию git, например
`20260925-042702_4659cac3` (ISO-8601 UTC без разделителей плюс первые восемь символов хэша
коммита — так имена каталогов сортируются хронологически и привязаны к коду).

Эталонный прогон `results/20261015-052701_c4a6a8f1/` **закоммичен намеренно**: это
замороженный набор артефактов, на который ссылается рукопись. `.gitignore` каталог
`results/` не исключает. Старые прогоны следует удалять вручную, чтобы в репозитории
оставался один актуальный набор.

### 8.2 Таблица воспроизведения отчётных величин

| Отчётная величина | Команда, которая её создаёт | Артефакт / источник значения |
|---|---|---|
| 15/15 интеграционных тестов и их состав (4/2/2/3/3/1) | `cargo test` (или `cargo test --release`) | вывод Cargo; перечень — `tests/*.rs`, таблица в `README.md` §«Запуск тестов» |
| `ring_dimension == 16384` при batch 8192 | `cargo test ring_dimension_positive` | `tests/ckks.rs` |
| KeyGen/ReKeyGen и Encrypt/Score/PRE/Decrypt для N = 10/100/1000/10000 | `cargo run --release --example bench` (`make bench`) | **канонический источник** — `results/<run-id>/results.json`; текущие значения — `README.md` §«Бенчмарк» (`examples/bench.rs`) |
| `max\|err\|` и `binMis` по размерам когорты | то же | `results/<run-id>/results.json`; текущие значения — `README.md` §«Бенчмарк» |
| Скоры P1–P5 и вердикты порога (−1.45) | `hospital keygen` → `lab encrypt` → `server score` → `hospital decrypt` | `README.md` §«Пример использования»; проверяется тестом `role_utilities_end_to_end` |
| Демонстрационная сверка `max\|HE скор − открытый скор\| < 1e-2`, «несовпадений порога: 0» | `cargo run --release --bin hospital -- decrypt --model … --patients …` | `src/bin/hospital.rs`; `README.md` §«Пример» |
| Размеры артефактов handoff (`features.cts.bin`, `rekey.ek.bin`, `scores.ct.bin`, `hosp.pk.bin`, `hosp.sk.bin`) | шаги 1–3 демо ролей | stdout утилит (`common::kb`); пример — `README.md` §«Пример» |
| Воспроизводимость когорты (бит-в-бит) | `cargo run --release --bin study …` (поле `determinism`), а также модульный тест `src/cohort.rs::generation_is_deterministic` | `results/<run-id>/results.json` (`DeterminismCheck`), SHA-256 когорты в `manifest.json` |
| Корректность SHA-256 | `cargo test` (тестовые векторы NIST в `src/hash.rs`) | `src/hash.rs` |
| Метрики исследования (AUC + ДИ ДеЛонга, чувствительность/специфичность, PPV/NPV, F1, MCC, каппа, Brier, калибровка, МакНемар, контроль чувствительности, бутстрап-интервалы) | `cargo run --release --bin study -- --config configs/study.json` (`make study`) | `results/<run-id>/results.json` (`clinical_performance`, `negative_control`), `roc_curve.csv`, `calibration_deciles.csv`; эталонный прогон — `results/20261015-052701_c4a6a8f1/` |
| Обоснование размера выборки (эквивалентность) | то же | `results/<run-id>/results.json` (`sample_size`: `validation_n`, `equivalence_margin`, `alpha`, верхняя граница 95 % ДИ при нуле расхождений) |
| Масштабируемость гомоморфного контура | то же | `results/<run-id>/scalability.csv` |
| Размеры ключей и объём шифротекстов | то же | `results/<run-id>/results.json` (`keys_and_volume`) |
| Погрешность и глубина аппроксимации сигмоиды | то же | `polynomial_sweep.csv`, `polynomial_coefficients.csv` |
| Отпечаток среды и версии | раннер исследования | `results/<run-id>/environment.json`, `manifest.json` |
| Digest-ы артефактов | `Get-FileHash -Algorithm SHA256 <file>` (сверка с манифестом) | `results/<run-id>/manifest.json` |

Правило: если величины нет ни в `README.md`, ни в `results/<run-id>/`, она **не является** отчётной
величиной проекта и не должна приводиться как измеренная.

---

## 9. Что воспроизвести нельзя

1. **Абсолютные тайминги.** Зависят от CPU, ОС, загрузки и версии рантайма; сравнивать допустимо
   только относительные стоимости на одной машине. Публикация «мс» без `environment.json`
   некорректна.
2. **Побайтовые шифротексты, ключи и `rekey`.** PRNG OpenFHE не пинится; воспроизводимы только
   структурные свойства (размеры, успешность расшифровки, fail-closed).
3. **Точные значения `max|err|`.** CKKS-шум случаен: воспроизводим порядок величины и инвариант
   «порог совпадает», а не само число.
4. **`vendor/` не входит в репозиторий.** Дифф правок воспроизводим: патч
   `patches/openfhe-1.5.1-msvc.patch` вместе с закреплённым коммитом `ed361af` применяется
   скриптом `scripts/setup-openfhe.ps1`. Сходимость патча проверяется командой
   `git -C vendor/openfhe-development apply --reverse --check patches/openfhe-1.5.1-msvc.patch`
   (нулевой код возврата). Построенное дерево/префикс при этом не воспроизводится побайтово:
   другой компилятор или состояние кэша дадут иную сборку.
5. **Прогоны на других версиях инструментов.** Иной clang-cl / MSVC-рантайм / коммит OpenFHE дают
   иные `max|err|`, размеры и тайминги; CI к тому же ставит `stable`-Rust, а не 1.97.0.
6. **Точные тайминги.** Они машинозависимы и воспроизводятся лишь по порядку величины и по
   характеру масштабирования (линейный рост по числу пакетов). Эталонные значения получены на
   8-ядерной машине под Windows x64; среда зафиксирована в `environment.json`.
7. **История экспериментов.** Черновые прогоны не архивируются: в репозитории остаются два
   набора, описанные в §3.1, п. 3. Новый запуск создаёт новый `run-id`, а совпадение с
   эталоном проверяется не глазами, а скриптом `scripts/compare_runs.py` (см. §3.1).
8. **Клинические результаты.** Реальных данных нет: когорта синтетическая, исход — Bernoulli из
   DGP (`docs/DATA_DICTIONARY.md` §2.3). Воспроизводится работа криптографического прототипа и
   внутренняя согласованность, а не клиническая точность.
9. **Стойкость параметров и безопасность.** Уровень безопасности явно не задан, аудита и
   формального доказательства нет; «воспроизвести оценку стойкости» по этому репозиторию нельзя
   (`docs/THREAT_MODEL.md` §7, O10; `SECURITY.md`).
