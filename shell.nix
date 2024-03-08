{ pkgs ? import <nixpkgs> { } }:
pkgs.mkShell {
  nativeBuildInputs = with pkgs; [
    unzip
    python3
    cargo
    nodejs_latest
  ];
}
