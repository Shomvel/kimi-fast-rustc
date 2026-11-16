
baseline/objects/libkernels.kernels.95fd440ee8c76f94-cgu.0.rcgu.o:     file format elf64-x86-64


Disassembly of section .text:

0000000000000000 <byte_shift>:
   0:	40 0f b6 c7          	movzbl %dil,%eax
   4:	b9 02 00 00 00       	mov    $0x2,%ecx
   9:	48 d3 e8             	shr    %cl,%rax
   c:	48 83 e0 3f          	and    $0x3f,%rax
  10:	c3                   	ret
	...

0000000000000020 <invert_mask>:
  20:	40 0f b6 c7          	movzbl %dil,%eax
  24:	48 83 e0 0f          	and    $0xf,%rax
  28:	48 f7 d0             	not    %rax
  2b:	0f b6 c0             	movzbl %al,%eax
  2e:	48 81 c8 f0 00 00 00 	or     $0xf0,%rax
  35:	c3                   	ret
	...

0000000000000040 <low_shift>:
  40:	b9 03 00 00 00       	mov    $0x3,%ecx
  45:	48 d3 e7             	shl    %cl,%rdi
  48:	48 83 e7 f8          	and    $0xfffffffffffffff8,%rdi
  4c:	48 89 f8             	mov    %rdi,%rax
  4f:	c3                   	ret

0000000000000050 <sign_shift>:
  50:	48 0f be c7          	movsbq %dil,%rax
  54:	48 83 c8 80          	or     $0xffffffffffffff80,%rax
  58:	b9 03 00 00 00       	mov    $0x3,%ecx
  5d:	48 d3 f8             	sar    %cl,%rax
  60:	48 83 c8 e0          	or     $0xffffffffffffffe0,%rax
  64:	c3                   	ret
