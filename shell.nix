{pkgs ? import <nixpkgs> {}}:
pkgs.mkShell {
  nativeBuildInputs = with pkgs; [
    alejandra
    python3
    #nixpkgs-fmt
    # cargo
    #nodejs_latest
  ];
}
