# 已验证产物的汇编摘录

由 Apple LLVM objdump 15 反汇编既有 ELF；生成这些 ELF 的编译器仍为 Rust 1.94.1 / LLVM 21.1.8 或本轮 KFRC。地址为各目标文件 section 内偏移。完整汇编由 analyze.py 生成，存于本机 artifact-review/asm。

## llvm: <semver::identifier::Identifier>::ptr_eq

对象 `lib.semver.fd940cb7d3d8ab00-cgu.03.rcgu.o`，SHA-256 `439e001f898394164a10067c591df7bdf033b070bd23ad0f81614f4ededafab9`，函数 10 B。

```asm
       0: 48 8b 07                     	mov	rax, qword ptr [rdi]
       3: 48 3b 06                     	cmp	rax, qword ptr [rsi]
       6: 0f 94 c0                     	sete	al
       9: c3                           	ret

```

## data: <semver::identifier::Identifier>::ptr_eq

对象 `lib.semver.fd940cb7d3d8ab00-cgu.03.rcgu.o`，SHA-256 `7fdb0504d4dbd477e658d3d9b0da6439ae244838c9616b48c33a539a43430dd3`，函数 30 B。

```asm
     290: 48 8b 07                     	mov	rax, qword ptr [rdi]
     293: 48 8b 0e                     	mov	rcx, qword ptr [rsi]
     296: 48 39 c8                     	cmp	rax, rcx
     299: 0f 94 c0                     	sete	al
     29c: 0f b6 c0                     	movzx	eax, al
     29f: 48 85 c0                     	test	rax, rax
     2a2: 74 07                        	je	0x2ab <<semver::identifier::Identifier>::ptr_eq+0x1b>
     2a4: b8 01 00 00 00               	mov	eax, 1
     2a9: eb 02                        	jmp	0x2ad <<semver::identifier::Identifier>::ptr_eq+0x1d>
     2ab: 31 c0                        	xor	eax, eax
     2ad: c3                           	ret
```

## llvm: <semver::identifier::Identifier>::is_empty

对象 `lib.semver.fd940cb7d3d8ab00-cgu.03.rcgu.o`，SHA-256 `439e001f898394164a10067c591df7bdf033b070bd23ad0f81614f4ededafab9`，函数 8 B。

```asm
       0: 48 83 3f ff                  	cmp	qword ptr [rdi], -1
       4: 0f 94 c0                     	sete	al
       7: c3                           	ret

```

## data: <semver::identifier::Identifier>::is_empty

对象 `lib.semver.fd940cb7d3d8ab00-cgu.03.rcgu.o`，SHA-256 `7fdb0504d4dbd477e658d3d9b0da6439ae244838c9616b48c33a539a43430dd3`，函数 58 B。

```asm
     2b0: 55                           	push	rbp
     2b1: 48 89 e5                     	mov	rbp, rsp
     2b4: 48 83 ec 10                  	sub	rsp, 16
     2b8: 48 8d 45 f8                  	lea	rax, [rbp - 8]
     2bc: 48 c7 00 ff ff ff ff         	mov	qword ptr [rax], -1
     2c3: 48 8b 0f                     	mov	rcx, qword ptr [rdi]
     2c6: 48 8b 10                     	mov	rdx, qword ptr [rax]
     2c9: 48 39 d1                     	cmp	rcx, rdx
     2cc: 0f 94 c1                     	sete	cl
     2cf: 0f b6 c9                     	movzx	ecx, cl
     2d2: 48 85 c9                     	test	rcx, rcx
     2d5: 74 07                        	je	0x2de <<semver::identifier::Identifier>::is_empty+0x2e>
     2d7: b9 01 00 00 00               	mov	ecx, 1
     2dc: eb 02                        	jmp	0x2e0 <<semver::identifier::Identifier>::is_empty+0x30>
     2de: 31 c9                        	xor	ecx, ecx
     2e0: 31 d2                        	xor	edx, edx
     2e2: 48 89 10                     	mov	qword ptr [rax], rdx
     2e5: 48 89 c8                     	mov	rax, rcx
     2e8: c9                           	leave
     2e9: c3                           	ret
```

## llvm: <humantime::duration::Parser>::parse_unit

对象 `lib.humantime.7c8d5c717cf60a7d-cgu.00.rcgu.o`，SHA-256 `d285c592795918d9f19702c450e9d148fc911ed141a25de091f355a76cdb9f8e`，函数 1093 B。

```asm
       0: 55                           	push	rbp
       1: 41 57                        	push	r15
       3: 41 56                        	push	r14
       5: 41 55                        	push	r13
       7: 41 54                        	push	r12
       9: 53                           	push	rbx
       a: 48 83 ec 18                  	sub	rsp, 24
       e: 4d 89 cc                     	mov	r12, r9
      11: 48 89 d3                     	mov	rbx, rdx
      14: 49 89 f6                     	mov	r14, rsi
      17: 48 8b 44 24 50               	mov	rax, qword ptr [rsp + 80]
      1c: 48 89 c5                     	mov	rbp, rax
      1f: 4c 29 cd                     	sub	rbp, r9
      22: 72 26                        	jb	0x4a <<humantime::duration::Parser>::parse_unit+0x4a>
      24: 4d 85 e4                     	test	r12, r12
      27: 74 10                        	je	0x39 <<humantime::duration::Parser>::parse_unit+0x39>
      29: 49 39 dc                     	cmp	r12, rbx
      2c: 73 09                        	jae	0x37 <<humantime::duration::Parser>::parse_unit+0x37>
      2e: 43 80 3c 26 c0               	cmp	byte ptr [r14 + r12], -64
      33: 7d 04                        	jge	0x39 <<humantime::duration::Parser>::parse_unit+0x39>
      35: eb 13                        	jmp	0x4a <<humantime::duration::Parser>::parse_unit+0x4a>
      37: 75 11                        	jne	0x4a <<humantime::duration::Parser>::parse_unit+0x4a>
      39: 48 85 c0                     	test	rax, rax
      3c: 74 27                        	je	0x65 <<humantime::duration::Parser>::parse_unit+0x65>
      3e: 48 39 d8                     	cmp	rax, rbx
      41: 73 20                        	jae	0x63 <<humantime::duration::Parser>::parse_unit+0x63>
      43: 41 80 3c 06 bf               	cmp	byte ptr [r14 + rax], -65
      48: 7f 1b                        	jg	0x65 <<humantime::duration::Parser>::parse_unit+0x65>
      4a: 4c 8d 05 00 00 00 00         	lea	r8, [rip]               # 0x51 <<humantime::duration::Parser>::parse_unit+0x51>
		000000000000004d:  R_X86_64_PC32	.data.rel.ro..Lanon.1acb569f15bb15275ec0644dbf68a144.4-0x4
      51: 4c 89 f7                     	mov	rdi, r14
      54: 48 89 de                     	mov	rsi, rbx
      57: 4c 89 e2                     	mov	rdx, r12
      5a: 48 89 c1                     	mov	rcx, rax
      5d: ff 15 00 00 00 00            	call	qword ptr [rip]         # 0x63 <<humantime::duration::Parser>::parse_unit+0x63>
		000000000000005f:  R_X86_64_GOTPCREL	core::str::slice_error_fail-0x4
      63: 75 e5                        	jne	0x4a <<humantime::duration::Parser>::parse_unit+0x4a>
      65: 4c 89 44 24 10               	mov	qword ptr [rsp + 16], r8
      6a: 49 89 cd                     	mov	r13, rcx
      6d: 48 89 7c 24 08               	mov	qword ptr [rsp + 8], rdi
      72: 4f 8d 3c 26                  	lea	r15, [r14 + r12]
      76: 4c 89 ff                     	mov	rdi, r15
      79: 48 89 ee                     	mov	rsi, rbp
      7c: ff 15 00 00 00 00            	call	qword ptr [rip]         # 0x82 <<humantime::duration::Parser>::parse_unit+0x82>
		000000000000007e:  R_X86_64_GOTPCREL	<humantime::duration::Unit as core::str::traits::FromStr>::from_str-0x4
      82: 44 0f b6 c0                  	movzx	r8d, al
      86: 48 8d 05 00 00 00 00         	lea	rax, [rip]              # 0x8d <<humantime::duration::Parser>::parse_unit+0x8d>
		0000000000000089:  R_X86_64_PC32	.rodata._RNvMs1_NtCsaGZho4zWNqb_9humantime8durationNtB5_6Parser10parse_unit-0x4
      8d: 4a 63 0c 80                  	movsxd	rcx, dword ptr [rax + 4*r8]
      91: 48 01 c1                     	add	rcx, rax
      94: ff e1                        	jmp	rcx
      96: 4c 89 e8                     	mov	rax, r13
      99: 4d 89 c5                     	mov	r13, r8
      9c: 48 8b 74 24 58               	mov	rsi, qword ptr [rsp + 88]
      a1: 8b 7e 08                     	mov	edi, dword ptr [rsi + 8]
      a4: 48 01 c7                     	add	rdi, rax
      a7: 0f 83 ec 00 00 00            	jae	0x199 <<humantime::duration::Parser>::parse_unit+0x199>
      ad: e9 1f 01 00 00               	jmp	0x1d1 <<humantime::duration::Parser>::parse_unit+0x1d1>
      b2: b9 3c 00 00 00               	mov	ecx, 60
      b7: 4c 89 e8                     	mov	rax, r13
      ba: 48 f7 e1                     	mul	rcx
      bd: 0f 81 c4 00 00 00            	jno	0x187 <<humantime::duration::Parser>::parse_unit+0x187>
      c3: e9 ac 02 00 00               	jmp	0x374 <<humantime::duration::Parser>::parse_unit+0x374>
      c8: b9 e0 87 e1 01               	mov	ecx, 31557600
      cd: 4c 89 e8                     	mov	rax, r13
      d0: 48 f7 e1                     	mul	rcx
      d3: 0f 81 ae 00 00 00            	jno	0x187 <<humantime::duration::Parser>::parse_unit+0x187>
      d9: e9 96 02 00 00               	jmp	0x374 <<humantime::duration::Parser>::parse_unit+0x374>
      de: b9 40 42 0f 00               	mov	ecx, 1000000
      e3: 4c 89 e8                     	mov	rax, r13
      e6: 48 f7 e1                     	mul	rcx
      e9: 71 2c                        	jno	0x117 <<humantime::duration::Parser>::parse_unit+0x117>
      eb: e9 84 02 00 00               	jmp	0x374 <<humantime::duration::Parser>::parse_unit+0x374>
      f0: b9 80 3a 09 00               	mov	ecx, 604800
      f5: 4c 89 e8                     	mov	rax, r13
      f8: 48 f7 e1                     	mul	rcx
      fb: 0f 81 86 00 00 00            	jno	0x187 <<humantime::duration::Parser>::parse_unit+0x187>
     101: e9 6e 02 00 00               	jmp	0x374 <<humantime::duration::Parser>::parse_unit+0x374>
     106: b9 e8 03 00 00               	mov	ecx, 1000
     10b: 4c 89 e8                     	mov	rax, r13
     10e: 48 f7 e1                     	mul	rcx
     111: 0f 80 5d 02 00 00            	jo	0x374 <<humantime::duration::Parser>::parse_unit+0x374>
     117: 45 31 ed                     	xor	r13d, r13d
     11a: 48 8b 74 24 58               	mov	rsi, qword ptr [rsp + 88]
     11f: 8b 7e 08                     	mov	edi, dword ptr [rsi + 8]
     122: 48 01 c7                     	add	rdi, rax
     125: 73 72                        	jae	0x199 <<humantime::duration::Parser>::parse_unit+0x199>
     127: e9 a5 00 00 00               	jmp	0x1d1 <<humantime::duration::Parser>::parse_unit+0x1d1>
     12c: b9 10 0e 00 00               	mov	ecx, 3600
     131: 4c 89 e8                     	mov	rax, r13
     134: 48 f7 e1                     	mul	rcx
     137: 71 4e                        	jno	0x187 <<humantime::duration::Parser>::parse_unit+0x187>
     139: e9 36 02 00 00               	jmp	0x374 <<humantime::duration::Parser>::parse_unit+0x374>
     13e: b9 80 51 01 00               	mov	ecx, 86400
     143: 4c 89 e8                     	mov	rax, r13
     146: 48 f7 e1                     	mul	rcx
     149: 71 3c                        	jno	0x187 <<humantime::duration::Parser>::parse_unit+0x187>
     14b: e9 24 02 00 00               	jmp	0x374 <<humantime::duration::Parser>::parse_unit+0x374>
     150: 4d 85 e4                     	test	r12, r12
     153: 48 8b 4c 24 50               	mov	rcx, qword ptr [rsp + 80]
```

## data: <humantime::duration::Parser>::parse_unit

对象 `lib.humantime.7c8d5c717cf60a7d-cgu.00.rcgu.o`，SHA-256 `fe04aba3627ccd4d1f9afa4ce8fa85ba401bd04650338e224ed0bfb3cb9f5f16`，函数 23799 B。

```asm
    22cf: 48 69 f6 e0 87 e1 01         	imul	rsi, rsi, 31557600
    22d6: 48 89 f0                     	mov	rax, rsi
    22d9: be 01 00 00 00               	mov	esi, 1
    22de: 48 8d 8d 90 f4 ff ff         	lea	rcx, [rbp - 2928]
    22e5: 48 ba 03 00 00 00 00 00 00 80	movabs	rdx, -9223372036854775805
    22ef: 48 89 11                     	mov	qword ptr [rcx], rdx
    22f2: 4c 8d ad 20 f1 ff ff         	lea	r13, [rbp - 3808]
    22f9: 4c 89 ef                     	mov	rdi, r13
    22fc: 48 89 c2                     	mov	rdx, rax
    22ff: e8 00 00 00 00               	call	0x2304 <<humantime::duration::Parser>::parse_unit+0x2a4>
		0000000000002300:  R_X86_64_PLT32	<core::option::Option<u64>>::ok_or::<humantime::duration::Error>-0x4
    2304: 49 8b 45 00                  	mov	rax, qword ptr [r13]
    2308: 48 b9 05 00 00 00 00 00 00 80	movabs	rcx, -9223372036854775803
    2312: 48 39 c8                     	cmp	rax, rcx
    2315: 74 02                        	je	0x2319 <<humantime::duration::Parser>::parse_unit+0x2b9>
    2317: eb 04                        	jmp	0x231d <<humantime::duration::Parser>::parse_unit+0x2bd>
    2319: 49 8b 4d 08                  	mov	rcx, qword ptr [r13 + 8]
    231d: 48 ba 05 00 00 00 00 00 00 80	movabs	rdx, -9223372036854775803
    2327: 48 39 d0                     	cmp	rax, rdx
    232a: 75 09                        	jne	0x2335 <<humantime::duration::Parser>::parse_unit+0x2d5>
    232c: 4c 89 65 f8                  	mov	qword ptr [rbp - 8], r12
    2330: e9 92 00 00 00               	jmp	0x23c7 <<humantime::duration::Parser>::parse_unit+0x367>
    2335: 49 8b 55 00                  	mov	rdx, qword ptr [r13]
    2339: 48 be 00 00 00 00 00 00 00 80	movabs	rsi, -9223372036854775808
    2343: 48 39 f2                     	cmp	rdx, rsi
    2346: 74 02                        	je	0x234a <<humantime::duration::Parser>::parse_unit+0x2ea>
    2348: eb 04                        	jmp	0x234e <<humantime::duration::Parser>::parse_unit+0x2ee>
    234a: 49 8b 75 08                  	mov	rsi, qword ptr [r13 + 8]
    234e: 48 bf 01 00 00 00 00 00 00 80	movabs	rdi, -9223372036854775807
    2358: 48 39 fa                     	cmp	rdx, rdi
    235b: 74 02                        	je	0x235f <<humantime::duration::Parser>::parse_unit+0x2ff>
    235d: eb 04                        	jmp	0x2363 <<humantime::duration::Parser>::parse_unit+0x303>
    235f: 49 8b 7d 08                  	mov	rdi, qword ptr [r13 + 8]
    2363: 49 89 d0                     	mov	r8, rdx
    2366: 49 b9 00 00 00 00 00 00 00 80	movabs	r9, -9223372036854775808
    2370: 4d 29 c8                     	sub	r8, r9
    2373: 49 83 f8 04                  	cmp	r8, 4
    2377: 77 02                        	ja	0x237b <<humantime::duration::Parser>::parse_unit+0x31b>
    2379: eb 18                        	jmp	0x2393 <<humantime::duration::Parser>::parse_unit+0x333>
    237b: 4d 8b 45 18                  	mov	r8, qword ptr [r13 + 24]
    237f: 4d 8b 4d 20                  	mov	r9, qword ptr [r13 + 32]
    2383: 4d 8b 55 08                  	mov	r10, qword ptr [r13 + 8]
    2387: 4d 8b 5d 00                  	mov	r11, qword ptr [r13]
    238b: 4d 8b 75 10                  	mov	r14, qword ptr [r13 + 16]
    238f: 4d 8b 6d 28                  	mov	r13, qword ptr [r13 + 40]
    2393: 49 bf 00 00 00 00 00 00 00 80	movabs	r15, -9223372036854775808
    239d: 4c 29 fa                     	sub	rdx, r15
    23a0: 48 83 fa 04                  	cmp	rdx, 4
    23a4: 41 0f 96 c7                  	setbe	r15b
    23a8: 45 0f b6 ff                  	movzx	r15d, r15b
    23ac: 48 83 c2 00                  	add	rdx, 0
    23b0: 49 f7 df                     	neg	r15
    23b3: 4c 89 65 f8                  	mov	qword ptr [rbp - 8], r12
    23b7: 4d 89 fc                     	mov	r12, r15
    23ba: 49 f7 d4                     	not	r12
    23bd: 49 21 d7                     	and	r15, rdx
    23c0: 49 83 e4 02                  	and	r12, 2
    23c4: 4d 09 e7                     	or	r15, r12
    23c7: 48 8d 55 a0                  	lea	rdx, [rbp - 96]
    23cb: 49 bc 05 00 00 00 00 00 00 80	movabs	r12, -9223372036854775803
    23d5: 4c 39 e0                     	cmp	rax, r12
    23d8: 74 02                        	je	0x23dc <<humantime::duration::Parser>::parse_unit+0x37c>
    23da: eb 11                        	jmp	0x23ed <<humantime::duration::Parser>::parse_unit+0x38d>
    23dc: 48 89 4a 08                  	mov	qword ptr [rdx + 8], rcx
    23e0: 48 b9 05 00 00 00 00 00 00 80	movabs	rcx, -9223372036854775803
    23ea: 48 89 0a                     	mov	qword ptr [rdx], rcx
    23ed: 48 b9 05 00 00 00 00 00 00 80	movabs	rcx, -9223372036854775803
    23f7: 48 39 c8                     	cmp	rax, rcx
    23fa: 75 02                        	jne	0x23fe <<humantime::duration::Parser>::parse_unit+0x39e>
    23fc: eb 7a                        	jmp	0x2478 <<humantime::duration::Parser>::parse_unit+0x418>
    23fe: 4d 85 ff                     	test	r15, r15
    2401: 74 02                        	je	0x2405 <<humantime::duration::Parser>::parse_unit+0x3a5>
    2403: eb 11                        	jmp	0x2416 <<humantime::duration::Parser>::parse_unit+0x3b6>
    2405: 48 89 72 08                  	mov	qword ptr [rdx + 8], rsi
    2409: 48 b8 00 00 00 00 00 00 00 80	movabs	rax, -9223372036854775808
    2413: 48 89 02                     	mov	qword ptr [rdx], rax
    2416: 49 83 ff 01                  	cmp	r15, 1
    241a: 74 02                        	je	0x241e <<humantime::duration::Parser>::parse_unit+0x3be>
    241c: eb 11                        	jmp	0x242f <<humantime::duration::Parser>::parse_unit+0x3cf>
    241e: 48 89 7a 08                  	mov	qword ptr [rdx + 8], rdi
    2422: 48 b8 01 00 00 00 00 00 00 80	movabs	rax, -9223372036854775807
    242c: 48 89 02                     	mov	qword ptr [rdx], rax
    242f: 49 83 ff 02                  	cmp	r15, 2
    2433: 74 02                        	je	0x2437 <<humantime::duration::Parser>::parse_unit+0x3d7>
    2435: eb 17                        	jmp	0x244e <<humantime::duration::Parser>::parse_unit+0x3ee>
    2437: 4c 89 42 18                  	mov	qword ptr [rdx + 24], r8
    243b: 4c 89 4a 20                  	mov	qword ptr [rdx + 32], r9
    243f: 4c 89 52 08                  	mov	qword ptr [rdx + 8], r10
    2443: 4c 89 1a                     	mov	qword ptr [rdx], r11
    2446: 4c 89 72 10                  	mov	qword ptr [rdx + 16], r14
    244a: 4c 89 6a 28                  	mov	qword ptr [rdx + 40], r13
    244e: 49 83 ff 03                  	cmp	r15, 3
    2452: 74 02                        	je	0x2456 <<humantime::duration::Parser>::parse_unit+0x3f6>
    2454: eb 0d                        	jmp	0x2463 <<humantime::duration::Parser>::parse_unit+0x403>
    2456: 48 b8 03 00 00 00 00 00 00 80	movabs	rax, -9223372036854775805
    2460: 48 89 02                     	mov	qword ptr [rdx], rax
    2463: 49 83 ff 04                  	cmp	r15, 4
    2467: 74 02                        	je	0x246b <<humantime::duration::Parser>::parse_unit+0x40b>
    2469: eb 0d                        	jmp	0x2478 <<humantime::duration::Parser>::parse_unit+0x418>
    246b: 48 b8 04 00 00 00 00 00 00 80	movabs	rax, -9223372036854775804
    2475: 48 89 02                     	mov	qword ptr [rdx], rax
    2478: 4c 8d a5 f0 f0 ff ff         	lea	r12, [rbp - 3856]
    247f: 4c 89 e7                     	mov	rdi, r12
    2482: 48 89 d6                     	mov	rsi, rdx
    2485: e8 00 00 00 00               	call	0x248a <<humantime::duration::Parser>::parse_unit+0x42a>
		0000000000002486:  R_X86_64_PLT32	<core::result::Result<u64, humantime::duration::Error> as core::ops::try_trait::Try>::branch-0x4
    248a: 49 8b 04 24                  	mov	rax, qword ptr [r12]
    248e: 48 b9 05 00 00 00 00 00 00 80	movabs	rcx, -9223372036854775803
    2498: 48 39 c8                     	cmp	rax, rcx
    249b: 74 02                        	je	0x249f <<humantime::duration::Parser>::parse_unit+0x43f>
```

## llvm: <humantime::duration::Unit as core::str::traits::FromStr>::from_str

对象 `lib.humantime.7c8d5c717cf60a7d-cgu.00.rcgu.o`，SHA-256 `d285c592795918d9f19702c450e9d148fc911ed141a25de091f355a76cdb9f8e`，函数 675 B。

```asm
       0: b0 0a                        	mov	al, 10
       2: 48 ff ce                     	dec	rsi
       5: 48 83 fe 06                  	cmp	rsi, 6
       9: 0f 87 90 02 00 00            	ja	0x29f <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0x29f>
       f: 48 8d 0d 00 00 00 00         	lea	rcx, [rip]              # 0x16 <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0x16>
		0000000000000012:  R_X86_64_PC32	.rodata._RNvXs2_NtCsaGZho4zWNqb_9humantime8durationNtB5_4UnitNtNtNtCshX3mvxHm3fC_4core3str6traits7FromStr8from_str-0x4
      16: 48 63 14 b1                  	movsxd	rdx, dword ptr [rcx + 4*rsi]
      1a: 48 01 ca                     	add	rdx, rcx
      1d: ff e2                        	jmp	rdx
      1f: 0f b6 07                     	movzx	eax, byte ptr [rdi]
      22: 8d 48 b3                     	lea	ecx, [rax - 77]
      25: 83 f9 2a                     	cmp	ecx, 42
      28: 77 10                        	ja	0x3a <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0x3a>
      2a: 48 8d 15 00 00 00 00         	lea	rdx, [rip]              # 0x31 <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0x31>
		000000000000002d:  R_X86_64_PC32	.rodata._RNvXs2_NtCsaGZho4zWNqb_9humantime8durationNtB5_4UnitNtNtNtCshX3mvxHm3fC_4core3str6traits7FromStr8from_str+0x18
      31: 48 63 0c 8a                  	movsxd	rcx, dword ptr [rdx + 4*rcx]
      35: 48 01 d1                     	add	rcx, rdx
      38: ff e1                        	jmp	rcx
      3a: 3c 79                        	cmp	al, 121
      3c: 0f 94 c1                     	sete	cl
      3f: b0 0a                        	mov	al, 10
      41: 28 c8                        	sub	al, cl
      43: c3                           	ret
      44: b9 6e 61 6e 6f               	mov	ecx, 1869504878
      49: 33 0f                        	xor	ecx, dword ptr [rdi]
      4b: 0f b6 57 04                  	movzx	edx, byte ptr [rdi + 4]
      4f: 83 f2 73                     	xor	edx, 115
      52: 09 ca                        	or	edx, ecx
      54: 0f 84 b4 01 00 00            	je	0x20e <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0x20e>
      5a: b9 68 6f 75 72               	mov	ecx, 1920298856
      5f: 33 0f                        	xor	ecx, dword ptr [rdi]
      61: 0f b6 57 04                  	movzx	edx, byte ptr [rdi + 4]
      65: 83 f2 73                     	xor	edx, 115
      68: 09 ca                        	or	edx, ecx
      6a: 0f 84 2a 02 00 00            	je	0x29a <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0x29a>
      70: b9 77 65 65 6b               	mov	ecx, 1801807223
      75: 33 0f                        	xor	ecx, dword ptr [rdi]
      77: 0f b6 57 04                  	movzx	edx, byte ptr [rdi + 4]
      7b: 83 f2 73                     	xor	edx, 115
      7e: 09 ca                        	or	edx, ecx
      80: 0f 84 17 02 00 00            	je	0x29d <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0x29d>
      86: b9 6d 6f 6e 74               	mov	ecx, 1953394541
      8b: 33 0f                        	xor	ecx, dword ptr [rdi]
      8d: 0f b6 57 04                  	movzx	edx, byte ptr [rdi + 4]
      91: 83 f2 68                     	xor	edx, 104
      94: 09 ca                        	or	edx, ecx
      96: 0f 84 c9 01 00 00            	je	0x265 <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0x265>
      9c: b9 79 65 61 72               	mov	ecx, 1918985593
      a1: 33 0f                        	xor	ecx, dword ptr [rdi]
      a3: 0f b6 57 04                  	movzx	edx, byte ptr [rdi + 4]
      a7: 83 f2 73                     	xor	edx, 115
      aa: 09 ca                        	or	edx, ecx
      ac: 0f 85 ed 01 00 00            	jne	0x29f <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0x29f>
      b2: b0 09                        	mov	al, 9
      b4: c3                           	ret
      b5: 0f b7 07                     	movzx	eax, word ptr [rdi]
      b8: 35 c2 b5 00 00               	xor	eax, 46530
      bd: 0f b6 4f 02                  	movzx	ecx, byte ptr [rdi + 2]
      c1: 83 f1 73                     	xor	ecx, 115
      c4: 66 09 c1                     	or	cx, ax
      c7: 0f 84 c7 01 00 00            	je	0x294 <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0x294>
      cd: 0f b7 07                     	movzx	eax, word ptr [rdi]
      d0: 35 73 65 00 00               	xor	eax, 25971
      d5: 0f b6 4f 02                  	movzx	ecx, byte ptr [rdi + 2]
      d9: 83 f1 63                     	xor	ecx, 99
```

## data: <humantime::duration::Unit as core::str::traits::FromStr>::from_str

对象 `lib.humantime.7c8d5c717cf60a7d-cgu.00.rcgu.o`，SHA-256 `fe04aba3627ccd4d1f9afa4ce8fa85ba401bd04650338e224ed0bfb3cb9f5f16`，函数 1294 B。

```asm
    a290: 55                           	push	rbp
    a291: 48 89 e5                     	mov	rbp, rsp
    a294: 48 83 ec 10                  	sub	rsp, 16
    a298: 53                           	push	rbx
    a299: 41 54                        	push	r12
    a29b: 48 89 fb                     	mov	rbx, rdi
    a29e: 49 89 f4                     	mov	r12, rsi
    a2a1: 48 8d 15 00 00 00 00         	lea	rdx, [rip]              # 0xa2a8 <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0x18>
		000000000000a2a4:  R_X86_64_PC32	.Lkfrc__RNvXs2_NtCsaGZho4zWNqb_9humantime8durationNtB5_4UnitNtNtNtCshX3mvxHm3fC_4core3str6traits7FromStr8from_str_alloc71-0x4
    a2a8: 48 89 df                     	mov	rdi, rbx
    a2ab: 4c 89 e6                     	mov	rsi, r12
    a2ae: b9 05 00 00 00               	mov	ecx, 5
    a2b3: e8 00 00 00 00               	call	0xa2b8 <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0x28>
		000000000000a2b4:  R_X86_64_PLT32	<[u8] as core::slice::cmp::SlicePartialEq<u8>>::equal-0x4
    a2b8: 84 c0                        	test	al, al
    a2ba: 74 02                        	je	0xa2be <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0x2e>
    a2bc: eb 38                        	jmp	0xa2f6 <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0x66>
    a2be: 48 8d 15 00 00 00 00         	lea	rdx, [rip]              # 0xa2c5 <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0x35>
		000000000000a2c1:  R_X86_64_PC32	.Lkfrc__RNvXs2_NtCsaGZho4zWNqb_9humantime8durationNtB5_4UnitNtNtNtCshX3mvxHm3fC_4core3str6traits7FromStr8from_str_alloc72-0x4
    a2c5: 48 89 df                     	mov	rdi, rbx
    a2c8: 4c 89 e6                     	mov	rsi, r12
    a2cb: b9 04 00 00 00               	mov	ecx, 4
    a2d0: e8 00 00 00 00               	call	0xa2d5 <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0x45>
		000000000000a2d1:  R_X86_64_PLT32	<[u8] as core::slice::cmp::SlicePartialEq<u8>>::equal-0x4
    a2d5: 84 c0                        	test	al, al
    a2d7: 74 02                        	je	0xa2db <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0x4b>
    a2d9: eb 1b                        	jmp	0xa2f6 <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0x66>
    a2db: 48 8d 15 00 00 00 00         	lea	rdx, [rip]              # 0xa2e2 <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0x52>
		000000000000a2de:  R_X86_64_PC32	.Lkfrc__RNvXs2_NtCsaGZho4zWNqb_9humantime8durationNtB5_4UnitNtNtNtCshX3mvxHm3fC_4core3str6traits7FromStr8from_str_alloc73-0x4
    a2e2: 48 89 df                     	mov	rdi, rbx
    a2e5: 4c 89 e6                     	mov	rsi, r12
    a2e8: b9 02 00 00 00               	mov	ecx, 2
    a2ed: e8 00 00 00 00               	call	0xa2f2 <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0x62>
		000000000000a2ee:  R_X86_64_PLT32	<[u8] as core::slice::cmp::SlicePartialEq<u8>>::equal-0x4
    a2f2: 84 c0                        	test	al, al
    a2f4: 74 0c                        	je	0xa302 <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0x72>
    a2f6: 48 8d 45 ff                  	lea	rax, [rbp - 1]
    a2fa: c6 00 00                     	mov	byte ptr [rax], 0
    a2fd: e9 8f 04 00 00               	jmp	0xa791 <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0x501>
    a302: 48 8d 15 00 00 00 00         	lea	rdx, [rip]              # 0xa309 <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0x79>
		000000000000a305:  R_X86_64_PC32	.Lkfrc__RNvXs2_NtCsaGZho4zWNqb_9humantime8durationNtB5_4UnitNtNtNtCshX3mvxHm3fC_4core3str6traits7FromStr8from_str_alloc74-0x4
    a309: 48 89 df                     	mov	rdi, rbx
    a30c: 4c 89 e6                     	mov	rsi, r12
    a30f: b9 04 00 00 00               	mov	ecx, 4
    a314: e8 00 00 00 00               	call	0xa319 <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0x89>
		000000000000a315:  R_X86_64_PLT32	<[u8] as core::slice::cmp::SlicePartialEq<u8>>::equal-0x4
    a319: 84 c0                        	test	al, al
    a31b: 74 02                        	je	0xa31f <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0x8f>
    a31d: eb 38                        	jmp	0xa357 <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0xc7>
    a31f: 48 8d 15 00 00 00 00         	lea	rdx, [rip]              # 0xa326 <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0x96>
		000000000000a322:  R_X86_64_PC32	.Lkfrc__RNvXs2_NtCsaGZho4zWNqb_9humantime8durationNtB5_4UnitNtNtNtCshX3mvxHm3fC_4core3str6traits7FromStr8from_str_alloc75-0x4
    a326: 48 89 df                     	mov	rdi, rbx
    a329: 4c 89 e6                     	mov	rsi, r12
    a32c: b9 02 00 00 00               	mov	ecx, 2
    a331: e8 00 00 00 00               	call	0xa336 <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0xa6>
		000000000000a332:  R_X86_64_PLT32	<[u8] as core::slice::cmp::SlicePartialEq<u8>>::equal-0x4
    a336: 84 c0                        	test	al, al
    a338: 74 02                        	je	0xa33c <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0xac>
    a33a: eb 1b                        	jmp	0xa357 <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0xc7>
    a33c: 48 8d 15 00 00 00 00         	lea	rdx, [rip]              # 0xa343 <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0xb3>
		000000000000a33f:  R_X86_64_PC32	.Lkfrc__RNvXs2_NtCsaGZho4zWNqb_9humantime8durationNtB5_4UnitNtNtNtCshX3mvxHm3fC_4core3str6traits7FromStr8from_str_alloc76-0x4
    a343: 48 89 df                     	mov	rdi, rbx
    a346: 4c 89 e6                     	mov	rsi, r12
    a349: b9 03 00 00 00               	mov	ecx, 3
    a34e: e8 00 00 00 00               	call	0xa353 <<humantime::duration::Unit as core::str::traits::FromStr>::from_str+0xc3>
```
