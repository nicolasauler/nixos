# ZSA Voyager firmware: the Oryx layout vendored in ./oryx (refreshed by
# `nix run .#voyager-layout-update`), built with ZSA's QMK fork, plus a module
# that turns on QMK's Swap Hands while the mirror layer is active, so the foot
# pedal mirrors the whole keyboard rather than one hand-made layer.
#
# Flash: `nix run nixpkgs#zapp -- flash result/*.bin`, then press the reset
# button with a paperclip (the layout has no Reset key).
{
  lib,
  stdenv,
  fetchFromGitHub,
  writeText,
  qmk,
  # The layer whose activation mirrors the keyboard. Must match MIRROR in
  # modules/programs/foot-pedal/src/main.rs. null builds the layout as Oryx does.
  swapOnLayer ? 6,
  # Mirrored nav: an alias layer the module rewrites to navLayer + swapOnLayer,
  # so the pedal's layer_move (one layer on) can still give nav, mirrored. Must
  # match MIRRORED_NAV and NAV in modules/programs/foot-pedal/src/main.rs.
  # Either null (or swapOnLayer null) builds without the alias.
  mirroredNavLayer ? 7,
  navLayer ? 4,
}: let
  pin = lib.importJSON ./oryx.json;
  swap = swapOnLayer != null;
  alias = swap && mirroredNavLayer != null && navLayer != null;
  keymapC = builtins.readFile ./oryx/keymap.c;
  layout = layer: "[${toString layer}] = LAYOUT_voyager(";

  # ZSA's fork at branch firmware25 (the version Oryx compiles this layout
  # with), and the submodules a Voyager build reads. The fork's .gitmodules also
  # pins pico-sdk, lvgl, vusb and googletest: other MCUs and tests.
  github = owner: repo: rev: hash: fetchFromGitHub {inherit owner repo rev hash;};
  qmkFirmware = github "zsa" "qmk_firmware" "93b2b9ec3368f86c5eb5a2e3f934049f8daef885" "sha256-YMdZSfexFhynEBOMkawD35u6acVX6PnRoRXvdw28jfc=";
  submodules = {
    "lib/chibios" = github "qmk" "ChibiOS" "be44b3305f9a9fe5f2f49a4e7b978db322dc463e" "sha256-e0K+zXbTbfh/kP7JbTS624gGhtrKSg56yOfkq2salA8=";
    "lib/chibios-contrib" = github "qmk" "ChibiOS-Contrib" "77cb0a4f7589f89e724f5e6ecb1d76d514dd1212" "sha256-DHizIK4XWe3wnYC/MaeoQu+idL2lXApu7BxzcbchdYY=";
    # ChibiOS's USB layer includes LUFA's HID class headers (lufa_utils)
    "lib/lufa" = github "qmk" "lufa" "549b97320d515bfca2f95c145a67bd13be968faa" "sha256-BCaLSOn9ksj0+gYNdiTkZqrgKbGEbWNxnxrh3TOpsOY=";
    "lib/printf" = github "qmk" "printf" "c2e3b4e10d281e7f0f694d3ecbd9f320977288cc" "sha256-RgeQ1Hg1j9DF4C2N/0Qh6iGAFg8vAY1NdbxFMCcMoGQ=";
    "modules/zsa" = github "zsa" "qmk_modules" "13890cd7856175de20798689d15ba6a46bf0c5c7" "sha256-QrwXzNAtX7UakXz0jzL5R3vU1zbg6Juk9aO5daZN7+I=";
  };

  # Swap Hands mirrors key positions before any layer lookup, so the layer that
  # turns it on must be fully transparent: the layers below show through,
  # mirrored. A layer that mirrors by hand (as the Oryx "mirror" layer did)
  # would be mirrored twice and type exactly as if nothing were mirrored.
  # Checked at evaluation, from the vendored keymap. A transparent layer has
  # no nested parentheses, so the first ")" after its LAYOUT( ends it; any key
  # that is a function call fails the check anyway.
  layerKeys = layer: let
    after = lib.splitString (layout layer) keymapC;
  in
    if builtins.length after != 2
    then throw "voyager-firmware: layer ${toString layer} is not in oryx/keymap.c"
    else map lib.trim (lib.splitString "," (builtins.head (lib.splitString ")" (builtins.elemAt after 1))));
  opaqueKeys = lib.unique (lib.filter (key: !(builtins.elem key ["KC_TRANSPARENT" "KC_TRNS" "_______"])) (layerKeys swapOnLayer));
  checked = lib.assertMsg (opaqueKeys == []) ''
    voyager-firmware: layer ${toString swapOnLayer} must be fully transparent to mirror the keyboard, but has ${toString (builtins.length opaqueKeys)} distinct keys: ${lib.concatStringsSep ", " opaqueKeys}.
    In Oryx, make every key of that layer transparent (or replace the layer with a new, empty one), save, and run `nix run .#voyager-layout-update`.'';

  # The alias never reaches layer_state, so a keymap layer at its index (say a
  # layer added later in Oryx) would be unreachable. The module goes first in
  # the module list so the Oryx module (which reports the layer over raw HID)
  # and the keymap see the rewritten state, never the alias.
  aliasChecked =
    lib.assertMsg (!lib.hasInfix (layout mirroredNavLayer) keymapC) ''
      voyager-firmware: mirroredNavLayer ${toString mirroredNavLayer} is a layer in oryx/keymap.c; the alias must be an index the layout does not use.''
    && lib.assertMsg (lib.hasInfix (layout navLayer) keymapC) ''
      voyager-firmware: navLayer ${toString navLayer} is not in oryx/keymap.c.'';

  oryxKeymap = lib.importJSON ./oryx/keymap.json;
  keymapJson =
    writeText "keymap.json" (builtins.toJSON (oryxKeymap
        // lib.optionalAttrs swap {modules = ["nic/swap_on_layer"] ++ (oryxKeymap.modules or []);}));
  configH = writeText "config.h" (builtins.readFile ./oryx/config.h
    + lib.optionalString swap "\n#define SWAP_ON_LAYER ${toString swapOnLayer}\n"
    + lib.optionalString alias "#define MIRRORED_NAV_LAYER ${toString mirroredNavLayer}\n#define NAV_LAYER ${toString navLayer}\n");

  keymap = "keyboards/zsa/voyager/keymaps/oryx";
in
  stdenv.mkDerivation {
    pname = "voyager-firmware";
    version = "${pin.layout}-${pin.revision}" + lib.optionalString swap "-swap";
    src = qmkFirmware;

    nativeBuildInputs = [qmk];

    # forced when the derivation is instantiated (still evaluation time), not
    # when merely reaching .override
    postPatch = assert swap -> checked;
    assert alias -> aliasChecked;
      lib.concatStrings (lib.mapAttrsToList (path: source: "cp -r ${source}/. ${path}\n") submodules)
      + ''
        mkdir -p ${keymap}
        cp ${./oryx/keymap.c} ${keymap}/keymap.c
        cp ${./oryx/rules.mk} ${keymap}/rules.mk
        cp ${configH} ${keymap}/config.h
        cp ${keymapJson} ${keymap}/keymap.json
      ''
      + lib.optionalString swap ''
        mkdir -p modules/nic
        cp -r ${./swap_on_layer} modules/nic/swap_on_layer
      ''
      + ''
        chmod -R u+w lib modules ${keymap}
      '';

    buildPhase = ''
      runHook preBuild
      export HOME=$TMPDIR QMK_HOME=$PWD
      make zsa/voyager:oryx SKIP_GIT=yes -j$NIX_BUILD_CORES
      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall
      install -Dm444 zsa_voyager_oryx.bin $out/voyager-${pin.layout}-${pin.revision}${lib.optionalString swap "-swap"}.bin
      runHook postInstall
    '';

    meta = {
      description = "ZSA Voyager firmware: Oryx layout ${pin.layout}/${pin.revision}";
      platforms = lib.platforms.all;
    };
  }
