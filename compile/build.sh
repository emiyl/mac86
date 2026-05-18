#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SDK_PATH="${MACOSX_I386_SDK:-$HOME/Downloads/MacOSX10.13.sdk}"
SAMPLES_DIR="$ROOT_DIR/samples"

SRC_PATH="${1:?usage: $0 <src> <output>}"
OUT_PATH="${2:?usage: $0 <src> <output>}"

BUILD_DIR="$ROOT_DIR/.build"

if [[ ! -d "$SDK_PATH" ]]; then
    echo "error: i386 SDK not found at: $SDK_PATH"
    echo "set MACOSX_I386_SDK=/path/to/MacOSX10.13.sdk"
    exit 1
fi

mkdir -p "$BUILD_DIR"

clang -arch i386 \
    -isysroot "$SDK_PATH" \
    -mmacosx-version-min=10.6 \
    "$SRC_PATH" \
    -o "$OUT_PATH"

echo "built: $OUT_PATH"
file "$OUT_PATH"
