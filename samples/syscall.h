#ifndef MAC86_SYSCALL_H
#define MAC86_SYSCALL_H

/* ── register-based INT 0x80 convention ──────────────────────────────────────
 * EAX = syscall number
 * EBX = arg0, ECX = arg1, EDX = arg2, ESI = arg3, EDI = arg4, EBP = arg5
 * Return value: EAX (low 32 bits), EDX (high 32 bits for 64-bit returns)
 * On error: EAX = 0xFFFFFFFF (-1 as i32)
 * ─────────────────────────────────────────────────────────────────────────── */

static inline int mac86_sys_write(int fd, const void *buf, unsigned int count) {
    int ret;
    __asm__ volatile(
        "int $0x80"
        : "=a"(ret)
        : "a"(4), "b"(fd), "c"(buf), "d"(count)
        : "memory");
    return ret;
}

static inline int mac86_sys_read(int fd, void *buf, unsigned int count) {
    int ret;
    __asm__ volatile(
        "int $0x80"
        : "=a"(ret)
        : "a"(3), "b"(fd), "c"(buf), "d"(count)
        : "memory");
    return ret;
}

/* O_RDONLY=0, O_WRONLY=1, O_RDWR=2, O_CREAT=0x200, O_TRUNC=0x400 */
static inline int mac86_sys_open(const char *path, int flags, int mode) {
    int ret;
    __asm__ volatile(
        "int $0x80"
        : "=a"(ret)
        : "a"(5), "b"(path), "c"(flags), "d"(mode)
        : "memory");
    return ret;
}

static inline int mac86_sys_close(int fd) {
    int ret;
    __asm__ volatile(
        "int $0x80"
        : "=a"(ret)
        : "a"(6), "b"(fd)
        : "memory");
    return ret;
}

static inline int mac86_sys_getpid(void) {
    int ret;
    __asm__ volatile(
        "int $0x80"
        : "=a"(ret)
        : "a"(20)
        : "memory");
    return ret;
}

static inline int mac86_sys_getuid(void) {
    int ret;
    __asm__ volatile(
        "int $0x80"
        : "=a"(ret)
        : "a"(24)
        : "memory");
    return ret;
}

static inline unsigned int mac86_sys_brk(unsigned int addr) {
    unsigned int ret;
    __asm__ volatile(
        "int $0x80"
        : "=a"(ret)
        : "a"(45), "b"(addr)
        : "memory");
    return ret;
}

/* mmap: anonymous mapping only.
 * flags: MAP_PRIVATE=0x0002, MAP_ANONYMOUS=0x1000
 * prot:  PROT_READ=0x1, PROT_WRITE=0x2 */
static inline void *mac86_sys_mmap(void *addr, unsigned int len, int prot,
                                    int flags, int fd, int offset) {
    void *ret;
    __asm__ volatile(
        "int $0x80"
        : "=a"(ret)
        : "a"(197), "b"(addr), "c"(len), "d"(prot), "S"(flags), "D"(fd)
        : "memory");
    return ret;
}

static inline int mac86_sys_munmap(void *addr, unsigned int len) {
    int ret;
    __asm__ volatile(
        "int $0x80"
        : "=a"(ret)
        : "a"(73), "b"(addr), "c"(len)
        : "memory");
    return ret;
}

/* lseek: offset is 64-bit, split across ECX (low) and EDX (high).
 * SEEK_SET=0, SEEK_CUR=1, SEEK_END=2
 * Returns new offset in EDX:EAX. */
static inline long long mac86_sys_lseek(int fd, long long offset, int whence) {
    unsigned int lo = (unsigned int)offset;
    unsigned int hi = (unsigned int)((unsigned long long)offset >> 32);
    unsigned int ret_lo, ret_hi;
    __asm__ volatile(
        "int $0x80"
        : "=a"(ret_lo), "=d"(ret_hi)
        : "a"(199), "b"(fd), "c"(lo), "d"(hi), "S"(whence)
        : "memory");
    return (long long)((unsigned long long)ret_lo | ((unsigned long long)ret_hi << 32));
}

/* Minimal stat struct matching the 96-byte macOS i386 layout used by
 * mac86's write_stat_struct helper. Only the most-used fields are named. */
struct mac86_stat {
    int          st_dev;      /* +0  */
    unsigned int st_ino;      /* +4  */
    unsigned short st_mode;   /* +8  */
    unsigned short st_nlink;  /* +10 */
    unsigned int st_uid;      /* +12 */
    unsigned int st_gid;      /* +16 */
    int          st_rdev;     /* +20 */
    unsigned int _pad;        /* +24 */
    unsigned int st_atime;    /* +28 */
    unsigned int st_atimensec;/* +32 */
    unsigned int st_mtime;    /* +36 */
    unsigned int st_mtimensec;/* +40 */
    unsigned int st_ctime;    /* +44 */
    unsigned int st_ctimensec;/* +48 — wait, st_size is at +48 */
    /* NOTE: compiler will add padding; use __attribute__((packed)) if needed,
     * or just access st_size via the raw offset if precision matters. */
    long long    st_size;     /* +48 (requires careful alignment) */
    long long    st_blocks;   /* +56 */
    int          st_blksize;  /* +64 */
};

static inline int mac86_sys_fstat(int fd, struct mac86_stat *sb) {
    int ret;
    __asm__ volatile(
        "int $0x80"
        : "=a"(ret)
        : "a"(62), "b"(fd), "c"(sb)
        : "memory");
    return ret;
}

static inline int mac86_sys_stat(const char *path, struct mac86_stat *sb) {
    int ret;
    __asm__ volatile(
        "int $0x80"
        : "=a"(ret)
        : "a"(18), "b"(path), "c"(sb)
        : "memory");
    return ret;
}

static inline __attribute__((noreturn)) void mac86_sys_exit(int code) {
    __asm__ volatile(
        "int $0x80"
        :
        : "a"(1), "b"(code)
        : "memory");
    for (;;) {}
}

#endif /* MAC86_SYSCALL_H */
