#!/bin/bash
# mac86 build and test script

set -e

echo "=== mac86 i386 Emulator Setup ==="
echo

# Check Rust installation
if ! command -v cargo &> /dev/null; then
    echo "Error: Rust is not installed"
    echo "Please install from https://rustup.rs/"
    exit 1
fi

echo "✓ Rust toolchain found"
echo

# Build project
echo "Building mac86..."
cargo build --release

echo
echo "✓ Build complete!"
echo

# Display results
echo "=== Build Summary ==="
echo "Release binary: target/release/mac86"
echo
echo "To run a binary:"
echo "  ./target/release/mac86 /path/to/i386/binary"
echo
echo "For verbose output:"
echo "  RUST_LOG=debug ./target/release/mac86 /path/to/i386/binary"
echo
