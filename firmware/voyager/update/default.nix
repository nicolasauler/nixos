# Pulls the latest revision of the pinned Oryx layout into ../oryx (see
# src/main.rs); cargo test runs in checkPhase, without network.
{
  lib,
  rustPlatform,
}:
rustPlatform.buildRustPackage {
  pname = "voyager-layout-update";
  version = "0.1.0";
  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [./Cargo.toml ./Cargo.lock ./src];
  };
  cargoLock.lockFile = ./Cargo.lock;
  meta = {
    description = "Vendor the latest Oryx source export of the pinned Voyager layout";
    mainProgram = "voyager-layout-update";
  };
}
