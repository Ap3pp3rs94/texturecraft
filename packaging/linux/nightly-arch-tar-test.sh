#!/usr/bin/env bash
# Validate the staged Arch launcher without building or opening TextureCraft.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
stage="$work/source"
dst="$work/dist"
base=texturecraft-0.0.1-nightly.1-linux-x86_64
mkdir -p "$stage/bin" "$dst" "$work/extract"

# A local argv recorder stands in for the compiled binaries. It never opens a UI.
cat >"$stage/bin/texturecraft" <<'RECORDER'
#!/bin/sh
printf '<%s>\n' "$@"
RECORDER
chmod 755 "$stage/bin/texturecraft"
cp "$stage/bin/texturecraft" "$stage/bin/texturecraft-cli"

bash "$HERE/nightly-arch-tar.sh" "$stage" "$work/stage" "$dst" "$base"
tar -xzf "$dst/$base.tar.gz" -C "$work/extract"
bin="$work/extract/$base/bin"
# Simulate the existing Arch bin-tree copy and public launcher rename.
mkdir -p "$work/installed"
cp -R "$bin" "$work/installed/bin"
bin="$work/installed/bin"
mv "$bin/texturecraft" "$bin/texturecraft-nightly"
mv "$bin/texturecraft-cli" "$bin/texturecraft-nightly-cli"

test ! -e "$stage/bin/.texturecraft-nightly"
cmp "$stage/bin/texturecraft" "$bin/.texturecraft-nightly/texturecraft"
cmp "$stage/bin/texturecraft-cli" "$bin/texturecraft-nightly-cli"
expect() {
  expected="$1"
  shift
  actual=$(PATH="$bin:$PATH" texturecraft-nightly "$@")
  if [[ "$actual" != "$expected" ]]; then
    printf 'expected:\n%s\nactual:\n%s\n' "$expected" "$actual" >&2
    exit 1
  fi
}
expect '<>'
expect "$(printf '<%s>\n' 'mesh with spaces.glb' '--other-name.tcraft')" \
  'mesh with spaces.glb' '--other-name.tcraft'
expect '<--version>' --version
expect '<-V>' -V

echo "PASS: Arch Nightly handoff preserves binary layout and launch arguments"
