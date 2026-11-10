# 最终产物对照：运行冗余、枚举展开与链接粒度

2026-11-10，源码基线 `ec9ce39fa70841e15c4d241a1f1525f0abcea3fe`，继续 PR #8。本轮只诊断，未改变编译器、未新增运行计时。重新反汇编已核验的最终 `data` 与 LLVM 产物：100 个 ELF 对象、626 个去除同地址别名后的函数范围，其指令数逐项与前次统计吻合。对象指纹见 [objects.json](objects.json)，复算入口为 [analyze.py](analyze.py)。

主要结论：当前运行差距首先表现为**多执行几倍指令**；humantime 的枚举/聚合 ABI 往返、base64 的范围检查和调用链、Unicode reverse 的小包装调用是具体来源。另确认了影响链接体积的独立问题：同一 CGU 共用 `.text`，阻断逐函数 section GC。提高全局内联预算或先引入 SIMD，都不能替代这些修复。

## 1. 动态执行量与时间要分别看

时间列沿用最终 31 轮配对中位数。指令、cycles 来自同批串行计时结束后每臂单次 `perf stat instructions:u,cycles:u`，覆盖完整 driver 进程，不是逐函数采样。后两列是单次剖面，不能按 31 轮置信区间解读，也不能直接归因为缓存/分支预测。完整 13 个负载见 [dynamic.json](dynamic.json)。

| 负载 | 时间 KFRC/LLVM | 动态指令比 | cycles 比 | KFRC / LLVM CPI |
|---|---:|---:|---:|---:|
| base64 encode | 9.71 | 5.91 | 9.66 | 0.360 / 0.221 |
| base64 decode | 8.13 | 5.98 | 8.04 | 0.236 / 0.176 |
| humantime duration parse | 10.24 | 7.57 | 10.09 | 0.329 / 0.247 |
| semver requirement | 7.44 | 6.32 | 7.63 | 0.296 / 0.245 |
| semver parse | 4.05 | 4.07 | 3.94 | 0.243 / 0.251 |
| Unicode reverse | 3.76 | 3.56 | 3.77 | 0.222 / 0.210 |
| Unicode ASCII | 1.09 | 1.05 | 1.10 | 0.227 / 0.216 |
| humantime timestamp format | 1.65 | 1.95 | 1.65 | 0.272 / 0.321 |

base64 除了执行指令多，单位指令 cycles 也更高；下一次需要分支事件和采样才能区分依赖链、调用、访存及布局。反过来，timestamp format 的 CPI 比 LLVM 低，仍因指令更多而整体慢。不能用静态条数或 CPI 单独替代性能指标。

## 2. humantime：优先解决 niche enum 的反复解码/编码

| `Parser::parse_unit` | LLVM | 最终 KFRC |
|---|---:|---:|
| 机器码 B | 1,093 | 23,799 |
| 静态指令 | 293 | 5,674 |
| 条件 / 无条件跳转 | 54 / 26 | 714 / 722 |
| 含内存操作数的指令 | 47 | 1,450 |
| 相关 niche tag 常量物化 | 3 | 717 |
| 入口 `sub rsp` 预留 B | 24 | 6,448 |

栈预留不包括额外 push、动态对齐或 red zone，不能当作完整峰值栈量。tag 计数精确限定为立即数 `-9223372036854775808..-9223372036854775800` 的 mov 系列指令。

KFRC 在 `ok_or` 返回处读 tag，分支读 payload，再按嵌套 Error 的不同 niche tag 解码，随后再次编码并调用 `Try::branch`。这种结构在同一函数内反复出现；[汇编摘录](excerpts.md) 保存了首个 `ok_or` 附近的连续指令。最终仍有 17 个 `ok_or`、28 个 `Try::branch`、9 个 OverflowOp::div 调用点。辅助函数自身也不是免费的：`ok_or` 为 871 B / 216 指令，`Try::branch<Result<u64, Error>>` 为 465 B / 120 指令。

源码限制与产物一致：

- `lower_layout/variants.rs::decode_variant_with_transport` 按被需求的变体字段生成守卫并递归解码。
- `lower_layout/variant_lanes.rs::plan` 的共享 lane 只接受 direct-tag；嵌套 niche enum 不受这条优化覆盖。
- `lower_layout/boundary_transport.rs::select` 的整体 transport 证书严格限定使用者，且包含回边时拒绝整个函数。它不是覆盖所有 enum ABI 往返的通用表示。
- `codegen.rs` 在 layout lowering 后直接验证并进入 x86 lowering，没有一轮通用 L 层内存提升与 CFG 清理来消除新暴露的冗余。

这些是源码限制与机器码模式的对应，尚未通过逐节点来源映射证明每条分支属于哪个降低步骤。不能简单删除 payload 守卫：inactive payload、未初始化字节、drop 和错误值传递都必须保留合法性。

**最先实验**：对满足所有权/不逃逸条件的嵌套 niche 结果保留整体表示，只在真正观察字段处解码；同时把 `ok_or → Try::branch` 这类 seam 在 S 层化简掉。记录每函数 S/L 节点数、layout 新增分支、栈字节和编译时间。验收应包括完整 Error payload、循环中重复调用、drop 和非法 speculative read 负例。

## 3. base64：当前 LLVM 主循环仍是标量，检查和包装调用才是第一缺口

`internal_encode` 的 LLVM 产物没有使用 XMM/YMM/ZMM 寄存器，依靠整数 load、bswap、移位和表查找。最终 KFRC 与 LLVM 都有 44 条非 CL 移位，动态移位问题已经解决。

| `internal_encode` | LLVM | 最终 KFRC |
|---|---:|---:|
| 机器码 B | 1,376 | 5,629 |
| 静态指令 | 327 | 1,287 |
| 条件 / 无条件跳转 | 16 / 1 | 77 / 85 |
| bounds-panic 调用点 | 5 | 56 |
| 直接使用 rbp/rsp 的内存指令 | 0 | 243 |
| `read_u64` 调用点 | 0 | 4 |

`read_u64` 又包含 108 指令 / 7 调用，涉及 slice metadata、len、as_ptr、cast_array 和 Result::unwrap。LLVM 把这些语义收缩进 load/bswap。panic 调用通常是冷路径，因此 **56 个调用点不表示正常输入执行 56 次 panic**；相关判断、调用边界和活跃值才需要动态归因。

**优先顺序**：有界 callee-first 化简 `read_u64` 及 slice 包装 → 传播主循环的输入/输出剩余长度 → 在支配条件下删除重复 bounds 检查 → layout 后消除私有栈往返。保留短输入、尾部、输出不足、溢出与别名边界测试。等标量循环接近 LLVM 后再测 SIMD 的额外价值。

## 4. 小函数直接揭示缺失的 CFG 和私有内存清理

`Identifier::ptr_eq`：LLVM 4 指令 / 10 B，KFRC 11 指令 / 30 B。KFRC 已经执行 `cmp; sete; movzx`，随后又 `test; je`，在两条边上分别赋 1/0，最后返回。这是可独立最小化的 bool → branch → bool 往返，适合用 SimplifyCFG/InstCombine 的思路折叠。

`Identifier::is_empty`：LLVM 3 指令 / 8 B，KFRC 20 指令 / 58 B。KFRC 为常量 -1 建 16 B 栈空间、store 后 reload，再执行相同的布尔往返，结束前还写零。说明不应把每个栈访问都归咎于寄存器溢出：这里首先是私有对象与值生命周期没有收缩。后端删除这些写入前，要证明其私有且不再被观察。

humantime `Unit::from_str` 也仍有 40 个字符串/切片相等调用点，LLVM 为 0。`semver::parse::op` 保留 35 调用点，包含重复 Option 比较、metadata、边界检查与切片包装。现有初次 S 优化已经存在；缺口是 callee 包装链在成为上层内联候选前未充分收缩，以及 splicing/layout 新暴露形态的清理不足，不能表述为“完全没有预优化”。

## 5. Unicode reverse：单看函数变小会误判

KFRC `prev_boundary` 只有 1,326 B，LLVM 为 2,906 B，但该负载 KFRC 仍慢 3.76 倍。KFRC 把工作留在 `is_boundary`（2,105 B / 37 调用）、UTF-8 反向解码（484 B / 7 调用）及其他 helper 中；LLVM 的调用者包含更多被融合的逻辑。LLVM 未独立保留 `is_boundary`/反向解码函数并不表示相关工作不存在。

反向解码 helper 还调用 4 次 `NonNull::sub` 和 3 次 `unreachable_unchecked`。应按调用链及执行路径衡量，而不是要求每个同名函数都比 LLVM 更小。先化简迭代器/指针/char 包装，再处理公共边界事实；Unicode ASCII 当前已在 1.09 倍附近，应作为不得回退的对照。

## 6. 新确认的产物缺口：同一 CGU 的 section GC 粒度

| 库 | `.text` KFRC / LLVM B | 只读数据统计 KFRC / LLVM B | 对象总字节 KFRC / LLVM |
|---|---:|---:|---:|
| semver | 34,052 / 18,515 | 11,795 / 2,008 | 149,960 / 85,240 |
| base64 | 28,778 / 8,648 | 10,890 / 1,926 | 128,416 / 53,632 |
| Unicode | 18,490 / 8,398 | 90,444 / 75,874 | 179,864 / 112,552 |
| humantime | 58,709 / 10,426 | 11,149 / 1,656 | 165,960 / 53,768 |

体积沿用 [library-sizes.json](../division-inline/library-sizes.json) 的口径；只读数据统计包含 `.rodata*` 与 `.data.rel.ro*`。对象大小还包含符号、重定位等，rlib 又包含 Rust metadata，不混用分母。626 个函数范围已去除同地址别名；函数字节和也不等于含对齐空隙的 `.text`。

`object_file.rs::write_object_with_data` 为整个 CGU 取得一个 `.text`，将每个函数以 16 字节对齐追加；只读数据也共用相应 section。LLVM 为函数/不少数据对象生成独立 section。因此保留一个函数可能连带保留同 CGU 的其他代码和数据。

新增可复算实验：[gc_probe.py](gc_probe.py)，在原始 base64 CGU00 上用同一 LLD 执行 `-r --gc-sections -u <internal_decoded_len_estimate>`，只指定同一个保留根：

| 项目 | LLVM | KFRC |
|---|---:|---:|
| 目标函数本身 | 27 B | 31 B |
| GC 后保留 `.text` | 27 B | **13,654 B** |
| GC 后保留 `.rodata` + `.data.rel.ro` | 0 B | **5,059 B** |

结果见 [gc-probe.json](gc-probe.json)。实验使用本机 LLD 23.1.1，对照双方使用同一链接器；并非重放原 Linux 驱动的完整链接过程。这是**单对象、可重定位链接的粒度验证**，没有解析完整依赖闭包，也不是可运行的小 API 消费者；不能据此宣称完整程序小 500 倍。它证明小函数机器码已相近时，输出组织本身仍会阻断链接剔除。

这一项应独立实现 per-function sections / 合法的数据 section 分离，保留 symbol binding、alignment、TLS、地址身份及 relocation 语义，再用真实小 API 消费者和四库驱动验证链接体积、对象/重定位开销与编译/链接墙钟。不要把 read-only 等同于可合并地址身份，也不能预先声称会修复 duration format 的布局回退。

## 7. 下一轮实施次序与证伪条件

| 优先级 | 工作 | 对应 LLVM 方法 | 首个可证伪验收 |
|---|---|---|---|
| P0，产物体积 | 函数/数据 section 粒度 | FunctionSections / DataSections + linker GC | 单根 probe 不再拖入整个 CGU；真实消费者缩小，身份/重定位测试通过 |
| P0，通用低成本 | bool 分支往返、私有常量槽清理 | SimplifyCFG + InstCombine + SROA/promotion | ptr_eq 不再 branch 回填 bool，is_empty 不再保留私有常量栈槽；无额外 poison 观察 |
| P1，最大运行缺口 | 嵌套 niche enum transport、Result seam | SROA 的表示选择、按需 scalarization、内联后清理 | parse_unit 的 L 节点、tag 物化、栈空间同时下降；Error/drop 合同不变 |
| P1，base64 | 有界 callee-first + 范围事实 | CGSCC simplification、CVP/JumpThreading、KnownBits/range | read_u64 包装调用消失、重复 bounds 检查减少；短输入/尾部不回退 |
| P2，Unicode/semver | 迭代器、指针、字符串包装收缩 | 同上，复用已验证机制 | reverse/req 动态指令降低，ASCII 和 identifier 不回退 |

不设置未经实验支持的收益承诺。每项同时记录正确性、运行耗时、库/消费者体积及完整编译墙钟；继续保留用户原来的编译降低 50% 目标。本轮并未产生新的编译收益，也没有完成四库 LLVM 收敛。

## 复算与证据边界

输入为上一轮完整证据包 `69420198a6eaf347213f632598bf81582be97a112c3430d225692867f37f5ad7` 解包后的 `results/`。本机输出目录 `/Users/shom/Downloads/kfrc-divshift-20261110/artifact-review`。反汇编器 Apple LLVM 15 仅负责读取原始 x86-64 ELF，没有重新生成机器码；链接器版本记录在 gc-probe.json。

```sh
python3 analyze.py EVIDENCE_RESULTS OUTPUT_DIR --objdump /path/to/llvm-objdump
python3 gc_probe.py EVIDENCE_RESULTS GC_OUTPUT --lld /path/to/ld.lld --objdump /path/to/llvm-objdump
```

仓库保存全部对象指纹、库函数汇总、23 个选定函数数据、动态比例与汇编摘录，完整反汇编可由脚本重建。没有新开沙箱。参考用户 LLVM study 的 03 SROA、05 CFG、09 IPO、13 layout、16 phase ordering；这些是方法对应，不表示已从 LLVM 原样移植相应 Pass。
