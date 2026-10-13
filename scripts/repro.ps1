<#
.SYNOPSIS
    Однокомандное воспроизведение исследования HEPRE.

.DESCRIPTION
    Последовательно выполняет:
      0. проверку, что вендорный OpenFHE установлен;
      1. сборку Release (крэйт + бинарники ролей);
      2. интеграционный тест-набор (cargo test --release);
      3. раннер исследования `study` с конфигурацией configs\study.json;
      4. печать пути, куда записаны машинно-читаемые результаты.

    MSVC-рантайм добавляется в PATH автоматически (иначе запуск exe падает
    с 0xC0000135: не найдены MSVCP140.dll / VCRUNTIME140*.dll).

    Скрипт ничего не «дочиняет» руками: числа и артефакты получаются только
    прогоном раннера. См. CONTRIBUTING.md, раздел о численных результатах.

.EXAMPLE
    pwsh -NoProfile -File scripts/repro.ps1
#>

[CmdletBinding()]
param(
    [string]$Config = 'configs/study.json'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$RepoRoot   = Split-Path -Parent $PSScriptRoot
$InstallLib = Join-Path $RepoRoot 'vendor\openfhe-install\lib'
$ResultsDir = Join-Path $RepoRoot 'results'

function Invoke-Native {
    param(
        [Parameter(Mandatory = $true)][string]$What,
        [Parameter(Mandatory = $true)][scriptblock]$Command
    )
    Write-Host "==> $What" -ForegroundColor Cyan
    & $Command
    if ($LASTEXITCODE -ne 0) {
        throw "${What}: команда завершилась с кодом $LASTEXITCODE"
    }
}

Write-Host '=== HEPRE: воспроизведение исследования ===' -ForegroundColor Cyan
Write-Host "Корень репозитория: $RepoRoot"

# --- 0. Fail fast: вендорный OpenFHE ----------------------------------------
if (-not (Test-Path -LiteralPath $InstallLib)) {
    throw ("Вендорный OpenFHE не установлен: отсутствует $InstallLib`n" +
           'Сначала соберите его: pwsh -NoProfile -File scripts/setup-openfhe.ps1`n' +
           'Каталог vendor\ в репозиторий не входит (слишком тяжёлый) — его нужно получить локально; ' +
           'подробности в README.md, раздел «Сборка OpenFHE».')
}
Write-Host "[0/4] Префикс OpenFHE найден: $InstallLib" -ForegroundColor Green

# --- MSVC runtime в PATH ------------------------------------------------------
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (Test-Path -LiteralPath $vswhere) {
    $vsRoot = (& $vswhere -latest -products * -property installationPath)
    if ($LASTEXITCODE -eq 0 -and $vsRoot) {
        $msvcRoot = Join-Path $vsRoot.Trim() 'VC\Tools\MSVC'
        if (Test-Path -LiteralPath $msvcRoot) {
            $msvcVer = Get-ChildItem -LiteralPath $msvcRoot -Directory | Sort-Object Name -Descending | Select-Object -First 1
            if ($msvcVer) {
                $rt = Join-Path $msvcVer.FullName 'bin\Hostx64\x64'
                if (Test-Path -LiteralPath $rt) {
                    $env:PATH = "$rt;$env:PATH"
                    Write-Host "      MSVC runtime: $rt"
                }
            }
        }
    }
} else {
    Write-Warning 'vswhere.exe не найден — MSVC-рантайм не добавлен в PATH; при 0xC0000135 добавьте его вручную (README.md).'
}

Push-Location $RepoRoot
try {
    # --- 1. Сборка Release ---------------------------------------------------
    Invoke-Native -What '[1/4] Сборка Release (крэйт + бинарники ролей)' -Command {
        cargo build --release --bins
    }

    # --- 2. Тесты ------------------------------------------------------------
    Invoke-Native -What '[2/4] Интеграционные тесты (release)' -Command {
        cargo test --release
    }

    # --- 3. Раннер исследования ---------------------------------------------
    $configPath = if ([System.IO.Path]::IsPathRooted($Config)) { $Config } else { Join-Path $RepoRoot $Config }
    if (-not (Test-Path -LiteralPath $configPath)) {
        throw ("Конфигурация исследования не найдена: $configPath`n" +
               'Раннер исследования запускается только по явной конфигурации; ' +
               'создайте configs\study.json (или передайте -Config <путь>) и повторите запуск. ' +
               'Нормативное описание прогона и его артефактов — docs\REPRODUCIBILITY.md, разделы 6 и 8.')
    }

    $releaseDir = Join-Path $RepoRoot 'target\release'
    $studyExe = Get-ChildItem -LiteralPath $releaseDir -Filter '*.exe' -File -ErrorAction SilentlyContinue |
        Where-Object { $_.BaseName -eq 'study' -or $_.BaseName -like '*study*' } |
        Select-Object -First 1
    if (-not $studyExe) {
        $candidates = @(
            (Join-Path $releaseDir 'study.exe'),
            (Join-Path $releaseDir 'hepre-study.exe')
        ) -join ', '
        throw ("Бинарник раннера исследования не найден в $releaseDir (ожидался один из: $candidates).`n" +
               'Раннер `study` входит в поставку прототипа (`src\bin\study.rs`); пока его нет в дереве, ' +
               'воспроизведение численных результатов невозможно — это зафиксированный пробел, ' +
               'см. docs\REPRODUCIBILITY.md (раздел 3.1) и CONTRIBUTING.md (раздел 5).')
    }

    Write-Host "[3/4] Раннер исследования: $($studyExe.FullName)" -ForegroundColor Cyan
    Write-Host "      Конфигурация: $configPath"
    Invoke-Native -What '      study --config' -Command {
        & $studyExe.FullName --config $configPath
    }

    # --- 4. Где результаты ---------------------------------------------------
    Write-Host ''
    Write-Host '=== Воспроизведение завершено ===' -ForegroundColor Cyan
    if (Test-Path -LiteralPath $ResultsDir) {
        $artifacts = Get-ChildItem -LiteralPath $ResultsDir -Recurse -File |
            Sort-Object LastWriteTime -Descending
        if ($artifacts) {
            Write-Host "Машинно-читаемые результаты: $ResultsDir\<run-id>\ (нормативная раскладка — docs\REPRODUCIBILITY.md, раздел 8)"
            Write-Host 'Последние записанные артефакты:'
            $artifacts | Select-Object -First 20 | ForEach-Object {
                Write-Host ("  {0}  ({1} байт, {2:yyyy-MM-dd HH:mm:ss})" -f
                    $_.FullName.Substring($ResultsDir.Length + 1), $_.Length, $_.LastWriteTime)
            }
            # Ожидаемые по docs\REPRODUCIBILITY.md обязательные артефакты прогона.
            $latestRun = Get-ChildItem -LiteralPath $ResultsDir -Directory |
                Sort-Object LastWriteTime -Descending | Select-Object -First 1
            if ($latestRun) {
                $required = @('manifest.json', 'environment.json', 'cohort.csv', 'model.json', 'results.json')
                $missing = $required | Where-Object { -not (Test-Path -LiteralPath (Join-Path $latestRun.FullName $_)) }
                if ($missing) {
                    Write-Warning ("В $($latestRun.FullName) нет обязательных артефактов: $($missing -join ', ') — " +
                                   'см. docs\REPRODUCIBILITY.md, раздел 8.')
                } else {
                    Write-Host "Проверка раскладки: $($latestRun.Name) содержит все обязательные артефакты; digest-ы сверяются по manifest.json."
                }
            }
        } else {
            Write-Warning "Каталог результатов пуст: $ResultsDir — проверьте конфигурацию $configPath."
        }
    } else {
        Write-Warning ("Каталог результатов не создан: $ResultsDir. " +
                       'Раннер исследования должен записывать туда машинно-читаемые артефакты (docs\REPRODUCIBILITY.md, раздел 8).')
    }
    Write-Host 'Числа из results\ не редактируются руками: перегенерируйте их этим скриптом (CONTRIBUTING.md, раздел 5).'
} finally {
    Pop-Location
}
