{
  lib,
  rustPlatform,
  pkg-config,
  makeWrapper,
  pipewire,
  wireplumber,
  libnotify,
}:
# Include the isolated PipeWire regression: no sound card, session bus or
# user's PipeWire is reachable from the build sandbox.
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
  # Only the keyboard daemon sends notifications; status/waybar are read-only.
  postInstall = ''
    wrapProgram $out/bin/talkd --prefix PATH : ${lib.makeBinPath [libnotify]}
  '';
  meta.mainProgram = "talkd";
}
