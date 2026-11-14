
llvm/objects/libkernels.kernels.95fd440ee8c76f94-cgu.0.rcgu.o:     file format elf64-x86-64


Disassembly of section .text.nested_add:

0000000000000000 <nested_add>:
   0:	48 8d 47 2a          	lea    0x2a(%rdi),%rax
   4:	c3                   	ret

Disassembly of section .text.nested_and:

0000000000000000 <nested_and>:
   0:	48 89 f8             	mov    %rdi,%rax
   3:	25 0f 00 0f 00       	and    $0xf000f,%eax
   8:	c3                   	ret

Disassembly of section .text.nested_mul:

0000000000000000 <nested_mul>:
   0:	48 8d 04 bf          	lea    (%rdi,%rdi,4),%rax
   4:	48 8d 04 87          	lea    (%rdi,%rax,4),%rax
   8:	c3                   	ret

Disassembly of section .text.nested_or:

0000000000000000 <nested_or>:
   0:	48 89 f8             	mov    %rdi,%rax
   3:	48 0d 00 ff 00 00    	or     $0xff00,%rax
   9:	c3                   	ret

Disassembly of section .text.nested_xor:

0000000000000000 <nested_xor>:
   0:	48 89 f8             	mov    %rdi,%rax
   3:	48 35 00 ff 00 00    	xor    $0xff00,%rax
   9:	c3                   	ret

Disassembly of section .text.reversed_add:

0000000000000000 <reversed_add>:
   0:	31 c0                	xor    %eax,%eax
   2:	c3                   	ret
