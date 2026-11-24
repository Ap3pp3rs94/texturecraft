#!/bin/sh
set -eu

real_bin="$(dirname "$0")/.texturecraft-nightly/texturecraft"
exec "$real_bin" "$@"
