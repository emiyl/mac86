#!/usr/bin/env bash
set -euo pipefail

I386_SDK_PATH="${MACOSX_I386_SDK:-$HOME/Downloads/MacOSX10.13.sdk}"

SRC_PATH="${1:?usage: $0 <src> <output>}"
OUT_PATH="${2:?usage: $0 <src> <output>}"

if [[ ! -d "$I386_SDK_PATH" ]]; then
    echo "error: i386 SDK not found at: $I386_SDK_PATH"
    echo "set MACOSX_I386_SDK=/path/to/MacOSX10.13.sdk"
    exit 1
fi

clang -arch i386 \
    -isysroot "$I386_SDK_PATH" \
    -mmacosx-version-min=10.6 \
    -Wl,-undefined,dynamic_lookup \
    "$SRC_PATH" \
    -o "$OUT_PATH"_i386

echo "built: $OUT_PATH"
file "$OUT_PATH"
