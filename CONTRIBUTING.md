# Участие в разработке HEPRE

Спасибо за интерес к проекту. HEPRE — исследовательский прототип приватного скоринга
риска диабета 2-го типа на гомоморфном шифровании: математическое ядро на C++ (OpenFHE),
обёртка на Rust через `cxx` bridge. Ниже — как развернуть окружение, собрать проект
и в каком стиле писать код.

Перед началом прочитайте `README.md` (структура, параметры конфигурации OpenFHE,
разбор типовых сбоев) и `design.txt` (ТЗ и план исследования).

## 1. Окружение (Windows x64)

Поддерживаемая конфигурация — Windows x64:

- **Visual Studio 2022 Community** с компонентом
  `Microsoft.VisualStudio.Component.VC.Llvm.Clang` (clang-cl 19 + lld-link),
  путь по умолчанию: `C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\Llvm\x64\`.
- **CMake** и **NMake** (NMake появляется в среде `vcvars64.bat`).
- **Rust** с MSVC-toolchain `x86_64-pc-windows-msvc`.
- **MSVC C++ runtime** должен быть в `PATH` при запуске собранных exe, тестов и примеров
  (`MSVCP140.dll`, `VCRUNTIME140.dll`, `VCRUNTIME140_1.dll`), иначе процесс падает
  с `0xC0000135`:

  ```powershell
  $rt = "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207\bin\Hostx64\x64"
  $env:PATH = "$rt;$env:PATH"
  ```

`cl.exe` (MSVC) для сборки OpenFHE **не поддерживается** — используется `clang-cl`
(тот же MS ABI, но другой фронтенд).

Переопределяемые переменные окружения для сборки Rust-обёртки (`build.rs`):

| Переменная | Назначение | Значение по умолчанию |
|---|---|---|
| `OPENFHE_HOME` | префикс установки OpenFHE | `D:/lair/hepre/vendor/openfhe-install` |
| `CLANGCL` | путь к `clang-cl.exe` | `.../VC/Tools/Llvm/x64/bin/clang-cl.exe` |
| `CLANG_RT_DIR` | каталог `clang_rt.builtins-x86_64.lib` | `.../Llvm/x64/lib/clang/19/lib/windows` |

Каталог `vendor/` (исходники OpenFHE, сборки и префикс установки) **не входит в репозиторий**:
он слишком тяжёлый. Поэтому его нужно получить локально (см. ниже).

## 2. Сборка вендорного OpenFHE

Скрипт одной командой (рекомендуемый путь):

```powershell
pwsh -File scripts/setup-openfhe.ps1
```

Он клонирует OpenFHE в `vendor/openfhe-development` на закреплённый коммит,
применяет локальный MSVC-патч, конфигурирует Release-сборку `clang-cl` в
`vendor/openfhe-clang-build` и устанавливает префикс в `vendor/openfhe-install`.
Если префикс уже существует, скрипт останавливается с сообщением — удалите
`vendor/openfhe-install` только если действительно нужна пересборка.

Вручную — ровно теми же командами, что в `README.md` (раздел «Сборка OpenFHE»),
обязательно из среды `vcvars64.bat`:

```bat
call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
cmake -S vendor\openfhe-development -B vendor\openfhe-clang-build -G "NMake Makefiles" ^
  -DCMAKE_C_COMPILER=clang-cl -DCMAKE_CXX_COMPILER=clang-cl ^
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreadedDLL ^
  -DCMAKE_SYSTEM_VERSION=10.0.26100.0 ^
  -DBUILD_STATIC=ON -DBUILD_SHARED=OFF -DBUILD_UNITTESTS=OFF -DBUILD_EXAMPLES=OFF -DBUILD_BENCHMARKS=OFF ^
  -DWITH_OPENMP=OFF -DWITH_NTL=OFF -DBUILD_EXTRAS=OFF ^
  -DCMAKE_INSTALL_PREFIX=vendor\openfhe-install
cmake --build vendor\openfhe-clang-build   --config Release
cmake --install vendor\openfhe-clang-build --config Release
```

Важно:

- Ключевые параметры ядра: `HAVE_INT128=TRUE`, `NATIVEINT=64`, `MATHBACKEND=4`
  (uint64 + `__int128`). OpenMP/NTL/GMP отключены.
- Библиотеки статические (`OPENFHE{pke,core,binfhe}_static.lib`), к ним обязательно
  подключается `clang_rt.builtins-x86_64.lib` — иначе линковка падает на
  `__modti3`, `__floatuntidf`, `__fixunsdfti` (операции над `__int128`).
- CRT должен совпадать с библиотеками: Release → `/MD`, Debug → `/MDd` (иначе `LNK2038`).
- Debug-сборка (`vendor/openfhe-clang-debug-build`, `MultiThreadedDebugDLL`) собирается
  только для диагностики и не устанавливается.
- **Критично для CKKS** — исправление в `src/pke/lib/ciphertext-impl.cpp`
  (`SetLevel` использует не-владеющий `dynamic_pointer_cast<const CryptoParametersCKKSRNS>`).
  Вариант со вторым владеющим `shared_ptr` приводит к heap corruption (`0xC0000374`).
  Полный список локальных патчей — в таблице `README.md` «Вендорные патчи».
- PRE требует **обеих** настроек: `cc->Enable(PRE)` и `parameters.SetPREMode(INDCPA)`.
  Без первой `ReKeyGen` бросает исключение, без второй `KeySwitchHYBRID` уходит
  в неопределённое поведение (`0xC0000005`). Оба случая подробно разобраны в `README.md`.

## 3. Сборка, тесты, бенчмарк

Из корня репозитория, с MSVC-рантаймом в `PATH`:

```powershell
cargo build                      # debug
cargo build --release --bins     # бинарники ролей lab/server/hospital
cargo test --release             # 15 интеграционных тестов
cargo run --release --example bench
```

Ожидаемый результат тестов — **15/15 зелёные** (в том числе при параллельном прогоне:
все вызовы OpenFHE сериализуются единым мьютексом в shim). Раскладка по файлам —
в `README.md` (таблица «Ожидаемый результат»); при добавлении теста обновите
и число, и таблицу.

Воспроизведение исследования одной командой:

```powershell
pwsh -File scripts/repro.ps1
```

Скрипт проверяет наличие префикса OpenFHE, собирает Release, прогоняет тесты,
запускает раннер исследования `study` с конфигурацией `configs/study.json`
и печатает путь к машинно-читаемым результатам.

## 4. Соглашения о коде

- **Комментарии и документирующие комментарии — на русском языке.** Относится к Rust
  (`//!`, `///`, `//`) и к C++-шиму. Публичный API документируется через `///`
  (например, `hepre::Context::rekeygen`). Идентификаторы, имена файлов и сообщения
  об ошибках C++-исключений остаются латиницей.
- **Обработка ошибок — через `Result`, без паник.** `unwrap`/`expect`/`panic!`/`assert!`
  в библиотечном коде и в шиме не используются; ошибка возвращается вызывающему коду.
  В тестах `expect` допустим.
- **Через границу FFI паника не проходит.** `cxxbridge` генерирует `noexcept`-обёртки
  без try/catch: исключение C++, пересекающее границу, приводит к `std::terminate`
  (`0xC0000409`). Поэтому каждая FFI-функция шима ловит исключения шаблоном `guard<>`:
  успех очищает `g_last_error` (thread_local) и возвращает результат, провал сохраняет
  `e.what()` и возвращает «нулевое» значение (`0` для хэндлов, пустой вектор для `Vec`).
  Rust-сторона проверяет сентинель (`ok_handle`/`ok_vec`) и читает `take_last_error()`.
  Новые FFI-функции обязаны следовать этому шаблону без исключений.
- **Ресурсы — только через RAII.** Любой объект OpenFHE живёт в хэндле-обёртке
  (макрос `handle!`), освобождение — в `Drop`. Ручное освобождение и «сырые» указатели
  в публичном API не появляются.
- **Потокобезопасность.** OpenFHE держит глобальные статические реестры без
  синхронизации, поэтому shim сериализует все входы единым `std::mutex`. Не обходите
  этот мьютекс и не вызывайте OpenFHE в обход шима.
- **Формат.** `cargo fmt` (стандартный rustfmt, 4 пробела), `cargo clippy --all-targets -- -D warnings`.
  Перед push:
  ```powershell
  cargo fmt --all
  cargo clippy --all-targets -- -D warnings
  ```
- Заголовки сериализации OpenFHE (`ciphertext-ser.h`, `cryptocontext-ser.h`, `key-ser.h`
  и др.) подключаются **строго после** `using namespace lbcrypto;` — они используют
  неквалифицированный `DCRTPoly`.

## 5. Численные и статистические результаты

**Правило без исключений: числа не редактируются руками.** Любая численная или
статистическая величина в документации, таблицах, отчётах и `results/` должна быть
получена прогоном раннера исследования (`cargo run --release --bin study --
--config configs/study.json`, либо `scripts/repro.ps1`, либо `make study`), а не
вписана или поправлена вручную.

- Не «подгоняйте» метрики, доверительные интервалы и p-значения под ожидаемый вид.
  Если результат изменился — приведите его фактическое значение и объясните причину
  (смена параметров CKKS, версии OpenFHE, патча, модели).
- Не переносите числа из чужих машин и из старых прогонов как свои: публикуемые
  значения должны воспроизводиться текущим деревом на текущем окружении.
- Машинно-читаемые артефакты прогона складываются в `results/`; ручные правки файлов
  в `results/` недопустимы — их перезаписывает раннер.
- В коммите, меняющем численные результаты, указывайте команду и версию окружения,
  которыми они получены.
- Случайные величины (шум CKKS) воспроизводимы только при зафиксированном зерне —
  используйте генератор из `src/rng.rs` и явно записывайте зерно в конфигурацию.

## 6. Сообщения об ошибках и предложения

- Ошибки и предложения — через Issues репозитория
  <https://github.com/datascienceadvice/hepre/issues>.
- Пожалуйста, приложите: версию OpenFHE и коммит вендорного дерева, версию Rust
  (`rustc -V`), версию clang-cl, команду запуска, полный вывод и код завершения.
  Для падений (`0xC0000409`, `0xC0000374`, `0xC0000005`, `0xC0000135`) полезен стек
  из отладчика; символы берутся с symbol server
  `srv*<локальный-каталог-символов>*https://msdl.microsoft.com/download/symbols`.
- Уязвимости и проблемы безопасности — **не** в публичный трекер, а на
  <dal@capsula.by>; см. `SECURITY.md`.
- Правки кода — через Pull Request в `main`; держите изменения тематическими, добавляйте
  или обновляйте тесты и документацию, следите за зелёными `fmt`/`clippy`.

## 7. Лицензия

Проект распространяется по лицензии BSD-2-Clause (см. `LICENSE`). Отправляя вклад,
вы соглашаетесь с тем, что он распространяется на тех же условиях.
