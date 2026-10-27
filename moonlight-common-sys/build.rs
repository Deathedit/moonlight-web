use std::{env, path::PathBuf};

const SRC: &str = "moonlight-common-c";

fn main() {
    let src = PathBuf::from(SRC);
    if !src.join("CMakeLists.txt").exists() {
        panic!("moonlight-common-c submodule missing — run `git submodule update --init --recursive`");
    }

    // Build the static library + enet; there is no install target, so pull
    // the archives straight out of the cmake build directory.
    let dst = cmake::Config::new(&src)
        .define("BUILD_SHARED_LIBS", "OFF")
        .define("CMAKE_POSITION_INDEPENDENT_CODE", "ON")
        .build_target("moonlight-common-c")
        .build();
    let build = dst.join("build");

    println!("cargo:rustc-link-search=native={}", build.display());
    println!("cargo:rustc-link-search=native={}", build.join("enet").display());
    println!("cargo:rustc-link-lib=static=moonlight-common-c");
    println!("cargo:rustc-link-lib=static=enet");
    println!("cargo:rerun-if-changed={SRC}/src");
    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-changed=shim.c");

    // Always optimise: Nix's hardening enables _FORTIFY_SOURCE, which warns at -O0.
    cc::Build::new().file("shim.c").opt_level(2).warnings(true).compile("mlw_shim");

    let bindings = bindgen::Builder::default()
        .header("wrapper.h")
        .clang_arg(format!("-I{}", src.join("src").display()))
        // Everything declared in Limelight.h (functions, structs, #defines),
        // nothing from libc headers it pulls in.
        .allowlist_file(".*Limelight\\.h")
        .derive_default(true)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("bindgen failed for Limelight.h");

    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out.join("bindings.rs"))
        .expect("could not write bindings");
}
