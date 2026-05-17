.section __TEXT,__text
.globl _start

_start:
    mov $4, %eax          # sys_write
    mov $1, %ebx          # stdout
    mov $msg, %ecx
    mov $len, %edx
    int $0x80

    mov $1, %eax          # sys_exit
    xor %ebx, %ebx
    int $0x80

.section __TEXT,__cstring
msg:
    .ascii "hello world\n"
len = . - msg