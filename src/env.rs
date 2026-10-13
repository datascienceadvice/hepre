//! Отпечаток окружения, в котором получены результаты.
//!
//! Воспроизводимость измерений невозможна без фиксации того, **чем** они
//! получены: версия компилятора, целевая платформа, путь к OpenFHE и
//! ревизия репозитория попадают в `environment.json` рядом с результатами.
//!
//! Модуль намеренно не тянет внешних зависимостей: время форматируется
//! собственной реализацией `civil_from_days` (алгоритм Х. Хиннанта), а
//! ревизия git читается прямо из `.git`, без запуска `git`.

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Версия компилятора, зафиксированная на этапе сборки (см. `build.rs`).
pub const RUSTC_VERSION: &str = match option_env!("HEPRE_RUSTC_VERSION") {
    Some(v) => v,
    None => "unknown",
};

/// Префикс установки OpenFHE, использованный при сборке.
pub const OPENFHE_HOME: &str = match option_env!("HEPRE_OPENFHE_HOME") {
    Some(v) => v,
    None => "unknown",
};

/// Отпечаток окружения.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnvironmentFingerprint {
    /// Версия компилятора Rust.
    pub rustc_version: String,
    /// Целевая платформа (`std::env::consts::ARCH`).
    pub arch: String,
    /// Семейство ОС (`std::env::consts::OS`).
    pub os: String,
    /// Полная строка версии ОС, если её удалось получить.
    pub os_version: String,
    /// Число логических процессоров.
    pub cpu_count: usize,
    /// Каталог установки OpenFHE, использованный при сборке.
    pub openfhe_home: String,
    /// Версия крейта.
    pub crate_version: String,
    /// Ревизия git рабочего дерева (или `unknown`).
    pub git_revision: String,
    /// Ветка git (или `unknown`).
    pub git_branch: String,
    /// Момент запуска в секундах Unix.
    pub started_unix: i64,
    /// Момент запуска в формате UTC `YYYY-MM-DDThh:mm:ssZ`.
    pub started_utc: String,
}

/// Возвращает текущее время в секундах Unix.
pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Преобразует число суток от эпохи в `(год, месяц, день)`.
///
/// Алгоритм `civil_from_days` Говарда Хиннанта: точен для всего диапазона
/// пролептического григорианского календаря.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Форматирует секунды Unix как строку UTC вида `2026-09-07T12:34:56Z`.
pub fn unix_to_utc(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// Читает текущую ветку и ревизию из каталога `.git` без запуска git.
///
/// Нужно для артефактов исследования: они должны быть привязаны к ревизии
/// кода, но сам раннер не обязан зависеть от установленного `git`.
pub fn git_info(repo_root: &Path) -> (String, String) {
    let git_dir = repo_root.join(".git");
    let head = match std::fs::read_to_string(git_dir.join("HEAD")) {
        Ok(s) => s.trim().to_string(),
        Err(_) => return ("unknown".to_string(), "unknown".to_string()),
    };

    if let Some(reference) = head.strip_prefix("ref: ") {
        let branch = reference
            .rsplit('/')
            .next()
            .unwrap_or(reference)
            .to_string();
        let sha = std::fs::read_to_string(git_dir.join(reference))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        (sha, branch)
    } else {
        // Отделённое состояние HEAD: в файле лежит сам хэш.
        (head, "detached".to_string())
    }
}

/// Собирает отпечаток окружения.
pub fn capture(repo_root: &Path) -> EnvironmentFingerprint {
    let started_unix = now_unix();
    let (git_revision, git_branch) = git_info(repo_root);

    EnvironmentFingerprint {
        rustc_version: RUSTC_VERSION.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        os: std::env::consts::OS.to_string(),
        os_version: std::env::consts::OS.to_string(),
        cpu_count: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0),
        openfhe_home: OPENFHE_HOME.to_string(),
        crate_version: env!("CARGO_PKG_VERSION").to_string(),
        git_revision,
        git_branch,
        started_unix,
        started_utc: unix_to_utc(started_unix),
    }
}

/// Идентификатор запуска: UTC-метка без разделителей плюс ревизия.
///
/// Служит именем каталога результатов, поэтому должен быть уникальным и
/// сортируемым лексикографически.
pub fn run_id(env: &EnvironmentFingerprint) -> String {
    let stamp = env
        .started_utc
        .replace(['-', ':'], "")
        .replace('T', "-")
        .trim_end_matches('Z')
        .to_string();
    let short = if env.git_revision.len() >= 8 {
        &env.git_revision[..8]
    } else {
        "norev"
    };
    format!("{stamp}_{short}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        // 2026-09-07 — дата выпуска версии 0.1.0 (проверено: 56 лет от
        // эпохи = 20454 дня до 2026-01-01, плюс 249 дней до 7 сентября).
        assert_eq!(civil_from_days(20_703), (2026, 9, 7));
        assert_eq!(civil_from_days(20_704), (2026, 9, 8));
        // Високосный день.
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    }

    #[test]
    fn unix_to_utc_formats_correctly() {
        assert_eq!(unix_to_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(unix_to_utc(1_000_000_000), "2001-09-09T01:46:40Z");
        // 1_786_000_000 = 20671 суток + 25 600 с → 07:06:40 UTC.
        assert_eq!(unix_to_utc(1_786_000_000), "2026-08-06T07:06:40Z");
        // Полночь и конец суток на границе.
        assert_eq!(unix_to_utc(86_400), "1970-01-02T00:00:00Z");
        assert_eq!(unix_to_utc(86_399), "1970-01-01T23:59:59Z");
    }

    #[test]
    fn git_info_reads_this_repository() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let (sha, branch) = git_info(root);
        // Репозиторий существует и находится в обычном (не detached) состоянии.
        assert_eq!(sha.len(), 40, "ревизия = {sha}");
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!branch.is_empty());
    }

    #[test]
    fn git_info_on_missing_repo_degrades_gracefully() {
        let (sha, branch) = git_info(Path::new("D:/definitely/not/a/repo"));
        assert_eq!(sha, "unknown");
        assert_eq!(branch, "unknown");
    }

    #[test]
    fn run_id_is_filename_safe() {
        let env = EnvironmentFingerprint {
            rustc_version: "rustc test".into(),
            arch: "x86_64".into(),
            os: "windows".into(),
            os_version: "windows".into(),
            cpu_count: 8,
            openfhe_home: "x".into(),
            crate_version: "0.1.0".into(),
            git_revision: "0123456789abcdef0123456789abcdef01234567".into(),
            git_branch: "master".into(),
            started_unix: 1_786_000_000,
            started_utc: unix_to_utc(1_786_000_000),
        };
        let id = run_id(&env);
        assert_eq!(id, "20260806-070640_01234567");
        assert!(id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }
}
