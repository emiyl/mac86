#!/usr/bin/env bash
set -euo pipefail

I386_SDK_PATH="${MACOSX_I386_SDK:-$HOME/Downloads/MacOSX10.13.sdk}"

if [[ ! -d "$I386_SDK_PATH" ]]; then
    echo "error: i386 SDK not found at: $I386_SDK_PATH"
    echo "set MACOSX_I386_SDK=/path/to/MacOSX10.13.sdk"
    exit 1
fi

shopt -s nullglob

for SRC_PATH in *.c; do
    BASE_NAME="${SRC_PATH%.c}"

    echo "building $SRC_PATH..."

    clang -arch i386 \
        -isysroot "$I386_SDK_PATH" \
        -mmacosx-version-min=10.6 \
        -Wl,-undefined,dynamic_lookup \
        "$SRC_PATH" \
        -o "${BASE_NAME}_i386"

    clang -arch arm64 \
        -mmacosx-version-min=10.15 \
        -framework CoreFoundation \
        "$SRC_PATH" \
        -o "${BASE_NAME}_arm64"

    echo "built:"
    file "${BASE_NAME}_i386"
    file "${BASE_NAME}_arm64"
    echo
done