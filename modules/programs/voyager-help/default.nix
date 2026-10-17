{
  config,
  lib,
  pkgs,
  ...
}: let
  firmware = pkgs.callPackage ../../../firmware/voyager {};
  reference = pkgs.rustPlatform.buildRustPackage {
    pname = "voyager-help";
    version = "0.1.0";
    src = lib.fileset.toSource {
      root = ./.;
      fileset = lib.fileset.unions [./Cargo.toml ./Cargo.lock ./src];
    };
    cargoLock.lockFile = ./Cargo.lock;
    ROFI_BIN = lib.getExe config.programs.rofi.finalPackage;
    VOYAGER_PROFILE_JSON = firmware.referenceProfile.file;
    meta = {
      description = "Search the USB-reported Voyager revision's Oryx mappings";
      mainProgram = "voyager-help";
    };
  };
in {
  home.packages = [reference];

  xdg.desktopEntries.voyager-keyboard-reference = {
    name = "Voyager Keyboard Reference";
    genericName = "Keyboard Reference";
    comment = "Search mappings generated from the connected Voyager's Oryx revision";
    exec = lib.getExe reference;
    icon = "input-keyboard";
    terminal = false;
    startupNotify = false;
    categories = ["Utility"];
    settings.Keywords = "Voyager;Keyboard;Keymap;Reference;Help;";
  };
}
