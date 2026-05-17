# mac86 Architecture

## System Overview

mac86 is structured as a multi-layer compatibility and emulation system:

```
┌────────────────────────────────────────────────────────┐
│         User: i386 macOS Application                   │
│         (requires: i386 binary, Mach-O format)         │
└────────────────────────────┬───────────────────────────┘
                             │
┌────────────────────────────▼───────────────────────────┐
│              mac86 Emulation Layer                     │
│  ┌─────────────────────────────────────────────────┐  │
│  │  1. Binary Loader (binary_loader.rs)            │  │
│  │     • Parse Mach-O i386 binary                  │  │
│  │     • Extract executable code & data            │  │
│  │     • Verify architecture compatibility         │  │
│  └─────────────────────────────────────────────────┘  │
│  ┌─────────────────────────────────────────────────┐  │
│  │  2. Emulation Context (emulator.rs)             │  │
│  │     • Initialize emulation environment          │  │
│  │     • Manage emulation lifecycle                │  │
│  │     • Coordinate subsystems                     │  │
│  └─────────────────────────────────────────────────┘  │
│  ┌─────────────────────────────────────────────────┐  │
│  │  3. Process Management (process.rs)             │  │
│  │     • Load binary into memory                   │  │
│  │     • Setup execution environment               │  │
│  │     • Coordinate CPU emulation                  │  │
│  └─────────────────────────────────────────────────┘  │
│  ┌─────────────────────────────────────────────────┐  │
│  │  4. Virtual Memory (memory.rs)                  │  │
│  │     • Allocate virtual address space            │  │
│  │     • Enforce memory permissions                │  │
│  │     • Track page mapping                        │  │
│  └─────────────────────────────────────────────────┘  │
│  ┌─────────────────────────────────────────────────┐  │
│  │  5. Syscall Translation (syscall.rs)            │  │
│  │     • Intercept i386 syscalls                   │  │
│  │     • Translate to arm64 macOS syscalls         │  │
│  │     • Return results to emulated process        │  │
│  └─────────────────────────────────────────────────┘  │
│  ┌─────────────────────────────────────────────────┐  │
│  │  6. Virtual Filesystem (filesystem.rs)          │  │
│  │     • Mount directories                         │  │
│  │     • Translate paths                           │  │
│  │     • Manage file descriptors                   │  │
│  └─────────────────────────────────────────────────┘  │
│  ┌─────────────────────────────────────────────────┐  │
│  │  7. CPU Emulation (unicorn_engine)              │  │
│  │     • Fetch/decode i386 instructions            │  │
│  │     • Execute instructions                      │  │
│  │     • Manage CPU registers                      │  │
│  └─────────────────────────────────────────────────┘  │
└────────────────────────────┬───────────────────────────┘
                             │
┌────────────────────────────▼───────────────────────────┐
│         Host: arm64 macOS Kernel                       │
│    (filesystem, process, memory, networking)           │
└────────────────────────────────────────────────────────┘
```

## Execution Flow

```
1. main() starts
   │
2. Parse command-line arguments
   │
3. Initialize logging
   │
4. Create EmulationContext
   │
5. Load binary with BinaryLoader
   │   ├─ Read file
   │   ├─ Parse Mach-O header
   │   ├─ Extract segments and sections
   │   └─ Validate i386 architecture
   │
6. Create Process
   │   ├─ Initialize MemoryManager
   │   ├─ Allocate memory for segments
   │   ├─ Allocate stack and heap
   │   ├─ Setup SyscallHandler
   │   └─ Initialize VirtualFileSystem
   │
7. Execute Process
   │   ├─ Load binary code/data into memory
   │   ├─ Setup initial stack frame with args
   │   ├─ Initialize CPU registers (via Unicorn)
   │   ├─ Start CPU emulation at entry point
   │   │   ├─ Fetch instruction
   │   │   ├─ Decode instruction
   │   │   ├─ Execute instruction
   │   │   ├─ Handle exceptions/interrupts
   │   │   │   └─ Intercept syscalls
   │   │   │       ├─ Translate syscall
   │   │   │       └─ Execute handler
   │   │   └─ Repeat until process terminates
   │   └─ Cleanup resources
   │
8. main() exits
```

## Data Flow: Syscall Translation

```
i386 Application
        │
        │ (EAX=syscall_num, EDX/ECX/EBX/ESI/EDI=args)
        │
        ▼
[INT 0x80] ◄─── Syscall trigger (illegal instruction on arm64)
        │
        ▼
Unicorn exception handler
        │
        ▼
SyscallHandler::handle_syscall()
        │
        ├─ Extract syscall number and arguments
        ├─ Translate i386 macOS syscall → arm64 macOS syscall
        ├─ Execute handler
        │   ├─ Interact with MemoryManager (memory access)
        │   ├─ Interact with VirtualFileSystem (I/O)
        │   └─ Call host macOS syscalls via libc
        │
        ▼
Return value in EAX
        │
        ▼
Continue execution
```

## Memory Management Architecture

```
Virtual Address Space (Process View)
┌─────────────────────────────────────────┐
│ 0xFFFFFFFF - Kernel space               │
├─────────────────────────────────────────┤
│ 0xC0000000 - Shared libraries (ASLR)    │
├─────────────────────────────────────────┤
│             Heap (growing up)           │
│             ════════════                │
│                                         │
├─────────────────────────────────────────┤
│             Stack (growing down)        │
│             ════════════                │
│                                         │
├─────────────────────────────────────────┤
│ 0x01000000 - Binary code & data         │
├─────────────────────────────────────────┤
│ 0x00001000 - Reserved                   │
├─────────────────────────────────────────┤
│ 0x00000000 - PAGEZERO                   │
└─────────────────────────────────────────┘

Mapped to Host Memory
┌─────────────────────────────────────────┐
│  Host vectors/BTreeMap                  │
│  ┌─────────────────────────────────────┐│
│  │ Region: 0x1000-0x5000               ││
│  │ - Permissions: R/W                  ││
│  │ - Data: [0u8; 16384]                ││
│  └─────────────────────────────────────┘│
│  ┌─────────────────────────────────────┐│
│  │ Region: 0x10000-0x20000             ││
│  │ - Permissions: R/X                  ││
│  │ - Data: [code bytes...]             ││
│  └─────────────────────────────────────┘│
│  ...                                     │
└─────────────────────────────────────────┘
```

## Module Dependencies

```
main.rs
├── emulator.rs
│   └── errors.rs
├── binary_loader.rs
│   └── errors.rs
├── process.rs
│   ├── binary_loader.rs
│   ├── emulator.rs
│   ├── memory.rs
│   ├── syscall.rs
│   ├── filesystem.rs
│   └── errors.rs
├── memory.rs
│   └── errors.rs
├── syscall.rs
│   └── errors.rs
├── filesystem.rs
│   └── errors.rs
└── errors.rs
```

## Integration Points with Unicorn Engine

```
┌─────────────────────────────┐
│  mac86 Process              │
│  (emulator/process.rs)      │
└────────────┬────────────────┘
             │
             ▼
┌─────────────────────────────┐
│  Unicorn Engine             │
│  (unicorn crate)            │
├─────────────────────────────┤
│  • Initialize CPU emulator  │
│  • Map memory regions       │
│  • Set up hooks             │
│  • Start code emulation     │
└────────────┬────────────────┘
             │
    ┌────────┴────────┐
    │                 │
    ▼                 ▼
Code Hook         Exception Hook
(instruction)     (syscall/trap)
    │                 │
    └────────┬────────┘
             ▼
    mac86 Handler
    ├─ Validate state
    ├─ Execute logic
    └─ Return control to Unicorn
```

## Error Handling Strategy

```
Error Source
    │
    ├─ I/O Errors → EmulationError::IoError
    ├─ Binary Format Errors → EmulationError::BinaryLoadError
    ├─ Architecture Mismatch → EmulationError::InvalidArchitecture
    ├─ Memory Access Violation → EmulationError::MemoryError
    ├─ Syscall Failure → EmulationError::SyscallError
    ├─ File System Errors → EmulationError::FileSystemError
    ├─ Process Creation Errors → EmulationError::ProcessError
    └─ Emulation Errors → EmulationError::EmulationError
         │
         ▼
    EmulationResult<T> (Result<T, EmulationError>)
         │
         ▼
    Log error + exit gracefully
```

## Performance Considerations

1. **Instruction Caching**: Unicorn caches compiled blocks
2. **Memory Region Tracking**: BTreeMap for efficient lookups
3. **Syscall Fast Path**: Common syscalls bypass error handling
4. **Lazy Initialization**: Emulation environment initialized on-demand

## Future Architecture Enhancements

1. **Multi-threading**: Support POSIX threads (pthread)
2. **Dynamic Linking**: dlopen/dlsym support
3. **Signals**: POSIX signal handling
4. **Debugging**: GDB integration via RSP protocol
5. **Performance**: JIT compilation for hot paths
6. **Profiling**: Built-in tracing and performance analysis
