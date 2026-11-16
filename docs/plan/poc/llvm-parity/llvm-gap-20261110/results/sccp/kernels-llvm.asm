
llvm/objects/libkernels.kernels.95fd440ee8c76f94-cgu.0.rcgu.o:     file format elf64-x86-64


Disassembly of section .text.changing_loop:

0000000000000000 <changing_loop>:
   0:	48 8d 47 2a          	lea    0x2a(%rdi),%rax
   4:	c3                   	ret

Disassembly of section .text.constant_branch:

0000000000000000 <constant_branch>:
   0:	b8 2a 00 00 00       	mov    $0x2a,%eax
   5:	c3                   	ret

Disassembly of section .text.dynamic_merge:

0000000000000000 <dynamic_merge>:
   0:	48 8d 4e 01          	lea    0x1(%rsi),%rcx
   4:	85 ff                	test   %edi,%edi
   6:	b8 08 00 00 00       	mov    $0x8,%eax
   b:	48 0f 44 c1          	cmove  %rcx,%rax
   f:	c3                   	ret

Disassembly of section .text.narrow_merge:

0000000000000000 <narrow_merge>:
   0:	b0 01                	mov    $0x1,%al
   2:	c3                   	ret
