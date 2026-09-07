//! Общий код для раздельных утилит ролей (src/bin/{lab,server,hospital}).
//!
//! Подключается в каждый бинарь через `#[path = "../demo_common.rs"] mod common;`
//! (не входит в публичное API библиотеки).

#![allow(dead_code)] // каждый из трёх бинарей использует своё подмножество

use std::fs;
use std::path::{Path, PathBuf};

use hepre::{Ciphertext, Context, CkksParams};

pub const CKKS_PARAMS: CkksParams = CkksParams { batch_size: 8192, multiplicative_depth: 3, scaling_mod_size: 40 };
pub const BATCH: usize = 8192;

const MAGIC: u32 = 0x4850_4550; // "EHPE"
const VERSION: u16 = 1;
pub const KIND_FEATURES: u16 = 0;
pub const KIND_SCORES: u16 = 1;

pub fn make_context() -> Context {
    Context::new_ckks(CKKS_PARAMS).expect("ctx_new_ckks")
}

// ---------------------------------------------------------------------------
// Inline JSON: модель и пациенты
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Model {
    pub weights: Vec<f64>,
    pub intercept: f64,
    pub threshold: f64,
}

impl Model {
    pub fn score_plain(&self, x: &[f64]) -> f64 {
        self.weights.iter().zip(x.iter()).map(|(w, v)| w * v).sum::<f64>() + self.intercept
    }
}

impl Default for Model {
    fn default() -> Self {
        Self {
            weights: vec![0.22, 0.14, 0.08, 0.03, 0.10],
            intercept: -1.70,
            threshold: -1.45,
        }
    }
}

pub fn parse_model(s: &str) -> Result<Model, String> {
    serde_json::from_str(s).map_err(|e| format!("не удалось разобрать --model JSON: {e}"))
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Patient {
    #[serde(default)]
    pub name: String,
    pub features: Vec<f64>,
}

pub fn parse_patients(s: &str) -> Result<Vec<Patient>, String> {
    let mut v: Vec<Patient> = serde_json::from_str(s).map_err(|e| format!("не удалось разобрать --patients JSON: {e}"))?;
    for (i, p) in v.iter_mut().enumerate() {
        if p.name.is_empty() {
            p.name = format!("P{}", i + 1);
        }
    }
    Ok(v)
}

pub fn default_patients_json() -> String {
    r#"[
        {"name": "P1", "features": [0.50, 0.40, 0.20, 0.30, 0.00]},
        {"name": "P2", "features": [0.80, 0.70, 0.50, 0.60, 1.00]},
        {"name": "P3", "features": [0.10, 0.20, 0.10, 0.10, 0.00]},
        {"name": "P4", "features": [0.70, 0.90, 0.70, 0.80, 1.00]},
        {"name": "P5", "features": [0.30, 0.60, 0.30, 0.50, 0.00]}
    ]"#
    .to_string()
}

pub fn default_model_json() -> String {
    serde_json::to_string(&Model::default()).expect("default model json")
}

// ---------------------------------------------------------------------------
// Файловый handoff: бинарный контейнер для ключей/шифротекстов
// ---------------------------------------------------------------------------

pub struct Handoff {
    pub kind: u16,
    pub n: u32,
    pub f: u32,
    pub blobs: Vec<Vec<u8>>,
}

pub fn write_handoff(path: &Path, kind: u16, n: usize, f: usize, blobs: &[Vec<u8>]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            fs::create_dir_all(dir).map_err(|e| format!("create_dir_all {}: {e}", dir.display()))?;
        }
    }
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC.to_le_bytes());
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&(n as u32).to_le_bytes());
    out.extend_from_slice(&(f as u32).to_le_bytes());
    out.extend_from_slice(&(blobs.len() as u32).to_le_bytes());
    for b in blobs {
        out.extend_from_slice(&(b.len() as u64).to_le_bytes());
        out.extend_from_slice(b);
    }
    fs::write(path, &out).map_err(|e| format!("запись {}: {e}", path.display()))?;
    Ok(())
}

pub fn read_handoff(path: &Path) -> Result<(u16, u32, u32, Vec<Vec<u8>>), String> {
    let bytes = fs::read(path).map_err(|e| format!("чтение {}: {e}", path.display()))?;
    let mut p = 0usize;
    let read_u32 = |p: &mut usize| -> Result<u32, String> {
        if *p + 4 > bytes.len() {
            return Err(format!("{}: контейнер повреждён (ожидан u32)", path.display()));
        }
        let v = u32::from_le_bytes(bytes[*p..*p + 4].try_into().unwrap());
        *p += 4;
        Ok(v)
    };
    let read_u16 = |p: &mut usize| -> Result<u16, String> {
        if *p + 2 > bytes.len() {
            return Err(format!("{}: контейнер повреждён (ожидан u16)", path.display()));
        }
        let v = u16::from_le_bytes(bytes[*p..*p + 2].try_into().unwrap());
        *p += 2;
        Ok(v)
    };
    let read_u64 = |p: &mut usize| -> Result<u64, String> {
        if *p + 8 > bytes.len() {
            return Err(format!("{}: контейнер повреждён (ожидан u64)", path.display()));
        }
        let v = u64::from_le_bytes(bytes[*p..*p + 8].try_into().unwrap());
        *p += 8;
        Ok(v)
    };

    if read_u32(&mut p)? != MAGIC {
        return Err(format!("{}: не handoff-файл hepre (нет MAGIC)", path.display()));
    }
    if read_u16(&mut p)? != VERSION {
        return Err(format!("{}: не поддерживаемая версия контейнера", path.display()));
    }
    let kind = read_u16(&mut p)?;
    let n = read_u32(&mut p)?;
    let f = read_u32(&mut p)?;
    let n_blobs = read_u32(&mut p)? as usize;
    let mut blobs = Vec::with_capacity(n_blobs);
    for i in 0..n_blobs {
        let len = read_u64(&mut p)? as usize;
        if p + len > bytes.len() {
            return Err(format!("{}: контейнер повреждён (blob {i} выходит за границы)", path.display()));
        }
        blobs.push(bytes[p..p + len].to_vec());
        p += len;
    }
    Ok((kind, n, f, blobs))
}

pub fn blobs_to_ciphertexts(cc: &Context, blobs: &[Vec<u8>]) -> Result<Vec<Ciphertext>, String> {
    blobs
        .iter()
        .map(|b| cc.deserialize_ciphertext(b).map_err(|e| format!("deserialize_ciphertext: {e}")))
        .collect()
}

pub fn ciphertexts_to_blobs(cts: &[&Ciphertext]) -> Result<Vec<Vec<u8>>, String> {
    cts.iter().map(|ct| ct.serialize().map_err(|e| format!("serialize: {e}"))).collect()
}

// ---------------------------------------------------------------------------
// Утилиты
// ---------------------------------------------------------------------------

pub fn resolve_dir(dir: &str) -> PathBuf {
    let p = PathBuf::from(dir);
    if p.as_os_str().is_empty() {
        PathBuf::from("handoff")
    } else {
        p
    }
}

pub fn kb(bytes: usize) -> String {
    format!("{:.1} КБ", bytes as f64 / 1024.0)
}

pub fn err_exit<T: std::fmt::Display>(msg: T) -> ! {
    eprintln!("ОШИБКА: {msg}");
    std::process::exit(1);
}