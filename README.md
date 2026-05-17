# mac86: i386 macOS Emulator for arm64 Macs

An experimental Rust project to emulate 32-bit Intel (i386) macOS applications on Apple Silicon (arm64) Macs.

## Overview

**mac86** is a compatibility layer and emulator that enables running legacy i386 macOS applications on modern arm64 Mac hardware. This project draws inspiration from:
- **Darling**: macOS compatibility layer for Linux
- **Wine**: Windows compatibility layer for Unix systems
- **QEMU**: Generic machine emulator
- **Rosetta 2**: Apple's x86-64 to arm64 translator

## Project Status

🚧 **Early Development** - Core architecture and module structure in place. Not yet functional.

## Architecture

```
┌─────────────────────────────────────────────┐
│        i386 macOS Binary (Mach-O)           │
└──────────────────┬──────────────────────────┘
                   │
┌──────────────────▼──────────────────────────┐
│          Binary Loader                      │
│  - Parse Mach-O format                      │
│  - Extract segments/sections                │
│  - Validate i386 architecture               │
└──────────────────┬──────────────────────────┘
                   │
┌──────────────────▼──────────────────────────┐
│       Process & Emulation Context           │
│  - Memory Management                        │
│  - Virtual Filesystem                       │
│  - CPU Emulation (Unicorn)                  │
│  - Syscall Translation                      │
└──────────────────┬──────────────────────────┘
                   │
┌──────────────────▼──────────────────────────┐
│      host arm64 macOS kernel                │
└─────────────────────────────────────────────┘
```

## Core Modules

### `binary_loader`
Responsible for parsing i386 Mach-O executables:
- Binary format validation
- Segment and section extraction
- Architecture verification
- Entry point detection

### `emulator`
Main emulation context management:
- Initialization of the emulation environment
- Lifecycle management (initialize, run, shutdown)
- Environment path management

### `memory`
Virtual memory management:
- Memory region allocation and deallocation
- Permission management (read/write/execute)
- Safe read/write operations with bounds checking
- Page-aligned allocation

### `syscall`
BSD-style syscall handling:
- Syscall dispatch and routing
- Handler registration system
- Default implementations for common syscalls
- i386 macOS syscall numbers

### `filesystem`
Virtual filesystem abstraction:
- Mount emulated paths to host paths
- File descriptor management
- Read/write operations
- Standard stream handling (stdin, stdout, stderr)

### `process`
Process execution:
- Binary loading into emulated memory
- Stack and heap allocation
- Register initialization
- Execution engine invocation

### `errors`
Error handling and types:
- Custom error types for all domains
- Result type aliases

## Building

### Prerequisites
- Rust 1.70+
- macOS (arm64 recommended)

### Build
```bash
cargo build --release
```

### Run
```bash
cargo run -- /path/to/i386/binary [arguments]
```

### Verbose Output
```bash
RUST_LOG=debug cargo run -- /path/to/i386/binary
```

## Usage

### Basic Execution
```bash
mac86 ~/old_apps/legacy_app
```

### With Arguments
```bash
mac86 ~/old_apps/legacy_app arg1 arg2 arg3
```

### Verbose Mode
```bash
mac86 -v ~/old_apps/legacy_app
```

### Custom Environment Path
```bash
mac86 --env-path /tmp/custom_env ~/old_apps/legacy_app
```

## Implementation Details

### Syscall Translation
i386 macOS uses BSD-style syscalls. Key syscalls to implement:
- `exit` - Process termination
- `read`/`write` - I/O operations
- `open`/`close` - File operations
- `stat`/`fstat` - File metadata
- `mmap`/`munmap` - Memory mapping
- `fork`/`exec` - Process management
- `signal` - Signal handling

### Memory Layout
Typical i386 process memory layout:
```
0xFFFFFFFF ├─ Kernel space
           │
0xC0000000 ├─ Shared libraries
           │
0x08000000 ├─ Heap (grows up)
           │
0x04000000 ├─ BSS/uninitialized data
           │
0x02000000 ├─ Initialized data (.data, .rodata)
           │
0x01000000 ├─ Code (.text)
           │
0x00001000 ├─ Stack (grows down)
           │
0x00000000 ├─ Reserved (PAGEZERO)
```

### CPU Emulation
Currently uses Unicorn Engine for x86/i386 CPU emulation:
- Register state management
- Instruction execution
- Exception handling
- Interrupt handling

## Known Limitations

1. **No Dynamic Linking**: Statically linked binaries only (for now)
2. **Limited Syscall Support**: Only essential syscalls implemented
3. **No Graphics**: GUI applications not supported
4. **No Networking**: Network syscalls stubbed out
5. **Single Threading**: Multi-threaded apps may not work correctly
6. **No Frameworks**: macOS framework support not implemented

## Future Enhancements

- [ ] Full dynamic linking support
- [ ] Complete BSD syscall implementation
- [ ] Threading support (pthread)
- [ ] Signal handling
- [ ] Better file system virtualization
- [ ] Performance optimizations
- [ ] Debugging support (GDB integration)
- [ ] dylib/framework simulation
- [ ] Full macOS ABI compatibility

## References

- [Mach-O File Format](https://developer.apple.com/library/archive/documentation/Performance/Conceptual/CodeFootprint/Articles/MachOOverview.html)
- [i386 Architecture](https://en.wikipedia.org/wiki/IA-32)
- [BSD System Calls](https://www.unix.com/man-page/bsd/2/)
- [Darling Project](https://github.com/darlinghq/darling)
- [Wine Project](https://www.winehq.org/)
- [Unicorn Engine](https://www.unicorn-engine.org/)

## License

MIT (or Apache 2.0 - choose based on preference)

## Contributing

Contributions welcome! Please ensure:
- Code follows Rust conventions
- Tests are included for new features
- Documentation is updated
- Commits are descriptive

## Disclaimer

This is an experimental project. The i386 architecture is deprecated on macOS. Use only for compatibility with legacy applications. Not recommended for production use.
