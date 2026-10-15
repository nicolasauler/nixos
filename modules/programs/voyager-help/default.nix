{
  config,
  lib,
  pkgs,
  ...
}: let
  keymap = import ./keymap.nix;
  escapeMarkup = lib.replaceStrings ["&" "<" ">" "\""] ["&amp;" "&lt;" "&gt;" "&quot;"];
  # Rofi's list textboxes ellipsize rather than soft-wrap. Break recipes at
  # word boundaries before markup, and reserve enough lines for the longest row.
  wrap = text: let
    result =
      builtins.foldl' (state: word: let
        next =
          if state.line == ""
          then word
          else "${state.line} ${word}";
      in
        if state.line != "" && builtins.stringLength next > 88
        then {
          lines = state.lines ++ [state.line];
          line = word;
        }
        else state // {line = next;})
      {
        lines = [];
        line = "";
      } (lib.splitString " " text);
  in
    result.lines ++ [result.line];
  markupLines = lines: lib.concatMapStringsSep "&#10;" escapeMarkup lines;
  render = status: entry: let
    title = wrap "${status}: ${entry.name}";
    gesture = wrap entry.gesture;
    layer = wrap "Layer: ${entry.layer}";
  in {
    lines = builtins.length title + builtins.length gesture + builtins.length layer;
    text =
      "<b>${markupLines title}</b>&#10;${markupLines gesture}&#10;${markupLines layer}"
      + "\t${escapeMarkup (lib.concatStringsSep " " entry.aliases)}";
  };
  rows = map (render "Reference") keymap.current ++ map (render "Planned") keymap.planned;
  rowHeight =
    builtins.foldl' (height: row:
      if row.lines > height
      then row.lines
      else height)
    1
    rows;
  mapping =
    pkgs.writeText "voyager-keyboard-reference.tsv"
    (lib.concatMapStringsSep "\n" (row: row.text) rows + "\n");
  message =
    "Reference revision ${keymap.revision}; snapshot, not live-detected state. "
    + "Start every gesture from the unmirrored base; key names are Colemak base legends. "
    + "Planned rows are target mappings, not verified active keys. Search functions, keys or aliases; Enter/Escape closes without action.";
  # Desktop Entry Exec is not shell syntax. Escape both its string-value and
  # double-quoted argument layers, including literal percent field-code escapes.
  quoteExec = argument:
    "\""
    + lib.replaceStrings
    ["\\" "\"" "`" "$" "%"]
    ["\\\\\\\\" "\\\\\"" "\\\\`" "\\\\$" "%%"]
    argument
    + "\"";
  arguments = [
    (lib.getExe config.programs.rofi.finalPackage)
    "-dmenu"
    "-input"
    (toString mapping)
    "-i"
    "-no-case-smart"
    "-no-auto-select"
    "-no-custom"
    "-markup-rows"
    "-display-columns"
    "1"
    "-display-column-separator"
    "\\t"
    # Even accepted rows produce only a newline, never row text or a command.
    "-format"
    ""
    "-p"
    "Voyager Reference"
    "-mesg"
    message
    "-eh"
    (toString rowHeight)
    "-no-show-icons"
    "-theme-str"
    "window { width: 96%; } listview { lines: 4; columns: 1; fixed-height: false; }"
    # drun may still own its normal PID lock while starting this view. Use a
    # separate lock, without killing/replacing the launcher or adding a wrapper.
    "-pid"
    "${config.xdg.cacheHome}/rofi-voyager-reference.pid"
  ];
in {
  xdg.dataFile."voyager-help/reference.tsv".source = mapping;

  xdg.desktopEntries.voyager-keyboard-reference = {
    name = "Voyager Keyboard Reference";
    genericName = "Keyboard Reference";
    comment = "Read-only Voyager reference and planned mappings";
    exec = lib.concatMapStringsSep " " quoteExec arguments;
    icon = "input-keyboard";
    terminal = false;
    startupNotify = false;
    categories = ["Utility"];
    settings.Keywords = "Voyager;Keyboard;Keymap;Reference;Help;";
  };
}
