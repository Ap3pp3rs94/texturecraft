#!/bin/sh
set -eu

if [ -z "${TEXTURECRAFT_DATA_DIR:-}" ]; then
  export TEXTURECRAFT_DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/TextureCraft-Nightly"
fi
real_bin="$(dirname "$0")/.texturecraft-nightly/texturecraft"
exec "$real_bin" "$@"
