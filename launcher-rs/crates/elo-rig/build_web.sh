#!/usr/bin/env bash
# Build what watchtower/animate.html serves, from this crate:
#
#   watchtower/animate/elo_rig.wasm         this crate for wasm32
#   watchtower/animate/clips/*.glb          Mesh2Motion's three CC0 clip packs,
#                                           pinned by commit and sha256, cut to
#                                           their bare skeletons by `clips_only`
#
# and record what it built in web.sha256 beside this script, which
# tests/web_is_current.rs holds against the source: change a .rs file here
# and the test fails until this is run again. Needs the wasm32-unknown-unknown
# target (`rustup target add wasm32-unknown-unknown`) and network for the clips.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
ws="$here/../.."
site="$here/../../../watchtower/animate"
mkdir -p "$site/clips"

(cd "$ws" && cargo build -q -p elo-rig --target wasm32-unknown-unknown --release)
install -m 644 "$ws/target/wasm32-unknown-unknown/release/elo_rig.wasm" "$site/elo_rig.wasm"

# Mesh2Motion (https://github.com/Mesh2Motion/mesh2motion-app): art is CC0,
# code MIT. Its base pack is built on Quaternius's Universal Animation
# Library (CC0).
M2M=eef2de986dab53283cb636f860d502d64b8073e7
packs=(
  "base 406eb0a8dc4ab366e623b79b6e3005a4951392e1bda78ae39c1099d31147733c"
  "addon a0d64d555e0d492026b72d58bf8e16c5e86779295f9093e376dcc001915c2c95"
  "mocap 814593d62522f5be8d6bd32df2a8fa3c7feb90a8e1a1ba43f4aaf0048555280d"
)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
(cd "$ws" && cargo build -q -p elo-rig --example rig --release)
for p in "${packs[@]}"; do
  set -- $p
  curl -fsSL "https://raw.githubusercontent.com/Mesh2Motion/mesh2motion-app/$M2M/static/animations/human-$1-animations.glb" -o "$work/$1.glb"
  got=$(sha256sum "$work/$1.glb" | cut -d' ' -f1)
  [ "$got" = "$2" ] || { echo "integrity mismatch for Mesh2Motion $1" >&2; exit 1; }
  "$ws/target/release/examples/rig" clips "$work/$1.glb" "$site/clips/mesh2motion-$1.glb"
done

{
  echo "$(cd "$here" && LC_ALL=C cat Cargo.toml $(LC_ALL=C ls src/*.rs) | sha256sum | cut -d' ' -f1)  src"
  (cd "$site" && sha256sum elo_rig.wasm clips/*.glb)
} > "$here/web.sha256"
cat "$here/web.sha256"

# The page asks for each file by the first eight hex of its hash, so a new
# build is a new url and no cache can serve the old one; then the page's own
# ?v= token for js/animate.js follows (build_meta.py).
js="$site/../js/animate.js"
while read -r hash name; do
  [ "$name" = src ] && continue
  sed -i -E "s#(${name//./\\.})\?v=[0-9a-f]{8}#\1?v=${hash:0:8}#" "$js"
done < "$here/web.sha256"
python3 "$site/../build_meta.py"
