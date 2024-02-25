{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  buildInputs = with pkgs; [
    cargo
    cargo-watch
    rustc
    rust-analyzer
    rustfmt
    openssl
    pkg-config
    sqlite
  ];
}
