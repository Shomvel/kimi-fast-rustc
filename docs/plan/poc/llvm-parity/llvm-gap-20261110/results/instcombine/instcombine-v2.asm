
instcombine-v2/objects/libkernels.kernels.95fd440ee8c76f94-cgu.0.rcgu.o:     file format elf64-x86-64


Disassembly of section .text:

0000000000000000 <nested_add>:
   0:	48 83 c7 2a          	add    $0x2a,%rdi
   4:	48 89 f8             	mov    %rdi,%rax
   7:	c3                   	ret
	...

0000000000000010 <nested_and>:
  10:	48 81 e7 0f 00 0f 00 	and    $0xf000f,%rdi
  17:	48 89 f8             	mov    %rdi,%rax
  1a:	c3                   	ret
  1b:	00 00                	add    %al,(%rax)
  1d:	00 00                	add    %al,(%rax)
	...

0000000000000020 <nested_mul>:
  20:	48 6b ff 15          	imul   $0x15,%rdi,%rdi
  24:	48 89 f8             	mov    %rdi,%rax
  27:	c3                   	ret
	...

0000000000000030 <nested_or>:
  30:	48 81 cf 00 ff 00 00 	or     $0xff00,%rdi
  37:	48 89 f8             	mov    %rdi,%rax
  3a:	c3                   	ret
  3b:	00 00                	add    %al,(%rax)
  3d:	00 00                	add    %al,(%rax)
	...

0000000000000040 <nested_xor>:
  40:	48 81 f7 00 ff 00 00 	xor    $0xff00,%rdi
  47:	48 89 f8             	mov    %rdi,%rax
  4a:	c3                   	ret
  4b:	00 00                	add    %al,(%rax)
  4d:	00 00                	add    %al,(%rax)
	...

0000000000000050 <reversed_add>:
  50:	48 01 f7             	add    %rsi,%rdi
  53:	48 89 f8             	mov    %rdi,%rax
  56:	48 31 f8             	xor    %rdi,%rax
  59:	c3                   	ret
