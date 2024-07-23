let
  rust_overlay = import (builtins.fetchTarball "https://github.com/oxalica/rust-overlay/archive/master.tar.gz");
  pkgs = import <nixpkgs> {overlays = [rust_overlay];};
  # rust = pkgs.rust-bin.fromRustupToolchainFile ./master/rust-toolchain.toml;
  rust = pkgs.rust-bin.selectLatestNightlyWith (toolchain: toolchain.default);
in
  pkgs.mkShell {
    buildInputs =
      [
        rust
      ]
      ++ (with pkgs; [
        protobuf
        pkg-config
        openssl
        postgresql
        grpcurl
        python3
      ]);
    RUST_BACKTRACE = 1;
    shellHook = ''
      export PATH="$HOME/.cargo/bin:$PATH"
    '';
  }
