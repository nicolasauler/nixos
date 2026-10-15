# Frozen Oryx export JqpmQ/bvrAW7: keymap.c, config.h, rules.mk, oryx.json.
# Update this snapshot when the actual exported layout changes. This is NOT live
# device detection. Recipes start from neutral, unmirrored base (Colemak legends).
# Transparent fall-through routes are not exhaustively repeated; explicit alternate
# routes and every non-letter action are included. Ordinary alphabet rows are omitted.
# Symbol glyphs assume the repo's Hyprland us / altgr-intl host layout; another
# layout (or host dead-key/composition handling) can change the resulting text.
let
  entry = layer: name: gesture: aliases: {inherit name gesture layer aliases;};
  base = entry "Base 0";
  num = entry "Numbers 1";
  sym = entry "Symbols 2";
  sys = entry "Brd+Sys 3";
  nav = entry "Navigation 4";
  mouse = entry "Mouse 5";
  mirror = entry "Legacy hand-mirror 6";
  plan = entry "Proposed firmware (not flashed by this feature)";
  symbol = layer: name: gesture: aliases:
    entry layer name (gesture + "; glyph assumes host us / altgr-intl") (["symbol" "punctuation"] ++ aliases);
  shifted = name: key: aliases:
    symbol "Base 0 + Shift" name
    ("Hold Bspc (right Shift), then tap " + key + "; release Bspc without tapping")
    aliases;
  digitKeys = ["A" "R" "S" "T" "G" "M" "N" "E" "I" "O"];
  functionKeys = ["0" "1" "2" "3" "4" "5" "6" "7" "8" "9" "top-right CapsWord" "quote (')"];
in {
  revision = "JqpmQ/bvrAW7";
  current =
    [
      (base "Numbers layer" "Hold Space; release to return (tap Space types a space)" ["layer" "num" "digits"])
      (base "Symbols and function-keys layer" "Hold Enter; release to return (tap Enter sends Return)" ["layer" "sym" "F keys" "punctuation"])
      (base "Keyboard and system layer" "Hold Tab; release to return (tap Tab sends Tab)" ["layer" "Brd+Sys" "media" "RGB"])
      (base "Navigation layer" "Hold T; release to return (tap T types t)" ["layer" "nav" "arrows"])
      (base "Mouse layer" "Hold Esc; release to return (tap Esc sends Escape)" ["layer" "pointer" "click" "wheel"])
      (mirror "Momentary legacy hand-mirror" "From base, hold Z OR slash (/), without interruption until the tap dance resolves; release to leave layer 6" ["layer" "mirror" "hold" "tap dance"])
      (mirror "Enter latched legacy hand-mirror" "From base, double-tap Z OR slash (/) without interruption; enters layer 6 via layer_move, NOT a toggle" ["layer" "mirror" "latch" "double tap" "modal"])
      (mirror "Leave latched legacy hand-mirror" "While latched, hold Z OR slash (/) until its single-hold dance resolves, then release; layer_off(6) returns to base" ["mirror" "exit" "unlatch" "modal"])
      (mirror "Legacy tap-dance caveats" "Z and /: interrupted single hold becomes a letter/slash tap; interrupted double tap emits twice; triple and further taps emit that many characters; tap then hold (double-hold) has no action" ["tap dance" "timing" "mirror" "slash"])
      (mirror "Legacy mirror is modal, not Swap Hands" "Enter with Z or /; layer 6 replaces home-row mod-taps and layer-tap thumbs with plain keys, so normal Tab/T/Space/Esc/Enter layer holds are unavailable there" ["mirror" "modifier" "layers" "caveat"])

      (base "Caps Word" "Tap the top-right CapsWord key (right of 9); toggles word capitalization, not Caps Lock" ["caps" "capitalization" "uppercase" "CW_TOGG"])
      (sym "Caps Word" "Hold Enter, tap Tab; toggles word capitalization, not Caps Lock" ["caps" "capitalization" "uppercase"])
      (base "Left GUI / Super modifier" "Hold A (tap types a)" ["modifier" "super" "win" "command" "meta"])
      (base "Right GUI / Super modifier" "Hold O (tap types o)" ["modifier" "super" "win" "command" "meta"])
      (base "Left Control modifier" "Hold R (tap types r)" ["modifier" "ctrl" "control"])
      (base "Right Control modifier" "Hold I (tap types i)" ["modifier" "ctrl" "control"])
      (base "Left Alt modifier" "Hold S (tap types s); distinct from right Alt / AltGr" ["modifier" "alt" "option"])
      (base "Right Alt / AltGr modifier" "Hold the bottom-right AltGr key (right of /); host layout determines alternate characters" ["modifier" "right alt" "altgr" "level3"])
      (base "Left Shift modifier" "Hold E (tap types e)" ["modifier" "shift" "uppercase"])
      (base "Right Shift modifier" "Hold Bspc (tap sends Backspace)" ["modifier" "shift" "uppercase"])
      (sys "Plain left GUI / Super modifier" "Hold Tab, then hold A (no letter tap on this layer)" ["modifier" "super" "win" "command"])
      (sys "Plain left Shift modifier" "Hold Tab, then hold E (no letter tap on this layer)" ["modifier" "shift" "uppercase"])
      (sys "Plain right Control modifier" "Hold Tab, then hold I (no letter tap on this layer)" ["modifier" "ctrl" "control"])
      (base "Space" "Tap Space (holding enters Numbers)" ["whitespace" "spacebar"])
      (base "Escape" "Tap Esc (holding enters Mouse)" ["esc" "cancel"])
      (base "Enter / Return" "Tap Enter (holding enters Symbols)" ["return" "newline"])
      (base "Backspace" "Tap Bspc (holding is right Shift)" ["editing" "erase" "delete backward"])
      (base "Tab" "Tap Tab (holding enters Brd+Sys)" ["editing" "indent"])

      (symbol "Base 0" "Backtick / grave (`)" "Tap the top-left backtick key" ["backtick" "grave" "`"])
      (symbol "Base 0" "Equals (=)" "Tap = (left of Q)" ["equals" "equal" "="])
      (symbol "Base 0" "Semicolon (;)" "Tap ; (right of Y)" ["semicolon" ";"])
      (symbol "Base 0" "Apostrophe / quote (')" "Tap quote (right of ;)" ["apostrophe" "quote" "'"])
      (symbol "Base 0" "Minus / hyphen (-)" "Tap - (right of O)" ["minus" "hyphen" "dash" "-"])
      (symbol "Base 0" "Backslash (\\)" "Tap backslash (left of Z)" ["backslash" "\\"])
      (symbol "Base 0" "Comma (,)" "Tap comma (right of H)" ["comma" ","])
      (symbol "Base 0" "Period / dot (.)" "Tap period (right of comma)" ["period" "dot" "full stop" "."])
      (symbol "Base 0" "Slash (/)" "Single-tap / (right of period); hold and double-tap enter legacy mirror instead" ["slash" "forward slash" "/" "tap dance"])
      (shifted "Tilde (~)" "backtick (top-left)" ["tilde" "~"])
      (shifted "Exclamation mark (!)" "1" ["exclamation" "bang" "!"])
      (shifted "At sign (@)" "2" ["at" "@"])
      (shifted "Hash / number sign (#)" "3" ["hash" "number sign" "pound" "#"])
      (shifted "Dollar ($)" "4" ["dollar" "$"])
      (shifted "Percent (%)" "5" ["percent" "%"])
      (shifted "Caret / circumflex (^)" "6" ["caret" "circumflex" "^"])
      (shifted "Ampersand (&)" "7" ["ampersand" "and" "&"])
      (shifted "Asterisk (*)" "8" ["asterisk" "star" "*"])
      (shifted "Left parenthesis — (" "9" ["left parenthesis" "open paren" "("])
      (shifted "Right parenthesis — )" "0" ["right parenthesis" "close paren" ")"])
      (shifted "Underscore (_)" "minus (-)" ["underscore" "_"])
      (shifted "Plus (+)" "equals (=)" ["plus" "+"])
      (shifted "Colon (:)" "semicolon (;)" ["colon" ":"])
      (shifted "Double quote (\")" "quote (')" ["double quote" "quotation mark" "\""])
      (shifted "Pipe / vertical bar (|)" "backslash (\\)" ["pipe" "vertical bar" "|"])
      (shifted "Less than (<)" "comma (,)" ["less than" "left angle" "<"])
      (shifted "Greater than (>)" "period (.)" ["greater than" "right angle" ">"])
      (shifted "Question mark (?)" "slash (/) once (do not hold or double-tap /)" ["question mark" "?"])
      (symbol "Symbols 2" "Left parenthesis — (" "Hold Enter, tap P" ["left parenthesis" "open paren" "("])
      (symbol "Symbols 2" "Right parenthesis — )" "Hold Enter, tap L" ["right parenthesis" "close paren" ")"])
      (symbol "Symbols 2" "Caret / circumflex (^)" "Hold Enter, tap A" ["caret" "circumflex" "^"])
      (symbol "Symbols 2" "Ampersand (&)" "Hold Enter, tap R" ["ampersand" "and" "&"])
      (symbol "Symbols 2" "Asterisk (*)" "Hold Enter, tap S" ["asterisk" "star" "*"])
      (symbol "Symbols 2" "Left square bracket ([)" "Hold Enter, tap T" ["left bracket" "open bracket" "["])
      (symbol "Symbols 2" "Percent (%)" "Hold Enter, tap G" ["percent" "%"])
      (symbol "Symbols 2" "Hash / number sign (#)" "Hold Enter, tap M" ["hash" "number sign" "pound" "#"])
      (symbol "Symbols 2" "Right square bracket (])" "Hold Enter, tap N" ["right bracket" "close bracket" "]"])
      (symbol "Symbols 2" "At sign (@)" "Hold Enter, tap E" ["at" "@"])
      (symbol "Symbols 2" "Exclamation mark (!)" "Hold Enter, tap I" ["exclamation" "bang" "!"])
      (symbol "Symbols 2" "Dollar ($)" "Hold Enter, tap O" ["dollar" "$"])
      (symbol "Symbols 2" "Left curly brace ({)" "Hold Enter, tap D" ["left brace" "open brace" "{"])
      (symbol "Symbols 2" "Right curly brace (})" "Hold Enter, tap H" ["right brace" "close brace" "}"])

      (nav "Left arrow" "Hold T, tap N (hold N to repeat)" ["arrows" "navigation" "left"])
      (nav "Down arrow" "Hold T, tap E (hold E to repeat)" ["arrows" "navigation" "down"])
      (nav "Up arrow" "Hold T, tap I (hold I to repeat)" ["arrows" "navigation" "up"])
      (nav "Right arrow" "Hold T, tap O (hold O to repeat)" ["arrows" "navigation" "right"])
      (nav "Home" "Hold T, tap H" ["navigation" "editing" "home" "line start"])
      (nav "Page Down" "Hold T, tap comma (,)" ["navigation" "page down" "pgdn"])
      (nav "Page Up" "Hold T, tap period (.)" ["navigation" "page up" "pgup"])
      (nav "End" "Hold T, tap slash (/)" ["navigation" "editing" "end" "line end"])
      (nav "Navigation with modifiers" "Hold T, then hold A/R/S for GUI/Ctrl/Alt via base mod-taps, or Bspc for Shift; E/I/O are arrows here, and T is already the layer hold" ["modifier" "selection" "shift" "control" "alt" "super"])
      (sys "Previous application tab (Ctrl+Shift+Tab)" "Hold Tab, tap H; application determines the effect" ["navigation" "editing" "previous tab" "ctrl shift tab"])
      (sys "Next application tab (Ctrl+Tab)" "Hold Tab, tap comma (,); application determines the effect" ["navigation" "editing" "next tab" "ctrl tab"])
      (sys "Print Screen" "Hold Tab, tap L; host determines screenshot behavior" ["print screen" "screenshot" "prtsc"])
      (sys "Previous media track" "Hold Tab, tap Q" ["media" "music" "previous" "track"])
      (sys "Next media track" "Hold Tab, tap W" ["media" "music" "next" "track"])
      (sys "Stop media" "Hold Tab, tap F" ["media" "music" "stop"])
      (sys "Play / pause media" "Hold Tab, tap P" ["media" "music" "play" "pause"])
      (sys "Speaker volume down" "Hold Tab, tap R" ["audio" "speaker" "volume" "quieter"])
      (sys "Speaker volume up" "Hold Tab, tap S" ["audio" "speaker" "volume" "louder"])
      (sys "Speaker mute" "Hold Tab, tap T; sends audio-output mute, NOT microphone mute" ["audio" "speaker" "mute" "output"])

      (mouse "Left mouse click / button 1" "Hold Esc, tap L; hold L for button-down, move with N/E/I/O to drag, then release L" ["mouse" "click" "left click" "drag" "button 1"])
      (mouse "Middle mouse click / button 3" "Hold Esc, tap U; hold U for button-down, release to end" ["mouse" "click" "middle click" "drag" "button 3"])
      (mouse "Right mouse click / button 2" "Hold Esc, tap Y; hold Y for button-down, release to end" ["mouse" "click" "right click" "drag" "button 2" "context menu"])
      (mouse "Move pointer left" "Hold Esc, hold N; release N to stop" ["mouse" "pointer" "movement" "left" "drag"])
      (mouse "Move pointer down" "Hold Esc, hold E; release E to stop" ["mouse" "pointer" "movement" "down" "drag"])
      (mouse "Move pointer up" "Hold Esc, hold I; release I to stop" ["mouse" "pointer" "movement" "up" "drag"])
      (mouse "Move pointer right" "Hold Esc, hold O; release O to stop" ["mouse" "pointer" "movement" "right" "drag"])
      (mouse "Mouse wheel left" "Hold Esc, tap H (hold to repeat)" ["mouse" "wheel" "scroll" "horizontal" "left"])
      (mouse "Mouse wheel down" "Hold Esc, tap comma (,) (hold to repeat)" ["mouse" "wheel" "scroll" "down"])
      (mouse "Mouse wheel up" "Hold Esc, tap period (.) (hold to repeat)" ["mouse" "wheel" "scroll" "up"])
      (mouse "Mouse wheel right" "Hold Esc, tap slash (/) (hold to repeat)" ["mouse" "wheel" "scroll" "horizontal" "right"])

      (sys "Toggle RGB lighting" "Hold Tab, tap top-left backtick (`)" ["RGB" "lighting" "backlight" "toggle"])
      (sys "Toggle layer colors" "Hold Tab, tap 0; toggles layer-color indication" ["RGB" "lighting" "layer colors" "toggle"])
      (sys "Next RGB mode" "Hold Tab, tap 1; available effects depend on firmware configuration" ["RGB" "lighting" "effect" "mode"])
      (sys "Solid RGB mode" "Hold Tab, tap 2; custom RGB_SLD calls rgblight_mode(1)" ["RGB" "lighting" "solid"])
      (sys "RGB brightness down" "Hold Tab, tap 3" ["RGB" "lighting" "brightness" "dim"])
      (sys "RGB brightness up" "Hold Tab, tap 4" ["RGB" "lighting" "brightness" "bright"])
      (sys "Solid red RGB preset" "Hold Tab, tap C; sets solid mode and HSV 0/255/255 (layer indication can override visible colors)" ["RGB" "lighting" "red" "color" "HSV_0_255_255"])
      (sys "Solid green RGB preset" "Hold Tab, tap D; sets solid mode and HSV 74/255/255 (layer indication can override visible colors)" ["RGB" "lighting" "green" "color" "HSV_74_255_255"])
      (sys "Solid blue RGB preset" "Hold Tab, tap V; sets solid mode and HSV 169/255/255 (layer indication can override visible colors)" ["RGB" "lighting" "blue" "color" "HSV_169_255_255"])

      # These are remapped non-letter positions, not an exhaustive expansion of
      # transparent base keys. Hold Z to enter; keep it held for the following taps.
      (symbol "Legacy hand-mirror 6" "Mirrored apostrophe / quote (')" "Hold Z until mirror engages, tap = (left of Q)" ["mirror" "apostrophe" "quote" "'"])
      (symbol "Legacy hand-mirror 6" "Mirrored semicolon (;)" "Hold Z until mirror engages, tap Q" ["mirror" "semicolon" ";"])
      (symbol "Legacy hand-mirror 6" "Mirrored equals (=)" "Hold Z until mirror engages, tap quote (')" ["mirror" "equals" "="])
      (symbol "Legacy hand-mirror 6" "Mirrored minus (-)" "Hold Z until mirror engages, tap Tab" ["mirror" "minus" "hyphen" "-"])
      (mirror "Mirrored Tab" "Hold Z until mirror engages, tap minus (-); plain Tab, not a layer hold" ["mirror" "tab" "indent"])
      (symbol "Legacy hand-mirror 6" "Mirrored period (.)" "Hold Z until mirror engages, tap X" ["mirror" "period" "dot" "."])
      (symbol "Legacy hand-mirror 6" "Mirrored comma (,)" "Hold Z until mirror engages, tap C" ["mirror" "comma" ","])
      (symbol "Legacy hand-mirror 6" "Mirrored backslash (\\)" "Hold Z until mirror engages, tap AltGr (bottom-right)" ["mirror" "backslash" "\\"])
      (mirror "Mirrored Backspace" "Hold Z until mirror engages, tap Space; plain Backspace, not a layer hold" ["mirror" "editing" "erase" "backspace"])
      (mirror "Mirrored Enter / Return" "Hold Z until mirror engages, tap Esc; plain Enter, not a layer hold" ["mirror" "enter" "return"])
      (mirror "Mirrored Escape" "Hold Z until mirror engages, tap Enter; plain Escape, not a layer hold" ["mirror" "escape" "cancel"])
      (mirror "Mirrored Space" "Hold Z until mirror engages, tap Bspc; plain Space, not Shift" ["mirror" "space" "whitespace"])
    ]
    ++ builtins.genList (
      i:
        base ("Digit " + toString i)
        ("Tap top-row " + toString i)
        ["number" "digit" (toString i)]
    )
    10
    ++ builtins.genList (
      i:
        num ("Digit " + toString i + " (home-row route)")
        ("Hold Space, tap " + builtins.elemAt digitKeys i)
        ["number" "digit" (toString i)]
    )
    10
    ++ builtins.genList (
      i:
        sym ("F" + toString (i + 1))
        ("Hold Enter, tap " + builtins.elemAt functionKeys i)
        ["function" "F keys" ("F" + toString (i + 1))]
    )
    12;

  planned = [
    (plan "Push-to-talk / F13" "Proposed: use the base top-right CapsWord position for F13; hold to invert mic mute, tap under 250 ms to latch. Requires activated talkd; top-row 4 stays 4" ["F13" "function" "caps" "top right" "talk" "ptt" "microphone" "mic"])
    (plan "F14 microphone mute toggle" "Proposed: hold Tab, tap top-right CapsWord position to toggle the default microphone. Requires the separate host binding in PR #36; set the baseline before an F13 cycle" ["F14" "function" "microphone" "mic" "mute" "input"])
    (plan "Caps Lock on Symbols" "Proposed: hold Enter, tap top-left backtick (`) to toggle Caps Lock" ["caps" "caps lock" "capitalization" "uppercase"])
    (plan "Retained Caps Word on Symbols" "Proposed layout retains: hold Enter, tap Tab for Caps Word; base top-right route is replaced by F13" ["caps" "caps word" "capitalization" "uppercase"])
    (plan "Retained F1–F12 routes" "Proposed layout retains: hold Enter and tap 0–9 for F1–F10, top-right CapsWord position for F11, quote (') for F12" ["function" "F keys" "F1" "F2" "F3" "F4" "F5" "F6" "F7" "F8" "F9" "F10" "F11" "F12"])
    (plan "Plain Z and slash (issue #33)" "Proposed: tap or hold Z and / as ordinary keys; remove both legacy mirror tap dances (no mirror latch on double-tap)" ["mirror" "tap dance" "slash" "/" "Z" "33"])
    (plan "Whole-board mirror / Swap Hands (issue #33)" "Proposed: hold = OR quote (') and wait 200 ms before typing; release to leave mirror. Tapping still types equals/quote" ["mirror" "swap hands" "hold" "equals" "quote" "33"])
    (plan "Plain navigation modifiers (issue #33)" "Proposed: hold T for navigation, then hold A/R/S for plain GUI/Ctrl/Alt; Bspc remains right Shift. Nav-layer T is plain Shift only when another route holds nav, not when T itself is the nav hold" ["navigation" "modifier" "super" "control" "ctrl" "alt" "shift" "33"])
    (plan "Pedal mirror + navigation alias 7 (issue #33)" "Proposed: hold the middle/nav pedal while the left/mirror pedal is held or locked; release nav to return to mirror. Requires #33 firmware and daemon; no mirror+mouse combo" ["pedal" "mirror" "navigation" "alias 7" "33"])
  ];
}
