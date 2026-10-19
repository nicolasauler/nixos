# Run directly as a continuous Waybar JSON custom module; cargo test runs in
# buildRustPackage's checkPhase, without network or hardware access.
{
  lib,
  rustPlatform,
}:
rustPlatform.buildRustPackage {
  pname = "waybar-cores";
  version = "0.1.0";
  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [./Cargo.toml ./Cargo.lock ./src];
  };
  cargoLock.lockFile = ./Cargo.lock;
  doCheck = true;
  meta = {
    description = "Live usage of every observed logical CPU for Waybar";
    mainProgram = "waybar-cores";
  };
}
