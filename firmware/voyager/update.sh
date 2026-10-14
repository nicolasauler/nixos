# Pulls the latest saved revision of the Oryx layout pinned in
# firmware/voyager/oryx.json into firmware/voyager/oryx/. Run from the root
# of the nixos checkout: `nix run .#voyager-layout-update`.
dir=firmware/voyager
if [[ ! -f $dir/oryx.json ]]; then
  echo "voyager-layout-update: run me from the root of the nixos checkout" >&2
  exit 1
fi
layout=$(jq -r .layout "$dir/oryx.json")
current=$(jq -r .revision "$dir/oryx.json")

# Oryx's public GraphQL API, as ZSA's own "custom QMK" tooling uses it.
query='query($hashId: String!, $revisionId: String!, $geometry: String) {
  layout(hashId: $hashId, geometry: $geometry, revisionId: $revisionId) {
    revision { hashId qmkVersion title }
  }
}'
revision=$(jq -n --arg q "$query" --arg layout "$layout" \
  '{query: $q, variables: {hashId: $layout, geometry: "voyager", revisionId: "latest"}}' |
  curl -fsS --max-time 30 https://oryx.zsa.io/graphql \
    -H 'Content-Type: application/json' --data @- |
  jq -e '.data.layout.revision')
latest=$(jq -r .hashId <<<"$revision")
firmware=$(jq -r .qmkVersion <<<"$revision")
title=$(jq -r .title <<<"$revision")

if [[ $latest == "$current" ]]; then
  echo "$layout/$current is already the latest revision"
  exit 0
fi
if [[ ${firmware%%.*} != 25 ]]; then
  echo "voyager-layout-update: $layout/$latest is for Oryx firmware $firmware;" \
    "firmware/voyager/default.nix builds against ZSA's firmware25 branch. Bump it first." >&2
  exit 1
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
curl -fsSL --max-time 120 "https://oryx.zsa.io/source/$latest" -o "$work/source.zip"
unzip -q -j "$work/source.zip" '*_source/*' -d "$work/source"
for file in keymap.c config.h rules.mk keymap.json; do
  [[ -f $work/source/$file ]] || { echo "voyager-layout-update: $file missing from the export" >&2; exit 1; }
done
rm -f "$dir"/oryx/*
cp "$work"/source/* "$dir/oryx/"
jq -n --arg layout "$layout" --arg revision "$latest" --arg firmware "$firmware" --arg title "$title" \
  '{layout: $layout, revision: $revision, firmware: $firmware, title: $title}' > "$dir/oryx.json"
echo "$layout: $current -> $latest ($title)"
echo "next: nix build .#voyager-firmware && nix run nixpkgs#zapp -- flash result/*.bin"
