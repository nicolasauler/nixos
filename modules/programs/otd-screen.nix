{pkgs, ...}: let
  # OTD's tablet database (physical digitizer sizes) is compiled into the daemon as
  # embedded resources; ship the JSON tree from the same source revision instead.
  configurations = "${pkgs.opentabletdriver.src}/OpenTabletDriver.Configurations/Configurations";
in {
  # `otd-screen [OUTPUT] [ROTATION]`: map the OpenTabletDriver tablet onto one
  # Hyprland output (default: the focused monitor), aspect-ratio locked, tablet
  # turned sideways on portrait outputs, then persist like the GUI's Save.
  # SUPER+CTRL+T in the hyprland config runs it.
  #
  # `otd` and `hyprctl` are deliberately taken from the session PATH: the CLI
  # must match the running daemon (NixOS hardware.opentabletdriver) and
  # compositor, not whatever this module's pkgs pins.
  home.packages = [
    (pkgs.writeShellApplication {
      name = "otd-screen";
      runtimeInputs = [pkgs.coreutils pkgs.gnused pkgs.jq];
      text = ''
        # ROTATION is the OTD tablet-area rotation in degrees. Default "auto":
        # 0 on landscape outputs, $portrait_rotation on portrait ones.
        # 90 = tablet turned counter-clockwise (HS64 express keys end up at the
        # bottom, former top edge on the left); 270 = turned clockwise.
        portrait_rotation=90
        output=''${1:-$(hyprctl monitors -j | jq -r '.[] | select(.focused) | .name')}
        rotation=''${2:-auto}

        # otd prints Vector2/float in the current culture; keep it parseable.
        export LC_ALL=C

        tablet=$(otd getallsettings | sed -n "s/^--- Profile for '\(.*\)' ---$/\1/p" | head -n1)
        if [[ -z $tablet ]]; then
          echo "otd-screen: no tablet profile (daemon running? tablet plugged in?)" >&2
          exit 1
        fi

        size=$(jq -r --arg name "$tablet" \
          'select(.Name == $name) | "\(.Specifications.Digitizer.Width) \(.Specifications.Digitizer.Height)"' \
          ${configurations}/*/*.json)
        if [[ -z $size ]]; then
          echo "otd-screen: no OTD configuration named '$tablet'" >&2
          exit 1
        fi
        read -r tw th <<<"$size"

        # OTD's own view of the layout (xdg-output logical coordinates):
        #   "<index>: <name> <description> (<w>x<h>@<<x>, <y>>)"; index 0 is the virtual screen.
        displays=$(otd listdisplays \
          | sed -n 's/^[1-9][0-9]*: \([^ ]*\) .*(\([0-9]*\)x\([0-9]*\)@<\(-\?[0-9]*\), \(-\?[0-9]*\)>)$/\1 \2 \3 \4 \5/p')
        if ! grep -q "^$output " <<<"$displays"; then
          echo "otd-screen: output '$output' not in otd listdisplays:" >&2
          otd listdisplays >&2
          exit 1
        fi

        # Areas are center-based. The daemon applies them verbatim (LockAspectRatio is a
        # GUI-only constraint), so fit the largest rectangle with the output's aspect ratio
        # into the (rotated) tablet ourselves. OTD normalises the layout so the top-left
        # output sits at 0,0 (see MapToDisplayIndex in OpenTabletDriver.Console).
        # Assignment (not a here-string substitution) so a jq failure, e.g. a bad
        # ROTATION, aborts under `set -e` instead of feeding otd empty arguments.
        areas=$(jq -rn \
          --arg displays "$displays" --arg output "$output" --arg rot "$rotation" \
          --argjson tw "$tw" --argjson th "$th" --argjson portrait "$portrait_rotation" '
          def r3: . * 1000 | round / 1000;
          ($displays | split("\n") | map(split(" ") | map(tonumber? // .)
            | {name: .[0], w: .[1], h: .[2], x: .[3], y: .[4]})) as $ds
          | ($ds[] | select(.name == $output)) as $d
          | ($d.x - ($ds | map(.x) | min) + $d.w / 2) as $dx
          | ($d.y - ($ds | map(.y) | min) + $d.h / 2) as $dy
          | (if $rot == "auto" then (if $d.h > $d.w then $portrait else 0 end) else ($rot | tonumber) end) as $r
          | (if $r % 180 == 0 then [$tw, $th] else [$th, $tw] end) as [$mw, $mh]
          | ([$mw / $d.w, $mh / $d.h] | min) as $s
          | [($d.w * $s | r3), ($d.h * $s | r3), $tw / 2, $th / 2, $d.w, $d.h, $dx, $dy, $r]
          | map(tostring) | join(" ")')
        read -r aw ah ax ay dw dh dx dy rotation <<<"$areas"

        otd settabletarea "$tablet" "$aw" "$ah" "$ax" "$ay" "$rotation"
        otd setdisplayarea "$tablet" "$dw" "$dh" "$dx" "$dy"
        otd savedefaultsettings
        echo "$tablet -> $output ''${dw}x''${dh} (center $dx,$dy); tablet area ''${aw}x''${ah} mm, rotation $rotation"
      '';
    })
  ];
}
