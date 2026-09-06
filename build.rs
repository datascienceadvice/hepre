use std::env;
use std::path::PathBuf;

fn var_or(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_string())
}

fn main() {
    let openfhe = var_or(
        "OPENFHE_HOME",
        "D:/lair/hepre/vendor/openfhe-install",
    );
    let clang_cl = var_or(
        "CLANGCL",
        "C:/Program Files/Microsoft Visual Studio/2022/Community/VC/Tools/Llvm/x64/bin/clang-cl.exe",
    );
    let clang_rt = var_or(
        "CLANG_RT_DIR",
        "C:/Program Files/Microsoft Visual Studio/2022/Community/VC/Tools/Llvm/x64/lib/clang/19/lib/windows",
    );

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
        .flag("/D_USE_MATH_DEFINES");
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