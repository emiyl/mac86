# Implementation Roadmap

## Phase 1: Foundation (Current)
- [x] Project structure and module organization
- [x] Binary loading (Mach-O parser)
- [x] Error handling framework
- [x] Memory management system
- [x] Syscall dispatch infrastructure
- [x] Virtual filesystem abstraction
- [x] Process management skeleton
- [ ] Build and basic compilation

## Phase 2: CPU Emulation
- [ ] Integrate Unicorn Engine
- [ ] Initialize x86/i386 CPU state
- [ ] Implement instruction dispatch hooks
- [ ] Implement exception/interrupt handlers
- [ ] Map memory regions to Unicorn
- [ ] Load binary code into emulated memory
- [ ] Basic instruction execution

## Phase 3: Essential Syscalls
- [ ] Implement exit() fully
- [ ] Implement read/write for basic I/O
- [ ] Implement open/close for file operations
- [ ] Implement mmap/munmap for memory mapping
- [ ] Implement stat/fstat for file metadata
- [ ] Implement brk for heap management
- [ ] Implement getpid/getuid/geteuid

## Phase 4: Testing & Debugging
- [ ] Create minimal i386 test binaries
- [ ] Write unit tests for each module
- [ ] Add integration tests
- [ ] Debug with real i386 applications
- [ ] Performance profiling

## Phase 5: Advanced Features
- [ ] Multi-threading support
- [ ] Signal handling
- [ ] Dynamic linking (dlopen)
- [ ] More syscalls (fork, pipe, etc.)
- [ ] Environment variables
- [ ] Command-line arguments

## Phase 6: Compatibility
- [ ] Standard C library functions
- [ ] Common frameworks
- [ ] Graphics/GUI support (stretch goal)
- [ ] Networking (stretch goal)

## Technical Debt & TODOs

### memory.rs
- [ ] Implement page-based protection
- [ ] Add COW (copy-on-write) support
- [ ] Optimize memory region lookups

### syscall.rs
- [ ] Implement remaining BSD syscalls (~150 total)
- [ ] Add errno mapping (i386 → arm64)
- [ ] Handle signal-safe syscalls

### filesystem.rs
- [ ] Implement readdir properly
- [ ] Add file locking support
- [ ] Handle special files (/dev/*, /proc/*)

### process.rs
- [ ] Load sections into memory
- [ ] Setup proper stack frame
- [ ] Initialize all CPU registers
- [ ] Implement process cleanup

### binary_loader.rs
- [ ] Support Fat/Universal binaries
- [ ] Handle dylibs properly
- [ ] Parse LC_MAIN more robustly
- [ ] Validate against ARM64 incompatibilities

## Documentation Needed
- [ ] Syscall translation table
- [ ] Mach-O format reference guide
- [ ] i386 register diagram
- [ ] Memory layout diagram
- [ ] Contributing guidelines
- [ ] Debugging guide

## Testing Strategy

### Unit Tests
- Memory allocation/deallocation
- Syscall dispatch routing
- Path resolution
- Binary parsing

### Integration Tests
- Simple "Hello World" program
- File I/O operations
- Memory allocation tests
- Syscall translation accuracy

### Real-World Tests
- Legacy applications from the wild
- Open-source i386 macOS projects
- Benchmark suite

## Known Issues to Address

1. **Unicorn Integration**: Need to handle CPU state properly
2. **Mach-O Parsing**: Fat binaries not yet supported
3. **Error Propagation**: Some errors could be more specific
4. **Performance**: No optimization yet
5. **Thread Safety**: Not thread-safe in current implementation
