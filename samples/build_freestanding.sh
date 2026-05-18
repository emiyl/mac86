#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SAMPLES_DIR="$ROOT_DIR/samples"
SDK_PATH="${MACOSX_I386_SDK:-$HOME/Downloads/MacOSX10.13.sdk}"
OUT_PATH="${1:-$SAMPLES_DIR/phase2}"
SRC_PATH="${2:-$SAMPLES_DIR/phase2_main.c}"
BUILD_DIR="$SAMPLES_DIR/.build"

if [[ ! -d "$SDK_PATH" ]]; then
    echo "error: i386 SDK not found at: $SDK_PATH"
    echo "set MACOSX_I386_SDK=/path/to/MacOSX10.13.sdk"
    exit 1
fi

mkdir -p "$BUILD_DIR"

clang -arch i386 \
    -isysroot "$SDK_PATH" \
    -mmacosx-version-min=10.6 \
    -c "$SAMPLES_DIR/crt0.s" \
    -o "$BUILD_DIR/crt0.o"

clang -arch i386 \
    -isysroot "$SDK_PATH" \
    -mmacosx-version-min=10.6 \
    -ffreestanding \
    -fno-stack-protector \
    -fno-builtin \
    -fno-asynchronous-unwind-tables \
    -c "$SRC_PATH" \
    -o "$BUILD_DIR/main.o"

ld -arch i386 \
    -macos_version_min 10.6 \
    -static \
    -e _start \
    -o "$OUT_PATH" \
    "$BUILD_DIR/crt0.o" "$BUILD_DIR/main.o"

echo "built: $OUT_PATH"
file "$OUT_PATH"
