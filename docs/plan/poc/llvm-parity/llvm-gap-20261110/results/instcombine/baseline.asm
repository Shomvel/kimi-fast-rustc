
baseline/objects/libkernels.kernels.95fd440ee8c76f94-cgu.0.rcgu.o:     file format elf64-x86-64


Disassembly of section .text:

0000000000000000 <nested_add>:
   0:	48 83 c7 0d          	add    $0xd,%rdi
   4:	48 83 c7 1d          	add    $0x1d,%rdi
   8:	48 89 f8             	mov    %rdi,%rax
   b:	c3                   	ret
   c:	00 00                	add    %al,(%rax)
	...

0000000000000010 <nested_and>:
  10:	48 81 e7 ff 00 ff 00 	and    $0xff00ff,%rdi
  17:	48 81 e7 0f 00 0f 00 	and    $0xf000f,%rdi
  1e:	48 89 f8             	mov    %rdi,%rax
  21:	c3                   	ret
	...

0000000000000030 <nested_mul>:
  30:	48 6b ff 03          	imul   $0x3,%rdi,%rdi
  34:	48 6b ff 07          	imul   $0x7,%rdi,%rdi
  38:	48 89 f8             	mov    %rdi,%rax
  3b:	c3                   	ret
  3c:	00 00                	add    %al,(%rax)
	...

0000000000000040 <nested_or>:
  40:	48 81 cf 00 55 00 00 	or     $0x5500,%rdi
  47:	48 81 cf 00 aa 00 00 	or     $0xaa00,%rdi
  4e:	48 89 f8             	mov    %rdi,%rax
  51:	c3                   	ret
	...

0000000000000060 <nested_xor>:
  60:	48 81 f7 00 55 00 00 	xor    $0x5500,%rdi
  67:	48 81 f7 00 aa 00 00 	xor    $0xaa00,%rdi
  6e:	48 89 f8             	mov    %rdi,%rax
  71:	c3                   	ret
	...

0000000000000080 <reversed_add>:
  80:	48 89 f8             	mov    %rdi,%rax
  83:	48 01 f0             	add    %rsi,%rax
  86:	48 01 fe             	add    %rdi,%rsi
  89:	48 31 f0             	xor    %rsi,%rax
  8c:	c3                   	ret
