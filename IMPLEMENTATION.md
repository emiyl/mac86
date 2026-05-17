# Implementation Guide for mac86

## Current Status

The mac86 project has been scaffolded with a complete modular architecture ready for implementation. The following has been completed:

### ✅ Completed
- [x] Project structure and module organization
- [x] Cargo.toml with all necessary dependencies
- [x] Binary loader with Mach-O parsing (partial)
- [x] Error handling framework
- [x] Memory management system skeleton
- [x] Syscall dispatch infrastructure
- [x] Virtual filesystem abstraction (partial)
- [x] Process management skeleton
- [x] Documentation (README, ARCHITECTURE, QUICKSTART, ROADMAP)
- [x] Successful compilation

### 🔄 In Progress / Next Steps

#### 1. Unicorn Engine Integration
**File**: `src/process.rs`

Add CPU emulation using the Unicorn Engine:

```rust
use unicorn_engine::unicorn_const::*;
use unicorn_engine::Unicorn;

// Initialize x86 emulator
let mut emu = Unicorn::new(UC_ARCH_X86, UC_MODE_32)?;

// Map memory
emu.mem_map(0x1000, 0x100000, UC_PROT_ALL)?;

// Load code into memory
emu.mem_write(0x1000, &code_bytes)?;

// Set initial registers
emu.reg_write(UC_X86_REG_ESP, 0x10000)?;
emu.reg_write(UC_X86_REG_EIP, entry_point)?;

// Start execution
emu.emu_start(entry_point, 0x10000, 0, 0)?;
```

#### 2. Complete Syscall Implementation
**File**: `src/syscall.rs`

Implement key BSD syscalls for i386 macOS:

```rust
// Example: mmap implementation
self.register(197, |args| {
    let addr = args.arg0;
    let len = args.arg1;
    let prot = args.arg2;
    let flags = args.arg3;
    // Allocate memory and return address
    Ok(addr)
});
```

#### 3. Binary Loading into Memory
**File**: `src/process.rs` - `execute()` method

Load segments and sections:

```rust
// Load segments into emulated memory
for segment in &self.binary.segments {
    if segment.filesize > 0 {
        let data = &segment.data; // from binary loader
        self.memory_manager.write(segment.vaddr, data)?;
    }
}
```

#### 4. Stack Setup
**File**: `src/process.rs`

Initialize stack with argc/argv:

```rust
// Setup stack frame
let stack_ptr = 0x80000000u32; // Top of stack

// Write argc
let argc = args.len() as u32;
memory.write(stack_ptr - 4, &argc.to_le_bytes())?;

// Write argv pointers and strings
for (i, arg) in args.iter().enumerate() {
    // Write argument string
    // Write pointer to string in argv array
}
```

#### 5. Exception Handling
**File**: `src/process.rs`

Setup Unicorn hooks for syscalls and interrupts:

```rust
// Hook for interrupts (syscalls use INT 0x80)
emu.add_intr_hook(|emu, intno| {
    if intno == 0x80 {
        // Extract syscall number from EAX
        let syscall_num = emu.reg_read(UC_X86_REG_EAX)?;
        // Handle syscall
        syscall_handler.handle_syscall(syscall_num)?;
    }
    Ok(())
})?;
```

#### 6. Expand Binary Loader
**File**: `src/binary_loader.rs`

Improve parsing:

- [ ] Support for DYLD_INSERT_LIBRARIES
- [ ] Handle LC_THREAD load commands for entry point
- [ ] Support for ASLR
- [ ] Better segment permission mapping

#### 7. Complete Memory Manager
**File**: `src/memory.rs`

Add advanced features:

- [ ] Page-based protection
- [ ] Copy-on-Write (COW)
- [ ] Memory mapping statistics
- [ ] Bounds checking improvements

## Implementation Phases

### Phase 1: Basic Execution (Weeks 1-2)
1. Integrate Unicorn Engine
2. Load binary into memory
3. Set up initial CPU state
4. Implement basic instruction fetch/decode loop
5. Test with simple "hello world" equivalent

### Phase 2: Essential Syscalls (Weeks 3-4)
1. Implement exit, read, write
2. Implement open, close, stat
3. Implement mmap, munmap, brk
4. Add proper error handling
5. Test file I/O

### Phase 3: Advanced Features (Weeks 5-6)
1. Threading support (pthread)
2. Signal handling
3. Environment variables
4. Command-line argument parsing
5. Performance profiling

## Testing Strategy

### Unit Tests
```bash
cargo test
```

### Integration Tests
Create test binaries and verify:
```rust
#[test]
fn test_simple_addition() {
    let output = Command::new("./target/release/mac86")
        .arg("test_binaries/add.i386")
        .output()
        .expect("Failed to run binary");
    
    assert!(output.status.success());
}
```

### Real-World Testing
Test with actual i386 macOS applications once core functionality works.

## Debugging Tips

### Print CPU State
```rust
println!("EAX: 0x{:x}", emu.reg_read(UC_X86_REG_EAX)?);
println!("EBX: 0x{:x}", emu.reg_read(UC_X86_REG_EBX)?);
// ... other registers
```

### Trace Instructions
```rust
emu.add_code_hook(|emu, addr, size| {
    println!("Executing at 0x{:x}, size: {}", addr, size);
    Ok(())
})?;
```

### Monitor Syscalls
```rust
log::debug!("Syscall: {} with args: {:?}", syscall_num, args);
```

## Performance Considerations

1. **Caching**: Unicorn caches compiled blocks automatically
2. **Memory**: Use sparse memory representation for large gaps
3. **Syscalls**: Fast path for common syscalls
4. **I/O**: Buffer filesystem operations

## Common Pitfalls

1. **Byte Order**: i386 is little-endian
2. **Stack Direction**: Stack grows downward in x86
3. **Return Values**: Returned in EAX (and EDX:EAX for 64-bit)
4. **ABI Compliance**: Follow System V i386 ABI
5. **Syscall Numbers**: macOS syscalls differ from Linux/BSD

## Reference Implementation Points

- **Wine**: Excellent reference for syscall translation
- **Darling**: macOS compatibility layer patterns
- **QEMU**: CPU emulation techniques
- **Linux i386 emulation**: BSD syscall mappings

## Next Immediate Step

Start with Unicorn integration in `src/process.rs`. This unblocks all subsequent work and allows testing of the memory and syscall infrastructure.

```bash
# To begin:
1. Study Unicorn Engine API
2. Add unicorn = "0.3" to Cargo.toml (already done)
3. Implement initialize_cpu() in Process
4. Implement execute() with CPU emulation loop
5. Test with a simple binary
```

Good luck! 🚀
