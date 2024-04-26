{ pkgs ? import <nixpkgs> { } }:
pkgs.mkShell {
  nativeBuildInputs = with pkgs; [
    nixpkgs-fmt
    python3
    # cargo
    #nodejs_latest
  ];
}
