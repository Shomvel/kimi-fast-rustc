
baseline/objects/libkernels.kernels.95fd440ee8c76f94-cgu.0.rcgu.o:     file format elf64-x86-64


Disassembly of section .text:

0000000000000000 <changing_loop>:
   0:	b8 00 00 00 00       	mov    $0x0,%eax
   5:	b9 2a 00 00 00       	mov    $0x2a,%ecx
   a:	48 39 f8             	cmp    %rdi,%rax
   d:	73 1e                	jae    2d <changing_loop+0x2d>
   f:	48 89 c1             	mov    %rax,%rcx
  12:	48 83 e1 01          	and    $0x1,%rcx
  16:	48 85 c9             	test   %rcx,%rcx
  19:	75 07                	jne    22 <changing_loop+0x22>
  1b:	b9 2a 00 00 00       	mov    $0x2a,%ecx
  20:	eb 05                	jmp    27 <changing_loop+0x27>
  22:	b9 2a 00 00 00       	mov    $0x2a,%ecx
  27:	48 83 c0 01          	add    $0x1,%rax
  2b:	eb dd                	jmp    a <changing_loop+0xa>
  2d:	48 01 c1             	add    %rax,%rcx
  30:	48 89 c8             	mov    %rcx,%rax
  33:	c3                   	ret
	...

0000000000000040 <constant_branch>:
  40:	40 84 ff             	test   %dil,%dil
  43:	74 00                	je     45 <constant_branch+0x5>
  45:	b8 2a 00 00 00       	mov    $0x2a,%eax
  4a:	c3                   	ret
  4b:	00 00                	add    %al,(%rax)
  4d:	00 00                	add    %al,(%rax)
	...

0000000000000050 <constant_merge>:
  50:	40 84 ff             	test   %dil,%dil
  53:	74 07                	je     5c <constant_merge+0xc>
  55:	b8 07 00 00 00       	mov    $0x7,%eax
  5a:	eb 05                	jmp    61 <constant_merge+0x11>
  5c:	b8 07 00 00 00       	mov    $0x7,%eax
  61:	48 6b c0 06          	imul   $0x6,%rax,%rax
  65:	c3                   	ret
	...

0000000000000070 <dynamic_merge>:
  70:	40 84 ff             	test   %dil,%dil
  73:	74 08                	je     7d <dynamic_merge+0xd>
  75:	b8 07 00 00 00       	mov    $0x7,%eax
  7a:	48 89 c6             	mov    %rax,%rsi
  7d:	48 83 c6 01          	add    $0x1,%rsi
  81:	48 89 f0             	mov    %rsi,%rax
  84:	c3                   	ret
	...

0000000000000090 <narrow_merge>:
  90:	40 84 ff             	test   %dil,%dil
  93:	74 07                	je     9c <narrow_merge+0xc>
  95:	b8 ff 00 00 00       	mov    $0xff,%eax
  9a:	eb 05                	jmp    a1 <narrow_merge+0x11>
  9c:	b8 ff 00 00 00       	mov    $0xff,%eax
  a1:	48 83 c0 02          	add    $0x2,%rax
  a5:	0f b6 c0             	movzbl %al,%eax
  a8:	c3                   	ret
