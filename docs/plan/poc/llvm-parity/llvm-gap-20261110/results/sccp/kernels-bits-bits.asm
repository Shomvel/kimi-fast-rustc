
bits/objects/libkernels.kernels.95fd440ee8c76f94-cgu.0.rcgu.o:     file format elf64-x86-64


Disassembly of section .text:

0000000000000000 <byte_shift>:
   0:	40 0f b6 c7          	movzbl %dil,%eax
   4:	b9 02 00 00 00       	mov    $0x2,%ecx
   9:	48 d3 e8             	shr    %cl,%rax
   c:	c3                   	ret
   d:	00 00                	add    %al,(%rax)
	...

0000000000000010 <invert_mask>:
  10:	40 0f b6 c7          	movzbl %dil,%eax
  14:	48 83 e0 0f          	and    $0xf,%rax
  18:	48 f7 d0             	not    %rax
  1b:	0f b6 c0             	movzbl %al,%eax
  1e:	c3                   	ret
	...

0000000000000020 <low_shift>:
  20:	b9 03 00 00 00       	mov    $0x3,%ecx
  25:	48 d3 e7             	shl    %cl,%rdi
  28:	48 89 f8             	mov    %rdi,%rax
  2b:	c3                   	ret
  2c:	00 00                	add    %al,(%rax)
	...

0000000000000030 <sign_shift>:
  30:	48 0f be c7          	movsbq %dil,%rax
  34:	48 83 c8 80          	or     $0xffffffffffffff80,%rax
  38:	b9 03 00 00 00       	mov    $0x3,%ecx
  3d:	48 d3 f8             	sar    %cl,%rax
  40:	c3                   	ret
