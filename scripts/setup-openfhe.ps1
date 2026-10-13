<#
.SYNOPSIS
    Получение и сборка вендорного OpenFHE 1.5.1 для HEPRE (Windows x64, clang-cl).

.DESCRIPTION
    1. Проверяет, что префикс установки ещё не существует (fail fast).
    2. Клонирует OpenFHE на закреплённый коммит в vendor\openfhe-development.
    3. Применяет локальный MSVC-патч patches\openfhe-1.5.1-msvc.patch через `git apply`.
    4. Конфигурирует Release-сборку clang-cl (NMake Makefiles) в vendor\openfhe-clang-build.
    5. Собирает и устанавливает префикс в vendor\openfhe-install.

    Все шаги выполняются из среды vcvars64.bat (NMake + MSVC runtime), как описано
    в README.md. Скрипт рассчитан на идемпотентный ручной запуск, а не на повторную
    пересборку: если префикс уже есть, он останавливается с подсказкой.

.PARAMETER AllowMissingPatch
    Если патч-файл отсутствует, не завершаться ошибкой, а предупредить и продолжить.
    Нужно для CI, где дерево, полученное из закреплённого коммита, уже содержит
    эквивалентные правки. По умолчанию отсутствие патча — ошибка.

.EXAMPLE
    pwsh -NoProfile -File scripts/setup-openfhe.ps1
#>

[CmdletBinding()]
param(
    [switch]$AllowMissingPatch
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# --- Пути -------------------------------------------------------------------
$RepoRoot      = Split-Path -Parent $PSScriptRoot
$VendorDir     = Join-Path $RepoRoot 'vendor'
$SrcDir        = Join-Path $VendorDir 'openfhe-development'
$BuildDir      = Join-Path $VendorDir 'openfhe-clang-build'
$InstallDir    = Join-Path $VendorDir 'openfhe-install'
$PatchFile     = Join-Path $RepoRoot 'patches\openfhe-1.5.1-msvc.patch'
$RepoUrl       = 'https://github.com/openfheorg/openfhe-development.git'
$OpenFheCommit = 'ed361af22049007db2107e7c69bcff209e8c420d'

Write-Host '=== HEPRE: подготовка вендорного OpenFHE 1.5.1 ===' -ForegroundColor Cyan
Write-Host "Корень репозитория : $RepoRoot"
Write-Host "Исходники          : $SrcDir"
Write-Host "Каталог сборки     : $BuildDir"
Write-Host "Префикс установки  : $InstallDir"
Write-Host "Закреплённый коммит: $OpenFheCommit"

# --- 1. Fail fast, если префикс уже существует -------------------------------
$InstallLib = Join-Path $InstallDir 'lib'
if (Test-Path -LiteralPath $InstallLib) {
    throw ("Префикс OpenFHE уже установлен: $InstallLib`n" +
           'Ничего не делаю, чтобы не пересобирать дерево заново. ' +
           'Если пересборка действительно нужна, удалите каталог vendor\openfhe-install и запустите скрипт снова. ' +
           'Для стандартных сборок Rust-обёртки префикс уже готов: OPENFHE_HOME указывает на него.')
}
Write-Host '[1/5] Префикс не найден — продолжаю.' -ForegroundColor Green

# --- 2. Клонирование на закреплённый коммит ----------------------------------
if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    throw 'git не найден в PATH — установите Git for Windows и повторите запуск.'
}

if (Test-Path -LiteralPath (Join-Path $SrcDir '.git')) {
    Write-Host "[2/5] Каталог $SrcDir уже содержит git-дерево; переключаюсь на закреплённый коммит." -ForegroundColor Yellow
    git -C $SrcDir fetch --depth 1 origin $OpenFheCommit
    if ($LASTEXITCODE -ne 0) { throw "git fetch завершился с кодом $LASTEXITCODE" }
} else {
    Write-Host "[2/5] Клонирую OpenFHE в $SrcDir ..." -ForegroundColor Green
    New-Item -ItemType Directory -Force -Path $VendorDir | Out-Null
    git init $SrcDir
    if ($LASTEXITCODE -ne 0) { throw "git init завершился с кодом $LASTEXITCODE" }
    git -C $SrcDir remote add origin $RepoUrl
    if ($LASTEXITCODE -ne 0) { throw "git remote add завершился с кодом $LASTEXITCODE" }
    git -C $SrcDir fetch --depth 1 origin $OpenFheCommit
    if ($LASTEXITCODE -ne 0) { throw "git fetch завершился с кодом $LASTEXITCODE" }
}

git -C $SrcDir checkout --detach $OpenFheCommit
if ($LASTEXITCODE -ne 0) { throw "git checkout завершился с кодом $LASTEXITCODE" }

$head = (git -C $SrcDir rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0) { throw "git rev-parse завершился с кодом $LASTEXITCODE" }
if ($head -ne $OpenFheCommit) {
    throw "Ожидался коммит $OpenFheCommit, фактически $head"
}
Write-Host "      Исходники на коммите $head"

# --- 3. Локальный MSVC-патч --------------------------------------------------
Write-Host '[3/5] Применяю вендорный MSVC-патч ...' -ForegroundColor Green
if (Test-Path -LiteralPath $PatchFile) {
    git -C $SrcDir apply --check $PatchFile
    if ($LASTEXITCODE -ne 0) {
        throw ("git apply --check не прошёл для $PatchFile (код $LASTEXITCODE). " +
               'Патч не соответствует дереву коммита — обновите его или проверьте чистоту vendor\openfhe-development.')
    }
    git -C $SrcDir apply $PatchFile
    if ($LASTEXITCODE -ne 0) { throw "git apply завершился с кодом $LASTEXITCODE" }
    Write-Host '      Патч применён.'
} elseif ($AllowMissingPatch) {
    Write-Warning ("Патч $PatchFile не найден; продолжаю (-AllowMissingPatch). " +
                   'Проверьте, что дерево на закреплённом коммите уже содержит необходимые правки MSVC-совместимости.')
} else {
    throw ("Патч не найден: $PatchFile`n" +
           'Вендорный набор правок MSVC-совместимости обязателен: без него OpenFHE не собирается clang-cl, ' +
           'а CKKS повреждается в рантайме. Восстановите patches\openfhe-1.5.1-msvc.patch ' +
           'или запустите скрипт с -AllowMissingPatch, если дерево уже пропатчено.')
}

# --- 4. Среда vcvars64.bat ---------------------------------------------------
Write-Host '[4/5] Инициализирую среду Visual Studio (vcvars64.bat) ...' -ForegroundColor Green
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (-not (Test-Path -LiteralPath $vswhere)) {
    throw ("vswhere.exe не найден: $vswhere`n" +
           'Установите Visual Studio 2022 с компонентом Microsoft.VisualStudio.Component.VC.Llvm.Clang (см. README.md).')
}
$vsRoot = (& $vswhere -latest -products * -property installationPath)
if ($LASTEXITCODE -ne 0 -or -not $vsRoot) { throw 'vswhere не вернул путь установки Visual Studio.' }
$vsRoot = $vsRoot.Trim()

$vcvars = Join-Path $vsRoot 'VC\Auxiliary\Build\vcvars64.bat'
if (-not (Test-Path -LiteralPath $vcvars)) { throw "vcvars64.bat не найден: $vcvars" }

# Импортируем окружение vcvars64.bat в текущий процесс PowerShell:
# `cmd /c "call ...vcvars64.bat && set"` печатает переменные, которые мы копируем в $env:.
Write-Host "      $vcvars"
$vcvarsEnv = cmd /c "call `"$vcvars`" >nul 2>&1 && set"
if ($LASTEXITCODE -ne 0) { throw "vcvars64.bat завершился с кодом $LASTEXITCODE" }
foreach ($line in $vcvarsEnv) {
    if ($line -match '^([^=]+)=(.*)$') {
        Set-Item -Path "env:$($Matches[1])" -Value $Matches[2]
    }
}

# --- 5. Конфигурация, сборка, установка --------------------------------------
$clangCl = if ($env:CLANGCL) { $env:CLANGCL } else { 'clang-cl' }
try {
    $clangClPath = (Get-Command $clangCl -ErrorAction Stop).Source
} catch {
    throw ("clang-cl не найден ($clangCl). Установите компонент Microsoft.VisualStudio.Component.VC.Llvm.Clang " +
           'или задайте переменную окружения CLANGCL.')
}
Write-Host "      clang-cl: $clangClPath"

if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) {
    throw 'cmake не найден в PATH — установите CMake и повторите запуск.'
}

Write-Host '[5/5] Конфигурация Release (NMake Makefiles) ...' -ForegroundColor Green
$cmakeArgs = @(
    '-S', $SrcDir,
    '-B', $BuildDir,
    '-G', 'NMake Makefiles',
    '-DCMAKE_C_COMPILER=clang-cl',
    '-DCMAKE_CXX_COMPILER=clang-cl',
    '-DCMAKE_BUILD_TYPE=Release',
    '-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreadedDLL',
    '-DCMAKE_SYSTEM_VERSION=10.0.26100.0',
    '-DBUILD_STATIC=ON',
    '-DBUILD_SHARED=OFF',
    '-DBUILD_UNITTESTS=OFF',
    '-DBUILD_EXAMPLES=OFF',
    '-DBUILD_BENCHMARKS=OFF',
    '-DWITH_OPENMP=OFF',
    '-DWITH_NTL=OFF',
    '-DBUILD_EXTRAS=OFF',
    "-DCMAKE_INSTALL_PREFIX=$InstallDir"
)
& cmake @cmakeArgs
if ($LASTEXITCODE -ne 0) { throw "cmake configure завершился с кодом $LASTEXITCODE" }

Write-Host '      Сборка (это долго) ...' -ForegroundColor Green
& cmake --build $BuildDir --config Release
if ($LASTEXITCODE -ne 0) { throw "cmake --build завершился с кодом $LASTEXITCODE" }

Write-Host '      Установка префикса ...' -ForegroundColor Green
& cmake --install $BuildDir --config Release
if ($LASTEXITCODE -ne 0) { throw "cmake --install завершился с кодом $LASTEXITCODE" }

if (-not (Test-Path -LiteralPath (Join-Path $InstallDir 'lib'))) {
    throw "После установки каталог $(Join-Path $InstallDir 'lib') отсутствует — проверьте вывод cmake --install."
}

Write-Host ''
Write-Host '=== Готово: OpenFHE установлен ===' -ForegroundColor Cyan
Write-Host "Префикс: $InstallDir"
Write-Host 'Дальше (MSVC-рантайм в PATH, см. README.md «Запуск тестов»):'
Write-Host "  `$env:OPENFHE_HOME = '$InstallDir'"
Write-Host '  cargo test --release'
