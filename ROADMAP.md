# Implementation Roadmap

## Phase 1: Foundation ✅
- [x] Project structure and module organization
- [x] Binary loading (Mach-O static parser)
- [x] Error handling framework
- [x] Memory management (Unicorn-backed 2 GB flat map)
- [x] Syscall dispatch infrastructure (INT 0x80, register convention)
- [x] Virtual filesystem abstraction (VFS with host passthrough)
- [x] Process management and execution loop
- [x] Sample freestanding binary (`phase1_hello_static`)

## Phase 2: Tiny Userland ABI ✅
- [x] Unicorn Engine integration (x86/i386, code hooks)
- [x] Correct i386 SysV startup stack (argc/argv*/envp*)
- [x] INT 0x80 → syscall module wiring
- [x] exit, read, write, open, close, stat, fstat, lseek
- [x] brk (heap) and anonymous mmap/munmap
- [x] getpid, getuid
- [x] `--trace-syscalls` and `--trace-instr` flags
- [x] Golden tests for sample binaries (`tests/golden.rs`)
- [x] lib/bin split (`src/lib.rs`) enabling integration tests
- [x] Phase 2 sample program (`phase2_main.c`)

## Phase 3: Dynamic Linking — First Slice 🔄
- [x] Parse LC_DYLD_INFO bind + lazy-bind opcodes
- [x] Fallback: classic LC_DYSYMTAB + section `reserved1` binding
- [x] LC_MAIN support (direct entry to `main()`, bypasses crt1)
- [x] Remove dynamic-binary rejection; allow dynamically linked MH_EXECUTE
- [x] libSystem trampoline region (0x50000000, one slot per import)
- [x] Load-time symbol resolution: fill `__nl_symbol_ptr` / `__la_symbol_ptr` slots
- [x] libSystem handlers: write, read, open, close, exit, malloc, free, calloc
- [x] libSystem handlers: puts, printf (common format specifiers), fprintf, strlen,
       strcmp, strcpy, memcpy, memset, memmove
- [ ] Symbol version suffix stripping (`$NOCANCEL$UNIX2003` etc.)
- [ ] Weak bindings
- [ ] Rebase opcodes (ASLR — currently assumes zero slide)
- [ ] Lazy binding fallback (dyld_stub_binder interception)

## Phase 4: Broader Syscall Coverage
- [ ] fcntl, ioctl, dup/dup2
- [ ] select / poll
- [ ] sysctl (basic: hw.ncpu, kern.osrelease)
- [ ] sigaction / sigprocmask (minimal, no real delivery)
- [ ] gettimeofday, clock_gettime
- [ ] readdir / getdents
- [ ] mprotect
- [ ] Errno mapping (EAX = -errno on error, not just -1)

## Phase 5: Threads & Signals
- [ ] pthread_create / pthread_join (Unicorn multi-instance approach)
- [ ] pthread_mutex_lock/unlock (host mutex passthrough)
- [ ] Signal delivery (SIGTERM, SIGINT, SIGHUP)
- [ ] setjmp / longjmp (already works via register save/restore)

## Phase 6: Advanced Dynamic Linking
- [ ] Multiple dylib loading (libm, libc++, CoreFoundation stubs)
- [ ] dlopen / dlsym / dlclose
- [ ] ASLR slide computation and rebase
- [ ] Objective-C runtime stubs (enough for NSLog)
- [ ] Lazy binding (dyld_stub_binder simulation)

## Phase 7: Real-World Compatibility
- [ ] Fat/Universal binary support (extract i386 slice)
- [ ] Exception handling (Mach exceptions → signals)
- [ ] Core Foundation minimal stubs
- [ ] Graphics/UI: headless NSApplication stub
- [ ] Networking (socket syscalls passthrough)

---

## Technical Debt

| Area | Item |
|------|------|
| `memory.rs` | MemoryManager unused after Unicorn flat-map; remove or repurpose for mmap tracking |
| `syscall.rs` | Full errno mapping (ENOENT, EACCES, EBADF, …) |
| `filesystem.rs` | readdir, symlink, `/dev/null`, `/dev/random` |
| `binary_loader.rs` | Fat binary slice extraction |
| `process.rs` | envp population (currently empty) |
| tests | stdout capture in golden tests (needs VFS output-buffer mode) |
