
sccp/objects/libkernels.kernels.95fd440ee8c76f94-cgu.0.rcgu.o:     file format elf64-x86-64


Disassembly of section .text:

0000000000000000 <changing_loop>:
   0:	b8 00 00 00 00       	mov    $0x0,%eax
   5:	48 39 f8             	cmp    %rdi,%rax
   8:	73 12                	jae    1c <changing_loop+0x1c>
   a:	48 89 c1             	mov    %rax,%rcx
   d:	48 83 e1 01          	and    $0x1,%rcx
  11:	48 85 c9             	test   %rcx,%rcx
  14:	75 00                	jne    16 <changing_loop+0x16>
  16:	48 83 c0 01          	add    $0x1,%rax
  1a:	eb e9                	jmp    5 <changing_loop+0x5>
  1c:	48 83 c0 2a          	add    $0x2a,%rax
  20:	c3                   	ret
	...

0000000000000030 <constant_branch>:
  30:	40 84 ff             	test   %dil,%dil
  33:	74 00                	je     35 <constant_branch+0x5>
  35:	b8 2a 00 00 00       	mov    $0x2a,%eax
  3a:	c3                   	ret
  3b:	00 00                	add    %al,(%rax)
  3d:	00 00                	add    %al,(%rax)
	...

0000000000000040 <constant_merge>:
  40:	40 84 ff             	test   %dil,%dil
  43:	74 00                	je     45 <constant_merge+0x5>
  45:	b8 2a 00 00 00       	mov    $0x2a,%eax
  4a:	c3                   	ret
  4b:	00 00                	add    %al,(%rax)
  4d:	00 00                	add    %al,(%rax)
	...

0000000000000050 <dynamic_merge>:
  50:	40 84 ff             	test   %dil,%dil
  53:	74 08                	je     5d <dynamic_merge+0xd>
  55:	b8 07 00 00 00       	mov    $0x7,%eax
  5a:	48 89 c6             	mov    %rax,%rsi
  5d:	48 83 c6 01          	add    $0x1,%rsi
  61:	48 89 f0             	mov    %rsi,%rax
  64:	c3                   	ret
	...

0000000000000070 <narrow_merge>:
  70:	40 84 ff             	test   %dil,%dil
  73:	74 00                	je     75 <narrow_merge+0x5>
  75:	b8 01 00 00 00       	mov    $0x1,%eax
  7a:	c3                   	ret
