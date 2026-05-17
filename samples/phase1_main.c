#include "syscall.h"

int main(void) {
    static const char msg[] = "hello from freestanding c\n";
    mac86_sys_write(1, msg, (unsigned int)(sizeof(msg) - 1));
    return 0;
}
