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
  pin = lib.importJSON ./oryx/oryx.json;
  swap = swapOnLayer != null;
  alias = swap && mirroredNavLayer != null && navLayer != null;

  # ZSA's fork at the head of branch firmware25 when pinned: the branch for
  # Oryx's firmware 25, which this revision targets (`firmware` in
  # oryx/oryx.json). Oryx does not publish the exact commit it builds with.
  # Plus the submodules a Voyager build reads; the fork's .gitmodules also
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
  # Checked at evaluation, from the vendored keymap.
  #
  # Its layers, by index: {"0" = [keys]; …}, read from a strict subset of C
  # that Oryx's export falls in; anything outside it is refused, never
  # guessed at, so a keymap the parser cannot read never passes as "layer N is
  # absent":
  #
  # - Physical lines. C splices backslash-newline before it sees comments,
  #   literals or identifiers; this reader does not, so it refuses any CR
  #   byte (Oryx writes LF only, so LF is the only line end) and any
  #   backslash followed by whitespace that contains an LF (a superset of
  #   GCC's continuations, which allow spaces, tabs, FF and VT before the
  #   line end, and of `\\` + LF, where the last backslash splices).
  #   Trigraphs are not replaced, as GCC does not replace them either under
  #   QMK's `-std=gnu11` (builddefs/common_rules.mk:37 at 93b2b9ec).
  # - Comments and literals. keymap.c is lexed just enough to tell code from
  #   comments (`/* … */`, `// …`) and string and character literals (with
  #   escapes); each becomes "@", which code cannot otherwise hold. One never
  #   closed is refused.
  # - The declaration. In the code that remains, `keymaps` must occur exactly
  #   once, as `keymaps[][MATRIX_ROWS][MATRIX_COLS] = {` after whitespace
  #   (so no longer identifier ending in it, nor a second copy anywhere, in a
  #   function body or `#if 0`, can stand in). The array runs to the first
  #   ";" after it (an initialiser list holds none), which must follow its
  #   closing "}". A comment or literal inside it is refused.
  # - Its layers. The array must be exactly a comma-separated list of
  #   designated initialisers `[N] = LAYOUT…( … )`: N decimal in its canonical
  #   spelling (no leading zero), any layout macro name, any spacing, each
  #   call's parentheses balanced. Anything else before, between or after
  #   them (an initialiser without an index, `[07]`, `[NAV]`, a braced
  #   `{{ … }}` layer) or one index twice is refused.
  # - Its keys: the macro's top-level arguments, of every layer, may hold only
  #   letters, digits, `_`, whitespace, `(`, `)`, `,` and `|`. So no brace,
  #   bracket or digraph (`<%`, `%>`, `<:`, `:>`, `%:`) in a key can close a
  #   layer early and slip in an initialiser C would see and this would not.
  #
  # The preprocessor is not run: macros are not expanded and conditional
  # inclusion is not evaluated; keymap.c is read as written, so a definition,
  # here or in a header, that changes what `keymaps`, `LAYOUT…` or a
  # transparent keycode means is not seen.
  layers = let
    refuse = why: throw "voyager-firmware: oryx/keymap.c: ${why}; the layer guards cannot read this keymap";
    quote = text: "`${lib.substring 0 60 (lib.trim (builtins.replaceStrings ["\n" "\r"] [" " " "] text))}`";
    raw = builtins.readFile ./oryx/keymap.c;
    source =
      if lib.hasInfix "\r" raw
      then refuse "a CR byte (only LF line ends are read)"
      else if builtins.length (builtins.split "\\\\[[:space:]]*\n" raw) > 1
      then refuse "a backslash line continuation (lines are not spliced here)"
      else raw;
    escaped = "\\\\.";
    tokens =
      builtins.split (lib.concatStringsSep "|" [
        "/\\*([^*]|\\*+[^*/])*\\*+/"
        "//([^\\\\\n]|${escaped})*"
        "\"([^\"\\\\\n]|${escaped})*\""
        "'([^'\\\\\n]|${escaped})*'"
      ])
      source;
    pieces = lib.filter builtins.isString tokens;
    code = lib.concatStrings (map (token:
      if builtins.isList token
      then "@"
      else token)
    tokens);
    mentions = (builtins.length (builtins.split "keymaps" code) - 1) / 2;
    array = builtins.split "[[:space:]]keymaps\\[]\\[MATRIX_ROWS]\\[MATRIX_COLS][[:space:]]*=[[:space:]]*\\{" code;
    declaration = builtins.head (lib.splitString ";" (builtins.elemAt array 2));
    closed = builtins.match "([^;]*)}[[:space:]]*" declaration;
    body = builtins.head closed;
    parts = builtins.split "\\[[[:space:]]*(0|[1-9][0-9]*)[[:space:]]*][[:space:]]*=[[:space:]]*LAYOUT[[:alnum:]_]*[[:space:]]*\\(" body;
    count = (builtins.length parts - 1) / 2;
    # The rest of one layer after its "LAYOUT…(": the macro's arguments up to
    # the ")" that balances that "(", then only the "," before the next layer
    # (optional after the last). Returns the top-level arguments, trimmed.
    layer = index: last: rest: let
      step = state: token:
        if state.closed
        then
          if builtins.isList token
          then refuse "after layer ${index}: ${quote (state.tail + builtins.head token)} is not `, [N] = LAYOUT…(`"
          else state // {tail = state.tail + token;}
        else if builtins.isList token
        then let
          paren = builtins.head token;
          depth =
            state.depth
            + (
              if paren == "("
              then 1
              else -1
            );
        in
          if depth == 0
          then
            state
            // {
              closed = true;
              depth = 0;
              keys = state.keys ++ [state.key];
            }
          else
            state
            // {
              inherit depth;
              key = state.key + paren;
            }
        else if state.depth == 1
        then let
          # commas at the top level end a key
          pieces = lib.splitString "," token;
          n = builtins.length pieces;
        in
          if n == 1
          then state // {key = state.key + token;}
          else
            state
            // {
              keys = state.keys ++ [(state.key + builtins.head pieces)] ++ lib.sublist 1 (n - 2) pieces;
              key = lib.last pieces;
            }
        else state // {key = state.key + token;};
      end = builtins.foldl' step {
        closed = false;
        depth = 1;
        key = "";
        keys = [];
        tail = "";
      } (builtins.split "([()])" rest);
    in
      if !end.closed
      then refuse "layer ${index}: its LAYOUT…( has no balancing )"
      else if
        builtins.match (
          if last
          then "[[:space:]]*,?[[:space:]]*"
          else "[[:space:]]*,[[:space:]]*"
        )
        end.tail
        == null
      then refuse "after layer ${index}: ${quote end.tail} is not `, [N] = LAYOUT…(`"
      else let
        keys = map lib.trim end.keys;
        odd = lib.findFirst (key: builtins.match "[A-Za-z0-9_[:space:](),|]*" key == null) null keys;
      in
        if odd != null
        then refuse "layer ${index}: key ${quote odd} holds a character other than letters, digits, `_`, whitespace, `(`, `)`, `,` and `|`"
        else keys;
    entries =
      lib.genList (i: let
        index = builtins.head (builtins.elemAt parts (2 * i + 1));
      in {
        name = index;
        value = layer index (i == count - 1) (builtins.elemAt parts (2 * i + 2));
      })
      count;
    indices = map (entry: entry.name) entries;
  in
    # what is left of a comment or literal start in code was never closed
    if lib.any (piece: lib.any (start: lib.hasInfix start piece) ["/*" "//" "\"" "'" "@"]) pieces
    then refuse "an unterminated comment or literal, or an @ in code"
    else if mentions != 1 || builtins.length array != 3
    then refuse "`keymaps` must appear exactly once in code (outside comments and literals), as the declaration `keymaps[][MATRIX_ROWS][MATRIX_COLS] = {` after whitespace"
    else if closed == null
    then refuse "the keymaps array does not end in `}` before its `;`"
    else if lib.hasInfix "@" body
    then refuse "a comment or literal inside the keymaps array"
    else if count == 0
    then refuse "no `[N] = LAYOUT…(` layer"
    else if builtins.match "[[:space:]]*" (builtins.head parts) == null
    then refuse "${quote (builtins.head parts)} before the first `[N] = LAYOUT…(`"
    else if lib.unique indices != indices
    then refuse "a layer index given twice"
    # every layer is parsed, not only the ones asked for: a refusal hides in
    # any of them
    else builtins.deepSeq entries (builtins.listToAttrs entries);
  hasLayer = layer: layers ? ${toString layer};

  layerKeys = layer:
    layers.${toString layer} or (throw "voyager-firmware: layer ${toString layer} is not in oryx/keymap.c");
  opaqueKeys = lib.unique (lib.filter (key: !(builtins.elem key ["KC_TRANSPARENT" "KC_TRNS" "_______"])) (layerKeys swapOnLayer));
  checked = lib.assertMsg (opaqueKeys == []) ''
    voyager-firmware: layer ${toString swapOnLayer} must be fully transparent to mirror the keyboard, but has ${toString (builtins.length opaqueKeys)} distinct keys: ${lib.concatStringsSep ", " opaqueKeys}.
    In Oryx, make every key of that layer transparent (or replace the layer with a new, empty one), save, and run `nix run .#voyager-layout-update`.'';

  # The alias never reaches layer_state, so a keymap layer at its index (say a
  # layer added later in Oryx) would be unreachable. The module goes first in
  # the module list so the Oryx module (which reports the layer over raw HID)
  # and the keymap see the rewritten state, never the alias.
  aliasChecked =
    lib.assertMsg (!hasLayer mirroredNavLayer) ''
      voyager-firmware: mirroredNavLayer ${toString mirroredNavLayer} is a layer in oryx/keymap.c; the alias must be an index the layout does not use.''
    && lib.assertMsg (hasLayer navLayer) ''
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
