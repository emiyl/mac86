.section __TEXT,__text
.globl _start

_start:
    call _main

    # main() return value becomes exit status
    mov %eax, %ebx
    mov $1, %eax
    int $0x80

    # Safety: if sys_exit is not handled, stop execution.
    hlt
