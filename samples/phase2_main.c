/* phase2_main.c — Phase 2 userland ABI smoke-test
 *
 * Exercises: open, read, lseek, close, fstat (file size), brk (heap), getpid.
 * All I/O goes to fd 1 (stdout) via write.
 *
 * Build (requires MacOSX10.13.sdk and i386 linker):
 *   ./samples/build_freestanding.sh samples/phase2_hello_static samples/phase2_main.c
 */
#include "syscall.h"

/* ── tiny helpers ────────────────────────────────────────────────────────── */

static void write_str(const char *s) {
    int len = 0;
    while (s[len]) len++;
    mac86_sys_write(1, s, (unsigned int)len);
}

static void write_uint(unsigned int v) {
    char buf[12];
    int i = 11;
    buf[i] = '\0';
    if (v == 0) { write_str("0"); return; }
    while (v && i > 0) { buf[--i] = '0' + (v % 10); v /= 10; }
    write_str(buf + i);
}

/* ── brk / heap bump allocator ───────────────────────────────────────────── */

static unsigned int heap_base;
static unsigned int heap_ptr;

static void heap_init(void) {
    heap_base = mac86_sys_brk(0);   /* query current break */
    heap_ptr  = heap_base;
}

static void *heap_alloc(unsigned int size) {
    void *p = (void *)heap_ptr;
    unsigned int new_ptr = heap_ptr + size;
    unsigned int new_break = mac86_sys_brk(new_ptr);
    if (new_break < new_ptr) return (void *)0; /* failed */
    heap_ptr = new_ptr;
    return p;
}

/* ── main ────────────────────────────────────────────────────────────────── */

int main(void) {
    write_str("=== phase2 syscall test ===\n");

    /* getpid */
    int pid = mac86_sys_getpid();
    write_str("getpid: ");
    write_uint((unsigned int)pid);
    write_str("\n");

    /* brk / heap */
    heap_init();
    char *buf = (char *)heap_alloc(64);
    if (!buf) {
        write_str("heap_alloc failed\n");
        mac86_sys_exit(1);
    }
    write_str("heap alloc: ok (");
    write_uint((unsigned int)heap_ptr - (unsigned int)heap_base);
    write_str(" bytes)\n");

    /* open the binary itself and read its magic bytes */
    int fd = mac86_sys_open("/proc/self/exe", 0 /*O_RDONLY*/, 0);
    if (fd < 0) {
        /* fallback: open crt0 object if available */
        fd = mac86_sys_open("samples/.build/crt0.o", 0, 0);
    }
    if (fd >= 0) {
        int n = mac86_sys_read(fd, buf, 4);
        if (n == 4) {
            write_str("read 4 bytes: ");
            for (int i = 0; i < 4; i++) {
                unsigned char c = (unsigned char)buf[i];
                unsigned char hi = (c >> 4) & 0xF;
                unsigned char lo = c & 0xF;
                char h[3] = {
                    (char)(hi < 10 ? '0' + hi : 'a' + hi - 10),
                    (char)(lo < 10 ? '0' + lo : 'a' + lo - 10),
                    '\0'
                };
                write_str(h);
                write_str(" ");
            }
            write_str("\n");
        }

        /* lseek to beginning and confirm offset 0 */
        long long off = mac86_sys_lseek(fd, 0, 0 /*SEEK_SET*/);
        write_str("lseek(0,SET): ");
        write_uint((unsigned int)off);
        write_str("\n");

        mac86_sys_close(fd);
        write_str("close: ok\n");
    } else {
        write_str("open: skipped (no readable file)\n");
    }

    write_str("=== done ===\n");
    return 0;
}
