{pkgs ? import <nixpkgs> {}}:
pkgs.mkShell {
  nativeBuildInputs = with pkgs; [
    alejandra
    python3
    claude-code
    #nixpkgs-fmt
    # cargo
    #nodejs_latest
  ];
}
