
llvm/objects/libkernels.kernels.95fd440ee8c76f94-cgu.0.rcgu.o:     file format elf64-x86-64


Disassembly of section .text.byte_shift:

0000000000000000 <byte_shift>:
   0:	40 c0 ef 02          	shr    $0x2,%dil
   4:	40 0f b6 c7          	movzbl %dil,%eax
   8:	c3                   	ret

Disassembly of section .text.invert_mask:

0000000000000000 <invert_mask>:
   0:	89 f8                	mov    %edi,%eax
   2:	f6 d0                	not    %al
   4:	0c f0                	or     $0xf0,%al
   6:	c3                   	ret

Disassembly of section .text.low_shift:

0000000000000000 <low_shift>:
   0:	48 8d 04 fd 00 00 00 	lea    0x0(,%rdi,8),%rax
   7:	00 
   8:	c3                   	ret

Disassembly of section .text.sign_shift:

0000000000000000 <sign_shift>:
   0:	89 f8                	mov    %edi,%eax
   2:	c0 e8 03             	shr    $0x3,%al
   5:	0c f0                	or     $0xf0,%al
   7:	c3                   	ret
