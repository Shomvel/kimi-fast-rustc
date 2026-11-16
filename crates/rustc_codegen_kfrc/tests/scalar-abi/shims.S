# x86-64 Linux System V only. Preserve all callee-saved registers.
# Each poisoned bit is outside the source ABI extension obligation.
.text
.globl check_c_boolean
.type check_c_boolean, @function
check_c_boolean:
    movzbl %dil, %eax
    cmpl %eax, %edi
    jne .Lbad_boolean
    ret
.Lbad_boolean:
    ud2
.size check_c_boolean, .-check_c_boolean

.globl dirty_c_boolean
.type dirty_c_boolean, @function
dirty_c_boolean:
    movabsq $0xa55aaa55a5a5a500, %rax
    movb %dil, %al
    ret
.size dirty_c_boolean, .-dirty_c_boolean

.globl poison_c_boolean
.type poison_c_boolean, @function
poison_c_boolean:
    movq %rdi, %r11
    movzbl %sil, %eax
    movabsq $0xa55aaa5500000000, %rdi
    orq %rax, %rdi
    subq $8, %rsp
    call *%r11
    addq $8, %rsp
    ret
.size poison_c_boolean, .-poison_c_boolean

.globl dirty_rust_boolean
.type dirty_rust_boolean, @function
dirty_rust_boolean:
    movabsq $0xa55aaa55a5a5a500, %rax
    movb %dil, %al
    ret
.size dirty_rust_boolean, .-dirty_rust_boolean

.globl poison_rust_boolean
.type poison_rust_boolean, @function
poison_rust_boolean:
    movq %rdi, %r11
    movzbl %sil, %eax
    movabsq $0xa55aaa5500000000, %rdi
    orq %rax, %rdi
    subq $8, %rsp
    call *%r11
    addq $8, %rsp
    ret
.size poison_rust_boolean, .-poison_rust_boolean

.globl check_c_unsigned8
.type check_c_unsigned8, @function
check_c_unsigned8:
    movzbl %dil, %eax
    cmpl %eax, %edi
    jne .Lbad_unsigned8
    ret
.Lbad_unsigned8:
    ud2
.size check_c_unsigned8, .-check_c_unsigned8

.globl dirty_c_unsigned8
.type dirty_c_unsigned8, @function
dirty_c_unsigned8:
    movabsq $0xa55aaa55a5a5a500, %rax
    movb %dil, %al
    ret
.size dirty_c_unsigned8, .-dirty_c_unsigned8

.globl poison_c_unsigned8
.type poison_c_unsigned8, @function
poison_c_unsigned8:
    movq %rdi, %r11
    movzbl %sil, %eax
    movabsq $0xa55aaa5500000000, %rdi
    orq %rax, %rdi
    subq $8, %rsp
    call *%r11
    addq $8, %rsp
    ret
.size poison_c_unsigned8, .-poison_c_unsigned8

.globl dirty_rust_unsigned8
.type dirty_rust_unsigned8, @function
dirty_rust_unsigned8:
    movabsq $0xa55aaa55a5a5a500, %rax
    movb %dil, %al
    ret
.size dirty_rust_unsigned8, .-dirty_rust_unsigned8

.globl poison_rust_unsigned8
.type poison_rust_unsigned8, @function
poison_rust_unsigned8:
    movq %rdi, %r11
    movzbl %sil, %eax
    movabsq $0xa55aaa55a5a5a500, %rdi
    orq %rax, %rdi
    subq $8, %rsp
    call *%r11
    addq $8, %rsp
    ret
.size poison_rust_unsigned8, .-poison_rust_unsigned8

.globl check_c_signed8
.type check_c_signed8, @function
check_c_signed8:
    movsbl %dil, %eax
    cmpl %eax, %edi
    jne .Lbad_signed8
    movsbq %dil, %rax
    ret
.Lbad_signed8:
    ud2
.size check_c_signed8, .-check_c_signed8

.globl dirty_c_signed8
.type dirty_c_signed8, @function
dirty_c_signed8:
    movabsq $0xa55aaa55a5a5a500, %rax
    movb %dil, %al
    ret
.size dirty_c_signed8, .-dirty_c_signed8

.globl poison_c_signed8
.type poison_c_signed8, @function
poison_c_signed8:
    movq %rdi, %r11
    movsbl %sil, %eax
    movabsq $0xa55aaa5500000000, %rdi
    orq %rax, %rdi
    subq $8, %rsp
    call *%r11
    addq $8, %rsp
    ret
.size poison_c_signed8, .-poison_c_signed8

.globl dirty_rust_signed8
.type dirty_rust_signed8, @function
dirty_rust_signed8:
    movabsq $0xa55aaa55a5a5a500, %rax
    movb %dil, %al
    ret
.size dirty_rust_signed8, .-dirty_rust_signed8

.globl poison_rust_signed8
.type poison_rust_signed8, @function
poison_rust_signed8:
    movq %rdi, %r11
    movzbl %sil, %eax
    movabsq $0xa55aaa55a5a5a500, %rdi
    orq %rax, %rdi
    subq $8, %rsp
    call *%r11
    addq $8, %rsp
    ret
.size poison_rust_signed8, .-poison_rust_signed8

.globl check_c_unsigned16
.type check_c_unsigned16, @function
check_c_unsigned16:
    movzwl %di, %eax
    cmpl %eax, %edi
    jne .Lbad_unsigned16
    ret
.Lbad_unsigned16:
    ud2
.size check_c_unsigned16, .-check_c_unsigned16

.globl dirty_c_unsigned16
.type dirty_c_unsigned16, @function
dirty_c_unsigned16:
    movabsq $0xa55aaa55a5a50000, %rax
    movw %di, %ax
    ret
.size dirty_c_unsigned16, .-dirty_c_unsigned16

.globl poison_c_unsigned16
.type poison_c_unsigned16, @function
poison_c_unsigned16:
    movq %rdi, %r11
    movzwl %si, %eax
    movabsq $0xa55aaa5500000000, %rdi
    orq %rax, %rdi
    subq $8, %rsp
    call *%r11
    addq $8, %rsp
    ret
.size poison_c_unsigned16, .-poison_c_unsigned16

.globl dirty_rust_unsigned16
.type dirty_rust_unsigned16, @function
dirty_rust_unsigned16:
    movabsq $0xa55aaa55a5a50000, %rax
    movw %di, %ax
    ret
.size dirty_rust_unsigned16, .-dirty_rust_unsigned16

.globl poison_rust_unsigned16
.type poison_rust_unsigned16, @function
poison_rust_unsigned16:
    movq %rdi, %r11
    movzwl %si, %eax
    movabsq $0xa55aaa55a5a50000, %rdi
    orq %rax, %rdi
    subq $8, %rsp
    call *%r11
    addq $8, %rsp
    ret
.size poison_rust_unsigned16, .-poison_rust_unsigned16

.globl check_c_signed16
.type check_c_signed16, @function
check_c_signed16:
    movswl %di, %eax
    cmpl %eax, %edi
    jne .Lbad_signed16
    movswq %di, %rax
    ret
.Lbad_signed16:
    ud2
.size check_c_signed16, .-check_c_signed16

.globl dirty_c_signed16
.type dirty_c_signed16, @function
dirty_c_signed16:
    movabsq $0xa55aaa55a5a50000, %rax
    movw %di, %ax
    ret
.size dirty_c_signed16, .-dirty_c_signed16

.globl poison_c_signed16
.type poison_c_signed16, @function
poison_c_signed16:
    movq %rdi, %r11
    movswl %si, %eax
    movabsq $0xa55aaa5500000000, %rdi
    orq %rax, %rdi
    subq $8, %rsp
    call *%r11
    addq $8, %rsp
    ret
.size poison_c_signed16, .-poison_c_signed16

.globl dirty_rust_signed16
.type dirty_rust_signed16, @function
dirty_rust_signed16:
    movabsq $0xa55aaa55a5a50000, %rax
    movw %di, %ax
    ret
.size dirty_rust_signed16, .-dirty_rust_signed16

.globl poison_rust_signed16
.type poison_rust_signed16, @function
poison_rust_signed16:
    movq %rdi, %r11
    movzwl %si, %eax
    movabsq $0xa55aaa55a5a50000, %rdi
    orq %rax, %rdi
    subq $8, %rsp
    call *%r11
    addq $8, %rsp
    ret
.size poison_rust_signed16, .-poison_rust_signed16

.globl check_c_unsigned32
.type check_c_unsigned32, @function
check_c_unsigned32:
    movl %edi, %eax
    ret
.Lbad_unsigned32:
    ud2
.size check_c_unsigned32, .-check_c_unsigned32

.globl dirty_c_unsigned32
.type dirty_c_unsigned32, @function
dirty_c_unsigned32:
    movl %edi, %eax
    movabsq $0xa55aaa5500000000, %r10
    orq %r10, %rax
    ret
.size dirty_c_unsigned32, .-dirty_c_unsigned32

.globl poison_c_unsigned32
.type poison_c_unsigned32, @function
poison_c_unsigned32:
    movq %rdi, %r11
    movl %esi, %eax
    movabsq $0xa55aaa5500000000, %rdi
    orq %rax, %rdi
    subq $8, %rsp
    call *%r11
    addq $8, %rsp
    ret
.size poison_c_unsigned32, .-poison_c_unsigned32

.globl dirty_rust_unsigned32
.type dirty_rust_unsigned32, @function
dirty_rust_unsigned32:
    movl %edi, %eax
    movabsq $0xa55aaa5500000000, %r10
    orq %r10, %rax
    ret
.size dirty_rust_unsigned32, .-dirty_rust_unsigned32

.globl poison_rust_unsigned32
.type poison_rust_unsigned32, @function
poison_rust_unsigned32:
    movq %rdi, %r11
    movl %esi, %eax
    movabsq $0xa55aaa5500000000, %rdi
    orq %rax, %rdi
    subq $8, %rsp
    call *%r11
    addq $8, %rsp
    ret
.size poison_rust_unsigned32, .-poison_rust_unsigned32

.globl check_c_signed32
.type check_c_signed32, @function
check_c_signed32:
    movl %edi, %eax
    movslq %edi, %rax
    ret
.Lbad_signed32:
    ud2
.size check_c_signed32, .-check_c_signed32

.globl dirty_c_signed32
.type dirty_c_signed32, @function
dirty_c_signed32:
    movl %edi, %eax
    movabsq $0xa55aaa5500000000, %r10
    orq %r10, %rax
    ret
.size dirty_c_signed32, .-dirty_c_signed32

.globl poison_c_signed32
.type poison_c_signed32, @function
poison_c_signed32:
    movq %rdi, %r11
    movl %esi, %eax
    movabsq $0xa55aaa5500000000, %rdi
    orq %rax, %rdi
    subq $8, %rsp
    call *%r11
    addq $8, %rsp
    ret
.size poison_c_signed32, .-poison_c_signed32

.globl dirty_rust_signed32
.type dirty_rust_signed32, @function
dirty_rust_signed32:
    movl %edi, %eax
    movabsq $0xa55aaa5500000000, %r10
    orq %r10, %rax
    ret
.size dirty_rust_signed32, .-dirty_rust_signed32

.globl poison_rust_signed32
.type poison_rust_signed32, @function
poison_rust_signed32:
    movq %rdi, %r11
    movl %esi, %eax
    movabsq $0xa55aaa5500000000, %rdi
    orq %rax, %rdi
    subq $8, %rsp
    call *%r11
    addq $8, %rsp
    ret
.size poison_rust_signed32, .-poison_rust_signed32

.globl dirty_pair
.type dirty_pair, @function
dirty_pair:
    movabsq $0xa55aaa55a5a50000, %rax
    movw %di, %ax
    movabsq $0x5aa555aa5a5a5a00, %rdx
    movb %sil, %dl
    ret
.size dirty_pair, .-dirty_pair

.globl check_pair
.type check_pair, @function
check_pair:
    movzbl %sil, %eax
    cmpl %eax, %esi
    jne .Lbad_pair
    movl %esi, %ecx
    movswq %di, %rax
    addq %rcx, %rax
    ret
.Lbad_pair:
    ud2
.size check_pair, .-check_pair

.globl poison_stack_rust
.type poison_stack_rust, @function
poison_stack_rust:
    movq %rdi, %r11
    movl %edx, %r10d
    movzwl %si, %eax
    movabsq $0xa55aaa55a5a50000, %rdx
    orq %rdx, %rax
    subq $24, %rsp
    movq %rax, 0(%rsp)
    movabsq $0x5aa555aa00000000, %rax
    orq %r10, %rax
    movq %rax, 8(%rsp)
    movq $1, %rdi
    movq $2, %rsi
    movq $4, %rdx
    movq $8, %rcx
    movq $16, %r8
    movq $32, %r9
    call *%r11
    addq $24, %rsp
    ret
.size poison_stack_rust, .-poison_stack_rust

.globl poison_stack_c
.type poison_stack_c, @function
poison_stack_c:
    movq %rdi, %r11
    movl %edx, %r10d
    movswl %si, %eax
    movabsq $0xa55aaa5500000000, %rdx
    orq %rdx, %rax
    subq $24, %rsp
    movq %rax, 0(%rsp)
    movabsq $0x5aa555aa00000000, %rax
    orq %r10, %rax
    movq %rax, 8(%rsp)
    movq $1, %rdi
    movq $2, %rsi
    movq $4, %rdx
    movq $8, %rcx
    movq $16, %r8
    movq $32, %r9
    call *%r11
    addq $24, %rsp
    ret
.size poison_stack_c, .-poison_stack_c

.section .note.GNU-stack,"",@progbits

.globl check_signed_wrapper
.set check_signed_wrapper, check_c_signed8
