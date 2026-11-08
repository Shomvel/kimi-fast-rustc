# W05：`&mut Vec<u8>` 循环 push 另一个 `&[u8]`

检验 [内存分区](../../design/ir/memory-partitions.md) 的 `Unique`/`Frozen`、内联 `Reborrow`/`EndBorrow`、扩容调用上的分区合并，以及规则 3/4/5 是否自洽。命令：`RUSTC=<1.94.1> RUSTC_X86=<1.99.1> scripts/w05.sh <workdir>`。

## 1. 程序

[scripts/w05.rs](scripts/w05.rs)：`append`（逐个 `push`）和 `append_ext`（`extend_from_slice`），两者都是 `#[inline(never)]`。对照用的 [scripts/w05-contrast.rs](scripts/w05-contrast.rs) 包含两个函数：`append_raw` 的 `dst` 是 `*mut Vec<u8>`，没有 noalias；`append_my` 用的是自写的 `MyVec`，`push` 写法和 std 相同，扩容函数体在同一 crate，可以推断出 `captures(none)`。

```rust
pub fn append(dst: &mut Vec<u8>, src: &[u8]) { for &b in src { dst.push(b) } }
```

## 2. rustc 现在怎么做（实测）

**MIR**：`Vec::push` 和 `Iter::next` 在 MIR 层已内联，循环体只剩一次调用 `_24 = Vec::<u8>::push_mut(copy _1, move _5)`。`push_mut` 的源码先执行 `let len = self.len`，注释是 “Inform codegen that the length does not change across grow_one()”；`grow_one` 只标了 `#[inline(never)]`，没有 `#[cold]`。

**未优化 IR**（已过 MIR 优化）：

```text
   35 inst  alloca=2   memcpy=0   lifetime=3   assume=6   call=1   invoke=0   landingpad=0  load=4    store=4    append
   43 inst  alloca=2   memcpy=0   lifetime=5   assume=1   call=0   invoke=1   landingpad=1  load=6    store=5    push_mut
   34 inst  alloca=1   memcpy=0   lifetime=0   assume=6   call=0   invoke=0   landingpad=0  load=3    store=3    4next
TOTAL functions=7 inst=200 alloca=11 memcpy=3 lifetime=18 assume=17 call=6 invoke=1 landingpad=1 load=25 store=19
```

**O3 参数属性**（`compiler/rustc_ty_utils/src/abi.rs`：`MutableRef { unpin } => unpin && noalias_mut_ref`；`rustc_middle/src/ty/layout.rs`：`unpin: optimize && ty.is_unpin(..)`。`Frozen` 对应 `SharedRef { frozen }`，即 `is_freeze`）：

```text
  6append: ptr noalias noundef nonnull align 8 dereferenceable(24) %dst, ptr noalias noundef nonnull readonly align 1 captures(address) %src.0, i64 noundef range(i64 0, -9223372036854775808) %src.1
  declare grow_one: ptr noalias noundef align 8 dereferenceable(16)
  contrast append_my: ptr noalias nofree noundef align 8 captures(none) dereferenceable(24) %dst, ...
```

几点观察：`src.0` 是 `&[u8]` 的数据指针，**没有 `dereferenceable`**。`grow_one` 的声明没有 `captures(none)`，所以 `append` 的 `dst` 也推不出 `captures(none)`；`append_ext` 能推出来（见脚本输出）。内联 `push_mut` 之后，`dst` 的 4 次访问带 `!alias.scope`（scope 名是 `push_mut: %self`）。经 `self.ptr` 写入元素的那条 `store i8` **没有 `!noalias`**：LLVM 内联时无法证明这个从内存读出的指针不派生自 `self`（机制为推断，本机没有 LLVM 源码）。

**x86 循环**（rustc 1.99.1，每个基本块一行）：

| 函数 | 热循环 | 对 `*dst` 的访问/迭代 |
|---|---|---|
| `append` | `.LBB8_4: ; movq 8(%r15), %rax ; movb %bpl, (%rax,%r12) ; incq %r12 ; movq %r12, 16(%r15) ; incq %r13 ; cmpq %r13, %rbx ; je .LBB8_5`<br>`.LBB8_2: ; movzbl (%r14,%r13), %ebp ; cmpq (%r15), %r12 ; jne .LBB8_4 ; movq %r15, %rdi ; callq *…grow_one…` | 读 cap、读 ptr、写 len |
| `append_raw`（无 noalias） | 与 `append` 逐条相同（`36 asm-inst`） | 同上 |
| `append_my`（`grow` 可见） | `.LBB1_2: ; movzbl (%r14,%r13), %ebp ; cmpq %rax, %r12 ; jne .LBB1_4 ; … callq …grow ; movq (%r15), %rax ; jmp .LBB1_4` | cap 留在寄存器（冷块重读），**ptr 仍每次读**，len 每次写 |

结论：(1) len 留在寄存器，靠的是 std 手写的 `let len` 加上 store→load 转发，**每次迭代仍然 store len**。(2) `dst` 的 noalias 在这个循环里对 LLVM 没有作用：`append_raw` 生成的代码完全相同。(3) LLVM 只有在知道扩容函数 `captures(none)` 时才把 cap 留在寄存器，ptr 仍然每次重读。`append_ext` 的形状是：`reserve` 的比较（分支带 `branch_weights expected 1:2000`，来自 `#[cold] do_reserve_and_handle`），然后调用 memcpy，再写回 len，冷调用之后重读 len，共 `34 asm-inst`。

## 3. kfrc 编译计划

**Body**：与上面的 MIR 相同，循环体只有一个 `Call push_mut(copy _1, move _5)`，`unwind continue`。

**R 图**：下面是内联 `push_mut` 和 `Iter::next`、`Iter` 槽位提升之后的图。按 nodes.md 的 dump 格式书写；`FieldAddr[a.b]` 是投影链的简写。

```text
// P0=Unknown  P1=Unique(dst)  P2=Frozen(src)  P3=内联 push_mut 时 Reborrow 出的 Unique（规则 5）
n0  = Start : (Control, Effect[P0], Effect[P1], Effect[P2], &mut Vec<u8>, &[u8])
n1  = Field[0](n0.5) : *const u8                              // src.ptr
n2  = IndexAddr(n1, Field[1](n0.5)) : *const u8               // end
n3  = Loop ctl=n0.0 back=n22
n4  = Phi(n3; n1, n8) : *const u8                             // Iter.ptr
n5  = Phi(n3; n0.1, n19) : Effect[P0]
n6  = Phi(n3; n0.2, n21) : Effect[P1]
n7  = If ctl=n3 (Compare.eq(n4, n2)) : (Control, Control)     // .0 退出 → Return
n8  = IndexAddr(n4, 1) : *const u8
n9  = Load[P2](n4) mem=n0.3 : u8                              // b
n10 = Reborrow{Unique}(n0.4) mem=n6 : (&mut Vec<u8>, Effect[P3])
n11 = Load[P3](FieldAddr[len](n10.0)) mem=n10.1 : usize
n12 = Load[P3](FieldAddr[buf.inner.cap](n10.0)) mem=n10.1 : usize
n13 = If ctl=n7.1 (Compare.eq(n11, n12)) : (Control, Control)
n14 = Call(RawVec<u8>::grow_one) ctl=n13.0 mem=[P0:n5, P3:n10.1] (FieldAddr[buf](n10.0)) : (Control, Control, Effect[P0], Effect[P3], ())
n15 = Region(n13.1, n14.0)
n16 = Phi(n15; n5, n14.2) : Effect[P0]
n17 = Phi(n15; n10.1, n14.3) : Effect[P3]
n18 = Load[P3](FieldAddr[buf.inner.ptr](n10.0)) mem=n17 : *mut u8
n19 = Store[P0](IndexAddr(n18, n11), n9) mem=n16 : Effect[P0]  // 根是 load → Unknown（规则 1）
n20 = Store[P3](FieldAddr[len](n10.0), Add.wrap(n11, 1)) mem=n17 : Effect[P3]
n21 = EndBorrow(n10) mem=n20 : Effect[P1]
n22 = Jump ctl=n15
// n14.1（unwind）→ EndBorrow(n10) mem=n14.3 → Resume
```

**优化步骤**（推断）：

| # | 动作 | 依据 | 设计里有没有 |
|---|---|---|---|
| 1 | 把 `Vec<u8>::push`/`Iter<u8>::next` 内联进非泛型的 `append` | 被调者依赖 `T`，实参是具体类型，不是 `append` 的 `Param` | **没有**：R 级规则不允许；`append` 没有孔位，S 级种子为空（W05-9） |
| 2 | `P3` 并回 `P1`，删除 n10/n21 | 从 Reborrow 到 EndBorrow 之间，`P1` 链的 token 被 Reborrow 占住，父分区不可能被访问 | **没有**（W05-6） |
| 3 | len：n11 的 `mem` 回溯到循环 Phi，回边输入是 n20 → `len = Phi(预头部 load, n11+1)` | 分区内转发 | 需要“穿过 Phi 的转发”（W05-7） |
| 4 | cap/ptr：在 `P1` 链上只有 n14 写它们 → 冷块里重读，热路径用 Phi | load PRE | **没有**；placement 外提不了，因为 token 随循环变化（W05-7） |
| 5 | n19 不排在 cap/ptr/len 之后 | `P0` 与 `P1` 是不同的分区；前提是 `grow_one` 的摘要 `captures=∅`，否则按规则 4 插入 `Escape` | 摘要有；汇合处的语义**没有**（W05-1） |
| 6 | len 的 store 下沉出循环 | 要在 n14 之前、unwind 出口、循环出口三处补写 | **没有**，所以和 LLVM 一样每次都写 |

**S 级**：`append` 没有孔位，`specialize` 是恒等变换。被调实例 `grow_one` 的答案键来自 std，下游直接复用 std 的实例：IR 里只有声明，没有定义。

**L 级与期望机器码**（推断，前提是第 2–5 步都做到）：

```text
.Lloop: movzbl (%rsi),%eax ; cmpq %rcx,%rdx ; je .Lgrow          // len、cap 都在寄存器
.Lpush: movb %al,(%r8,%rcx) ; incq %rcx ; movq %rcx,16(%rdi)     // ptr 在寄存器；len 仍然每次写回
        incq %rsi ; cmpq %rsi,%r9 ; jne .Lloop
.Lgrow: 保存活跃值 ; call grow_one ; 重读 cap、ptr ; jmp .Lpush
```

如果拿不到摘要、或者走了规则 4 的合并，热循环会退回 rustc 的形状。`append_ext` 期望与 rustc 相同：`reserve` 的比较加冷调用，然后是 `CopyMem`，从 `P2` 读、向 `P0` 写。

## 4. 代价核算

| 项 | rustc | kfrc（推断） |
|---|---|---|
| 进入优化的 IR | 3 个函数共 112 条指令（35+43+34），alloca 5、lifetime 8、assume 13 | 内联后 R 图约 25 个节点，没有 alloca、lifetime、assume |
| 热循环 | 10 条指令，访问 `*dst` 3 次（2 读 1 写） | 9 条，1 次写；没有摘要时与 rustc 相同 |
| 机器码键 | — | `append` 的答案键 + `push_mut<u8>`/`Iter<u8>::next` 的答案键；**还应包含 `grow_one` 的摘要**（W05-8） |

## 5. 设计细化

| # | 问题 | 证据 | 建议改法 | 文件 |
|---|---|---|---|---|
| W05-1 | **健全性**：规则 4 说逃逸“此后并入 `Unknown`”，这是流敏感的，但分区按根静态指定（规则 1）。如果只在冷路径上逃逸，两条链在汇合后仍然分开：热路径上 `P0` 的 store 与 `P1` 的 load 之间没有顺序约束。而被调者可能已经把 `dst` 派生的指针写进了 `buf.ptr`，这时会读到旧值 | O3 里 `store i8` 没有 `!noalias`，`grow_one` 也没有 `captures(none)`：LLVM 在这里不敢把两者分开 | 逃逸状态改成数据流性质：在 Region 上只要有一个前驱已逃逸，就在其余前驱插入 token 合并，此后共用一条链；回边会把这个状态传遍整个循环。M6 可以先做最简单的版本：函数内任意位置逃逸，就让该分区从 `Start` 起就是 `Unknown` | memory-partitions.md 规则 4；son-verifier.md |
| W05-2 | **健全性**：Reborrow 出的子分区逃逸后，父分区没有跟着逃逸。被捕获的指针同样派生自 `dst`，在 `EndBorrow` 之后仍然可以使用 | n14 的分区是 `P3`，n21 把它合回 `P1` | `EndBorrow` 的输入分区已逃逸时，父分区也标为逃逸 | memory-partitions.md 规则 4/5 |
| W05-3 | **健全性**：规则 2 说因为有 `dereferenceable`，`Frozen` 的 load 可以上提出条件。但这只对 sized pointee 成立 | `src.0` 没有 `dereferenceable`；`frozen:false` 和 `unpin:false` 时 rustc 给的 size 是 0（abi.rs）。如果把 n9 提到 n7 之前，会越界读 | 投机读只允许落在“静态大小内的常量偏移”上；下标或 unsized 访问保留控制依赖 | memory-partitions.md 规则 2；values-and-types.md 值事实 |
| W05-4 | **健全性**：规则 1 的派生链没有覆盖 `Phi`/`Select`、指针 cast/transmute、聚合值的字段。根混合时，给出的答案是 `Unknown`，而用 `Unknown` 的 token 访问 `Unique` 内存会与 `Unique` 链失去顺序 | n4 = `Phi(src.ptr, ptr+1)`；MIR 中有 `NonNull` 的 `Transmute` | 根按集合计算；单一根时用该分区；混合根时同时取所有相关分区的 token（或者插入 `Escape`）；verifier 按这条规则检查 | memory-partitions.md 规则 1；son-verifier.md |
| W05-5 | **健全性**：规则 3 只考虑“作为实参的指针”，没有考虑放在聚合值实参字段里的指针（例如按值传入的 `IterMut`、闭包环境、`(&mut a, &mut b)`） | R/S 级的聚合是值；`extend_from_slice` 按值把 `slice::Iter` 传给 `spec_extend` | 规则 3 改为“实参值里沿值字段可达的全部指针”（不穿过内存） | memory-partitions.md 规则 3 |
| W05-8 | **错误命中**：调用者的优化读取了没被内联的被调者的摘要（`captures`、`memory`、`may_unwind`），但机器码键只包含被内联者 | 第 5 步完全依赖 `grow_one` 的 `captures=∅` | 机器码键加入“每个被查询过的摘要的哈希”；摘要不变时仍然命中 | design/build/cache.md；optimization-pipeline.md 调用摘要 |
| W05-6 | 在已经是 `Unique`/`Local` 的父分区下做 `Reborrow{Unique}`，没有带来新信息，反而让每次迭代都要跨过 Reborrow 和 EndBorrow 才能转发 | 第 2 步 | 规则 5 只在实参的根是 `Unknown`（或来自 load）时才建子分区；否则直接沿用父分区 | memory-partitions.md 规则 5 |
| W05-7 | runtime.md 写“`Unique` 链上没有别人”，但只要有一条分支调用，token 就随循环变化，placement 外提不了 | 第 3/4 步；LLVM 在 `append_my` 里也只把 cap 留在了寄存器 | 增加“穿过分区 Phi 的 load 转发与 PRE”；store 下沉作为可选项（要在调用前、unwind 出口补写） | optimization-pipeline.md；runtime.md |
| W05-9 | 非泛型函数调用具体类型的泛型实例时，R 级不能内联，S 级又没有种子。rustc 的 MIR 内联器能做到 | MIR 中有 `scope 34 (inlined Vec::<u8>::push)` | S 级种子包含所有已解析的直接调用；或者在 R 级用 `specialize(被调者, 具体答案)` 内联 | optimization-pipeline.md；generics-specialization.md 特化 |
| W05-10 | `CopyMem[P]` 只有一个分区，但 `copy_nonoverlapping` 从 `P2` 读、向 `P0` 写 | `append_ext` 的 memcpy | 改为 `CopyMem[Pdst←Psrc]`，同时接两个效果输入 | nodes.md |
| W05-11 | push 的扩容路径按 kfrc 的定义不是冷区域（`grow_one` 只标了 `inline(never)`），`reserve` 的扩容路径是冷区域 | LLVM 也一样：push 分支没有权重，reserve 分支是 1:2000 | 在冷区域定义里写明：`inline(never)` 不等于冷；`hint::cold_path` 算冷 | ownership-drop-unwind.md 冷区域 |
