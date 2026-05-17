# mac86 Quick Start Guide

## Building the Project

```bash
cd /path/to/mac86
cargo build --release
```

The binary will be available at: `target/release/mac86`

## Running an i386 Binary

```bash
./target/release/mac86 /path/to/i386/macos/binary
```

## Phase 1: Freestanding C (Works with Current Emulator)

The current emulator supports self-contained i386 binaries that issue syscalls directly.

Build a freestanding sample C program:

```bash
# If your SDK is somewhere else, set this first:
# export MACOSX_I386_SDK=~/Downloads/MacOSX10.13.sdk

./samples/build_freestanding.sh
```

Run it:

```bash
cargo run -- samples/phase1_hello_static
```

Use your own C source file:

```bash
./samples/build_freestanding.sh samples/myprog samples/myprog.c
cargo run -- samples/myprog
```

Notes:
- Entry code is provided by `samples/crt0.s` (`_start` calls `_main`, then does `sys_exit`).
- Syscall helpers are in `samples/syscall.h`.
- This path avoids dynamic linking and `libSystem`/`dyld` requirements.

## Example: Test Binary

To create a minimal i386 test binary for testing:

```bash
# Create a simple C program
cat > test.c << 'EOF'
#include <stdio.h>
int main() {
    printf("Hello from i386!\n");
    return 0;
}
EOF

# Compile for i386 (requires i386 support)
# Note: Apple Silicon Macs cannot compile i386 binaries natively
# You may need to use a Linux VM or x86 Mac for compilation
gcc -m32 -o test test.c

# Try to run with mac86
./target/release/mac86 ./test
```

## Development

### Running Tests

```bash
cargo test --release
```

### Running with Verbose Output

```bash
RUST_LOG=debug ./target/release/mac86 /path/to/binary
```

### Checking Code

```bash
cargo check
```

### Code Formatting

```bash
cargo fmt
```

### Linting

```bash
cargo clippy -- -D warnings
```

## Project Structure

```
mac86/
├── Cargo.toml                 # Project manifest
├── src/
│   ├── main.rs               # Entry point
│   ├── binary_loader.rs      # Mach-O binary parsing
│   ├── emulator.rs           # Emulation context
│   ├── process.rs            # Process execution
│   ├── memory.rs             # Virtual memory management
│   ├── syscall.rs            # Syscall translation
│   ├── filesystem.rs         # Virtual filesystem
│   └── errors.rs             # Error types
├── README.md                 # Project documentation
├── ARCHITECTURE.md           # Detailed architecture
├── ROADMAP.md               # Implementation roadmap
└── QUICKSTART.md            # This file
```

## Key Modules

### Binary Loader (`binary_loader.rs`)
- Parses Mach-O i386 binaries
- Extracts segments and sections
- Validates architecture

### Emulator (`emulator.rs`)
- Creates emulation context
- Manages emulation lifecycle
- Provides environment setup

### Memory Management (`memory.rs`)
- Allocates virtual address space
- Enforces memory permissions
- Manages memory regions

### Syscall Handler (`syscall.rs`)
- Translates i386 syscalls to host syscalls
- Implements BSD-style syscalls
- Handles syscall dispatching

### Virtual Filesystem (`filesystem.rs`)
- Maps emulated paths to host paths
- Manages file descriptors
- Provides I/O abstraction

### Process Management (`process.rs`)
- Loads binaries into memory
- Initializes process state
- Coordinates execution

## Common Issues

### "Binary loading failed"
- Ensure the file is a valid i386 Mach-O binary
- Check that the binary is for macOS, not Linux or other OS

### "Unsupported CPU type"
- The binary must be compiled for i386 (CPU type 7)
- x86_64 binaries are not currently supported

### "Permission denied"
- Ensure you have execute permissions on the emulator binary
- Check permissions on the target binary

## Next Steps

1. Review the [Architecture](ARCHITECTURE.md) document
2. Check the [Roadmap](ROADMAP.md) for what's being worked on
3. Try building and running with simple test programs
4. Contribute! See README.md for contribution guidelines

## Debugging

Enable debug logging:

```bash
RUST_LOG=mac86=debug cargo run -- /path/to/binary
```

View specific modules:

```bash
RUST_LOG=mac86::memory=debug,mac86::syscall=debug cargo run -- /path/to/binary
```

## Resources

- [Mach-O File Format](https://developer.apple.com/library/archive/documentation/Performance/Conceptual/CodeFootprint/Articles/MachOOverview.html)
- [i386 Instruction Set](https://en.wikipedia.org/wiki/IA-32)
- [goblin Crate](https://docs.rs/goblin/)
- [Unicorn Engine](https://www.unicorn-engine.org/)
