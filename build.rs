use std::env;
use std::path::PathBuf;
use std::process::Command;

fn var_or(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_string())
}

/// Записывает в бинарник версию компилятора и путь к OpenFHE.
///
/// Нужно для отпечатка окружения (`src/env.rs`): артефакты исследования
/// должны быть привязаны к конкретному инструментарию, а не только к
/// ревизии кода.
fn emit_environment_metadata(openfhe: &str) {
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let version = Command::new(&rustc)
        .arg("-V")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=HEPRE_RUSTC_VERSION={version}");
    println!("cargo:rustc-env=HEPRE_OPENFHE_HOME={openfhe}");
    println!("cargo:rerun-if-env-changed=RUSTC");
}

fn main() {
    // Путь по умолчанию вычисляется от корня крейта, а не задаётся
    // абсолютным: иначе репозиторий собирается только на машине автора.
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let default_openfhe = manifest_dir
        .join("vendor")
        .join("openfhe-install")
        .to_string_lossy()
        .replace('\\', "/");

    let openfhe = var_or("OPENFHE_HOME", &default_openfhe);
    let clang_cl = var_or(
        "CLANGCL",
        "C:/Program Files/Microsoft Visual Studio/2022/Community/VC/Tools/Llvm/x64/bin/clang-cl.exe",
    );
    let clang_rt = var_or(
        "CLANG_RT_DIR",
        "C:/Program Files/Microsoft Visual Studio/2022/Community/VC/Tools/Llvm/x64/lib/clang/19/lib/windows",
    );

    emit_environment_metadata(&openfhe);

    let mut bridge = cxx_build::bridge("src/ffi.rs");
    bridge
        .compiler(&clang_cl)
        .cpp(true)
        .file("cpp/openfhe-shim.cc")
        .include(PathBuf::from("cpp"))
        .include(format!("{openfhe}/include"))
        .include(format!("{openfhe}/include/openfhe/pke"))
        .include(format!("{openfhe}/include/openfhe/core"))
        .include(format!("{openfhe}/include/openfhe/binfhe"))
        .include(format!("{openfhe}/include/openfhe"))
        .flag("/MD")
        .flag("/std:c++17")
        .flag("/EHsc")
        .flag("/D_USE_MATH_DEFINES")
        // Заголовки OpenFHE порождают сотни предупреждений о неиспользуемых
        // параметрах виртуальных методов; они не относятся к нашему коду и
        // прячут полезный сигнал в журнале сборки.
        .flag("-Wno-unused-parameter");
    bridge.compile("hepre_openfhe");

    println!("cargo:rustc-link-search=native={openfhe}/lib");
    println!("cargo:rustc-link-search=native={clang_rt}");
    println!("cargo:rustc-link-lib=static=OPENFHEbinfhe_static");
    println!("cargo:rustc-link-lib=static=OPENFHEcore_static");
    println!("cargo:rustc-link-lib=static=OPENFHEpke_static");
    println!("cargo:rustc-link-lib=static=clang_rt.builtins-x86_64");

    println!("cargo:rerun-if-env-changed=OPENFHE_HOME");
    println!("cargo:rerun-if-env-changed=CLANGCL");
    println!("cargo:rerun-if-env-changed=CLANG_RT_DIR");
    println!("cargo:rerun-if-changed=cpp/openfhe-shim.h");
    println!("cargo:rerun-if-changed=cpp/openfhe-shim.cc");
    println!("cargo:rerun-if-changed=src/ffi.rs");
}
