{
  lib,
  rustPlatform,
  pkg-config,
  makeWrapper,
  pipewire,
  wireplumber,
  libnotify,
}:
# cargo test runs in buildRustPackage's checkPhase: a failing test means no
# package. --include-ignored adds the PipeWire test (src/mic.rs), which
# spawns its own pipewire and wireplumber inside the build sandbox: no
# sound card, no session bus, no user's PipeWire to reach.
rustPlatform.buildRustPackage {
  pname = "talkd";
  version = "0.1.0";
  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [./Cargo.toml ./Cargo.lock ./src ./tests];
  };
  cargoLock.lockFile = ./Cargo.lock;
  nativeBuildInputs = [pkg-config rustPlatform.bindgenHook makeWrapper];
  buildInputs = [pipewire];
  checkFlags = ["--include-ignored"];
  PW_TEST_PIPEWIRE = lib.getExe' pipewire "pipewire";
  PW_TEST_WIREPLUMBER = lib.getExe' wireplumber "wireplumber";
  # notify-send reaches mako for the errors that leave the mic flipped
  postInstall = ''
    wrapProgram $out/bin/talkd --prefix PATH : ${lib.makeBinPath [libnotify]}
  '';
  meta.mainProgram = "talkd";
}
