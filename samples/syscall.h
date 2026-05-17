#ifndef MAC86_SYSCALL_H
#define MAC86_SYSCALL_H

static inline int mac86_sys_write(int fd, const void *buf, unsigned int count) {
    int ret;
    __asm__ volatile(
        "int $0x80"
        : "=a"(ret)
        : "a"(4), "b"(fd), "c"(buf), "d"(count)
        : "memory");
    return ret;
}

static inline __attribute__((noreturn)) void mac86_sys_exit(int code) {
    __asm__ volatile(
        "int $0x80"
        :
        : "a"(1), "b"(code)
        : "memory");

    for (;;) {
    }
}

#endif
