# LP3 candidate28：identifier 回退的静态归因与下一次 profile

2026-11-09。只读取已有正式测量、对象反汇编、重定位、S/L 图和固定输入源码；没有重新编译、
运行 benchmark 或启动审查。结论是：**identifier 的 +6.70% 回退仍未定位；不能沿用
candidate23 的“两个热点机器序列均相同”结论。** candidate28 的 `parse::identifier`
已改变并缩小，`bytes_for_varint` 与 Identifier 的分配／clone／比较／析构路径仍相同。

## 正式输入和测量边界

证据根目录为 `evidence/poc-llvm-parity-lp3-20261109/results/candidate28/`。
以下相对路径均从该目录起算：

- `semver-performance/summary.json`、`samples.json`、`inputs.json`：本次四臂正式结果，
  `baseline22` 是冻结22、`baseline` 是冻结27、`kfrc` 是冻结28。
- `attribution-formal/{baseline22,kfrc}/`：精确对象符号大小、机器计数、反汇编与 `graphs/stderr.gz`。
- `transport-static/{baseline22,kfrc}.relocations-disassembly`：同批对象的指令字节和重定位目标。
- benchmark 源码为 `scripts/poc-validation/semver-perf-driver.rs`，SHA-256
  `e5e284fe1b96baa40b0c37bbdea123f56c68c7fd8939e02a6efdd549fe46ea3e`，与 inputs.json 一致。

identifier 每次运行 10,000 轮，输入固定为 1,024 个 ASCII `a`；每轮构造 Prerelease、clone、
取 clone 的字符串长度、比较两个值，最后析构两者。所有正式样本 checksum 为 10,250,000。

| arm | identifier 中位耗时 | 相对22 |
|---|---:|---:|
| baseline22 | 103,857,114 ns | — |
| baseline27 | 110,603,490 ns | +6.50% |
| candidate28 | 110,812,773 ns | +6.70% |

28 相对同轮27仅 +0.19%，不能由本轮数据把从22累计的回退归咎于28新增的子字段快照。
22七次样本范围102.400–106.072 ms，28为110.393–115.794 ms；这轮差异不能简单忽略，
但只有 elapsed 时间仍无法区分执行工作量、微架构与宿主调度。

已保存的正式driver SHA-256：22为
`ffd415ded95e5c1f7b1a42fd32c884e61c9de46de4743e4f722d3611d6027db0`，
28为`48a05422dc157f626ea575cecf1943831b89d342b84d0938237746eff3a37788`。
本地证据中没有28正式driver的链接后完整反汇编；本次地址比较仅到对象 `.text` 偏移，
不能把23的链接后归一化结果当成28的结果。

## 两个原热点并非都保持原序列

比较精确 ELF 对象符号 `[start, start + st_size)` 内所有指令字节；不把符号后的对齐填充
算入正文。重定位另按“相对符号偏移、类型、目标”比较，避免把尚未链接的零位移误作
完整等价证据。nm原始和demangled输出按对象文件、地址、size、type关联，不能按行号关联。

| 函数 | 对象字节22→28 | 指令数22→28 | 栈帧分配22→28 | 对象字节／重定位 |
|---|---:|---:|---:|---|
| `parse::identifier` | 7,892→5,670 | 1,456→1,123 | 0x400→0x320 | 均不同 |
| `identifier::bytes_for_varint` | 5,397→5,397 | 1,288→1,288 | 0x10→0x10 | 均完全相同 |
| `parse::prerelease_identifier` | 4,221→1,051 | 808→238 | 0x3b0→0x130 | 均不同 |
| `Prerelease::from_str` | 4,230→1,175 | 800→261 | 0x370→0x140 | 均不同 |
| `Prerelease::new` | 3,037→3,037 | 相同 | 0x230→0x230 | 均完全相同 |

`parse::identifier` 的 mov 从1,056降至806，rbp相对mov从675降至449，条件跳转41→30，
无条件跳转75→50，调用仍6处。rbp计数只是帧内存代理，不是经过证明的真实 spill 次数。
其最终 Specialized 图仍546节点；Layout由987→776，Phi98→53，Undefined57→1，
Load31不变，Store35→31，If41→30。28 stats为 owned_transport_phis=2、
transported_boundary_fields=0；这里变化来自完整返回值表示，不是新边界字段capture。

精确符号字节 SHA-256：

| 函数 | 22 | 28 |
|---|---|---|
| `parse::identifier` | `f88d8252760bf7a35b7de88cccaf80bbfc1c2212929b06ec7f03236bfcf3175d` | `e347ca5027f86b050e696d283ed8597236a91334e9cfd2cc4d94523bca41ee10` |
| `bytes_for_varint` | `79e90e4cb137e9722e23201cb71645eea47a861b6e5ddd4e59492655cfdfa80a` | 同22 |

对象内 `parse::identifier` 起点从0xc060移到0x78b0；`bytes_for_varint` 所在独立对象的
起点仍0x29d0。前者偏移改变和机器体变化都已观察到；缓存／分支预测受布局影响只是
待检验假设，不能写成回退原因。体积、静态分支数、栈帧下降也不证明成功路径更快。

## workload 还经过哪些代码

源码来自本机 cargo registry 的 semver **1.0.28**，已核对 `identifier.rs`、`lib.rs`、
`parse.rs`、`impls.rs` 的 SHA 与本次 inputs.json 完全一致。1.0.27 的 parse.rs 恰好相同，
但其余文件不同，不能拿1.0.27解释当前 Identifier 所有权实现。

实际流程是：

1. `Prerelease::new` → `Prerelease::from_str` → `parse::prerelease_identifier`
   → `parse::identifier`，随后 `Identifier::new_unchecked` 分配并复制长字符串。
2. `Identifier::clone` 解码长度，调用 `bytes_for_varint`，分配另一块内存并复制。
3. `Prerelease::as_str` → `Identifier::as_str` → `ptr_as_str`／`decode_len`；
   比较经 `Prerelease::eq` → `Identifier::eq` → `[u8]::eq`，需要比较不同分配中的字节。
4. 两个值析构，`Identifier::drop` 各自经 `decode_len`、`bytes_for_varint` 后释放。

这条路径中下列对象函数在22／28间正文与重定位均相同：

| 函数 | 对象字节 |
|---|---:|
| `Identifier::new_unchecked` | 3,241 |
| `Identifier::clone` | 866 |
| `Identifier::drop` | 549 |
| `Identifier::as_str` | 436 |
| `Identifier::eq` | 1,823 |
| `ptr_as_str`／`decode_len`／`decode_len_cold` | 325／359／401 |
| `ptr_to_repr`／`inline_as_str`／`inline_len` | 94／145／5,629 |
| `Prerelease::as_str`／`Prerelease::eq` | 115／53 |

`new_unchecked`／clone仍调用 `__rust_alloc`、`copy_nonoverlapping::<u8>`（最终内存复制），
Drop调用 `__rust_dealloc`。Allocator、libc memcpy/memcmp、driver本身及链接后地址必须纳入
后续profile，不能把整个benchmark时间归给两个原热点。

`parse::identifier` 的6个直接调用目标未变：slice metadata、`str::is_char_boundary`、
`RangeFrom<usize>::get_unchecked`、`str::starts_with::<char>`、`slice_error_fail`、`str::split_at`。
其中失败路径函数的静态存在不代表固定合法ASCII输入会执行它们。

两个改变的wrapper各有1个boundary capture；各新增2个Error表示memmove，共4个静态位置。
`prerelease_identifier` 对应L节点n137/n203，`Prerelease::from_str` 对应n153/n243：
第一处在原返回Result的Err guard内capture，第二处只在Err转运路径使用。
固定`a`×1024是合法完整Prerelease，按源码和图的分支条件不会走这4处。
因此，不能仅凭新增memmove数量解释当前identifier成功路径回退。

## 下一次profile如何区分假设

下面是待执行方案，本次没有启动。保留22／28正式可执行文件和共同输入，不重新构建混入RS。
若需要延长采样，仍用同一个driver的iterations参数，记录该变化；profile与正式计时分开。

1. 在同一已核对E2B宿主环境串行采集两臂，保存CPU、工具链、二进制、sysroot与输入指纹。
   先取per-process cycles/instructions/branches/branch-misses、task-clock、context-switches、
   migrations；CPU事件不可用时明确记录限制，不以编译profile替代应用profile。
2. 采集符号／调用栈样本，至少区分解析三层wrapper、`parse::identifier`扫描循环、
   `bytes_for_varint`、Identifier clone/drop、allocator、复制与字节比较。
   同时收集正式driver链接后反汇编、重定位和符号地址，归一化内外调用及RIP数据引用。
3. 核对成功路径新Error memmove确实零执行；再比较扫描循环的动态指令数、分支走向与
   寄存器搬运。若总retired instructions增加，先解释增加的执行路径；若instructions接近而
   cycles／branch misses增加，再检查布局和预测；若task-clock接近但elapsed增加，检查调度。
4. 只有采样确定热点后才做单独的机制对照。例如固定布局实验检验地址效应，或保留字节
   相同的varint代码与不同wrapper组合检验调用上下文；不可仅调整对齐后挑最好的一次。
   每次变更仍要求行为差分、固定基线和同宿主原版／候选串行对照。

目前LP3可接受结论仍是编译工作量和体积改善，并记录req运行改善与identifier回退并存。
没有足够证据宣称整体runtime达到LLVM parity，也没有定位到应回滚哪条具体规则。
