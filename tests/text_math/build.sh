#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
I386_SDK_PATH="${MACOSX_I386_SDK:-$HOME/Downloads/MacOSX10.13.sdk}"

if [[ ! -d "$I386_SDK_PATH" ]]; then
    echo "error: i386 SDK not found at: $I386_SDK_PATH"
    echo "set MACOSX_I386_SDK=/path/to/MacOSX10.13.sdk"
    exit 1
fi

cd "$SCRIPT_DIR"

clang -arch i386 \
    -isysroot "$I386_SDK_PATH" \
    -mmacosx-version-min=10.6 \
    -fno-builtin \
    -U_FORTIFY_SOURCE -D_FORTIFY_SOURCE=0 \
    -Wl,-undefined,dynamic_lookup \
    text_math.c \
    -o text_math_i386

clang -arch arm64 \
    -mmacosx-version-min=10.15 \
    -fno-builtin \
    -U_FORTIFY_SOURCE -D_FORTIFY_SOURCE=0 \
    text_math.c \
    -o text_math_arm64
