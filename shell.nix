# Dev environment for Moonlight Web. Works with plain `nix-shell`, and the
# flake's devShell imports this file, so both stay identical.
{ pkgs ? import <nixpkgs> { } }:

pkgs.mkShell {
  packages = with pkgs; [
    # Rust bridge
    rustc
    cargo
    clippy
    rustfmt
    rust-analyzer
    # moonlight-common-c (C, built via cmake from build.rs)
    cmake
    pkg-config
    openssl
    # Web UI
    nodejs_22
  ];

  # bindgen needs libclang
  LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
  RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
}
