#!/usr/bin/env bash
# Rebuild ../vendor.js: three.js and three-vrm, from npm tarballs pinned by the
# registry's own sha512, bundled and minified with bun. Run it only to move a
# version; then update VENDOR_SHA256 in tests/stage.rs in the same commit.
#
# Both are MIT (three.js: mrdoob and contributors; three-vrm: pixiv Inc.). The
# bundle keeps their licence banners.
set -euo pipefail
cd "$(dirname "$0")"

THREE=0.186.1
THREE_SRI=sha512-blFeqb49wRCSGUGj7gtpfnSGHy2lwDk94RhUmS1c/hTby70kvChbWpkJ4Pm1390LqzzvTmzgXKHPEafJwCb8jA==
VRM=3.5.5
VRM_SRI=sha512-RPXy7jYAXs704NIpZlosB0U2ENu21G9DrqGWdQgRe8dShaCo1ugpj+6BVPRCy91nt+MPMA96j5rbsSzEl0HlQA==
BUN=1.4.4

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fetch() {  # url sri dest
  curl -fsSL "$1" -o "$work/pkg.tgz"
  got="sha512-$(openssl dgst -sha512 -binary "$work/pkg.tgz" | base64 -w0)"
  [ "$got" = "$2" ] || { echo "integrity mismatch for $1" >&2; exit 1; }
  mkdir -p "$3"
  tar xzf "$work/pkg.tgz" -C "$3" --strip-components=1
}

fetch "https://registry.npmjs.org/three/-/three-$THREE.tgz" "$THREE_SRI" "$work/node_modules/three"
fetch "https://registry.npmjs.org/@pixiv/three-vrm/-/three-vrm-$VRM.tgz" "$VRM_SRI" "$work/node_modules/@pixiv/three-vrm"
cp entry.js "$work/"

have=$(bun --version)
[ "$have" = "$BUN" ] || echo "note: bun $have, the pinned bundle was made with $BUN; the hash may differ" >&2

(cd "$work" && bun build entry.js --minify --format=esm --target=browser --outfile=vendor.js >/dev/null)
{
  echo "/* three.js $THREE (MIT, https://github.com/mrdoob/three.js/blob/dev/LICENSE)"
  echo "   three-vrm $VRM (MIT, (c) pixiv Inc., https://github.com/pixiv/three-vrm/blob/release/LICENSE)"
  echo "   bundled by launcher-rs/crates/elo-launcher/stage/vendor/build.sh */"
  cat "$work/vendor.js"
} > ../vendor.js
sha256sum ../vendor.js
