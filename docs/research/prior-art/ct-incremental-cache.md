# 增量与缓存：查询图、内容寻址、源位置、增量链接

问题：[cache.md](../../design/build/cache.md) 的四个做法——(a) 键只含读过的东西、答案键代替类型参数；(b) 源位置隔离；(c) S 图不存、只存 R 图和机器码片段；(d) build 级内容寻址存储 + 根清单回收——有没有先例和反例？并行/流水线见 [ct-parallel-pipeline.md](ct-parallel-pipeline.md)。

## 1. 是什么

| 项目 | 年份 | 来源 | 要点 |
|---|---|---|---|
| rustc 增量 | 2016 alpha；2017 red-green | Woerister [博客](https://blog.rust-lang.org/2016/09/08/incremental/)（2016）；[PR #44901](https://github.com/rust-lang/rust/pull/44901)（2017-10 合入）；[dev-guide](https://rustc-dev-guide.rust-lang.org/queries/incremental-compilation-in-detail.html) | 读数据时自动记边；`try_mark_green` 逐个验证依赖；结果算 128 位指纹，与旧值相同即为绿（early cutoff）；`cache_on_disk` 只给部分查询；后端以 CGU 为单位，CGU 不绿就整单元重编 |
| rustc 的 span | 2018 起，仍开放 | [issue #47389](https://github.com/rust-lang/rust/issues/47389)；相对 span [PR #84373](https://github.com/rust-lang/rust/pull/84373)、默认开启 [PR #115656](https://github.com/rust-lang/rust/pull/115656)（1.74） | span 在 HIR、MIR、调试信息和 panic 消息里，加一行注释使其下所有项失效；方案“抽象 span 放旁表”未落地，只做了“相对父项编码”；`-Zincremental-ignore-spans` 在 [options.rs](https://github.com/rust-lang/rust/blob/master/compiler/rustc_session/src/options.rs) 中标为“仅供测试” |
| rustc CGU 与 SVH | — | [partitioning.rs](https://github.com/rust-lang/rust/blob/master/compiler/rustc_monomorphize/src/partitioning.rs) 头注释；[codegen-units 文档](https://doc.rust-lang.org/rustc/codegen-options/index.html#codegen-units)；[SVH](https://rustc-dev-guide.rust-lang.org/backend/libs-and-metadata.html) | 每个源模块分“稳定/易变（泛型实例）”两个 CGU；新增对泛型的引用也会使 CGU 失效；SVH 含全部 HIR 哈希、上游 crate 哈希、源文件名 |
| Salsa / rust-analyzer | 2023 | [算法](https://salsa-rs.github.io/salsa/reference/algorithm.html)、[durability](https://salsa-rs.github.io/salsa/reference/durability.html)；matklad [Durable Incrementality](https://rust-analyzer.github.io/blog/2023/07/24/durable-incrementality.html)（2023-07）；[架构文档](https://rust-analyzer.github.io/book/contributing/architecture.html) | backdating = early cutoff；按 durability 分级的版本号，只改低级输入时整片高级子图免验证；规则“在函数体内打字永不使全局派生数据失效”；“前提是不在 AST 里存位置” |
| Build Systems à la Carte | 2018 | Mokhov/Mitchell/Peyton Jones，[ICFP 2018](https://www.microsoft.com/en-us/research/uploads/prod/2018/03/build-systems.pdf) | 验证型 trace 只存哈希、构造型 trace 另存值；**深**构造型 trace（只记终端输入哈希，Nix/Buck）不支持 early cutoff |
| Bazel / REAPI | — | [remote caching](https://bazel.build/remote/caching)；[REAPI proto](https://github.com/bazelbuild/remote-apis/blob/main/build/bazel/remote/execution/v2/remote_execution.proto)；[hermeticity](https://bazel.build/basics/hermeticity) | 两级：Action 摘要 → ActionResult（AC），输出按内容放 CAS；Action = Command 摘要 + 输入树根摘要 + timeout + platform + `salt`；`salt` 用来整批作废被毒化的结果；只有 `--action_env` 声明的环境变量进键 |
| Buck2 | 2023 | Hopman/Mitchell [发布博客](https://engineering.fb.com/2023/04/06/open-source/buck2-open-source-large-scale-build-system/)（2023-04）；[Why Buck2](https://buck2.build/docs/about/why/) | 单一增量图（DICE），无阶段；用与 Bazel 相同的 RE API；本地执行是远程执行的特例；dep files 声明“未用到的输入”，对其变化不敏感 |
| sccache | — | [Rust.md](https://github.com/mozilla/sccache/blob/main/docs/Rust.md)、[README](https://github.com/mozilla/sccache/blob/main/README.md) | 整次 rustc 调用为键；不缓存 incremental crate、调用系统链接器的 crate（bin/dylib/cdylib/proc-macro）；默认要求绝对路径一致，`SCCACHE_BASEDIRS` 剥前缀后，命中产物的调试信息和 panic 消息仍指向原目录 |
| Zig 增量 | 0.14（2025-03）、0.15（2025-08）、2026 | [0.14 notes](https://ziglang.org/download/0.14.0/release-notes.html)；[0.15.1 notes](https://ziglang.org/download/0.15.1/release-notes.html)；mlugg [Inside Zig's Incremental Compilation](https://mlugg.co.uk/posts/incremental-compilation-internals/)（2026-07）；源码 [PerThread.zig](https://github.com/ziglang/zig/blob/master/src/Zcu/PerThread.zig)、[link.zig](https://github.com/ziglang/zig/blob/master/src/link.zig) | 分析单元：类型布局、声明类型、const 值、函数体；依赖记到声明源码哈希；旧新 ZIR 按声明名对应；声明起始行变化只排 `update_line_number` 任务（修调试信息，不重分析），函数内行号相对所在声明；**AIR/MIR 不缓存**；链接器在输出文件内原地打补丁；仍未默认开启 |
| Swift | 2016–2026 | [DependencyAnalysis.md](https://github.com/swiftlang/swift/blob/main/docs/DependencyAnalysis.md)；swift-driver [PR #1413](https://github.com/swiftlang/swift-driver/pull/1413)（2023）；[WWDC24 10171](https://developer.apple.com/videos/play/wwdc2024/10171/)；[LLVM CAS RFC](https://discourse.llvm.org/t/rfc-add-an-llvm-cas-library-and-experiment-with-fine-grained-caching-for-builds/59864)（2022-02）；Rastogi [LLVM DevMtg 2023](https://llvm.org/devmtg/2023-10/slides/techtalks/Rastogi-RepresentingDebugInformationInLLVMCAS.pdf) | 文件级 provides/depends，靠插桩名字查找记录；“黄金规则是保守”；外部模块变化旧时整模块重编；显式模块构建让构建系统看见模块任务；CAS 把对象拆成对象 DAG，`__debug_line` 按函数切分，文件号与重定位加数放在不去重的数据里 |
| Carbon | 2020 起 | [goals.md](https://github.com/carbon-language/carbon-lang/blob/trunk/docs/project/goals.md)；[代码组织](https://github.com/carbon-language/carbon-lang/blob/trunk/docs/design/code_and_name_organization/README.md) | 目标：解析不用语义信息、支持分离/分布式编译；API 文件独立于实现编译，实现文件只能导入 API |
| Unison | — | [The big idea](https://www.unison-lang.org/docs/the-big-idea/) | 定义 = 语法树的哈希，依赖换成哈希；名字是旁存元数据，不进哈希；类型检查结果永久缓存、永不失效 |
| 链接器 | 2016–2026 | [lld](https://lld.llvm.org/)；[mold README](https://github.com/rui314/mold/blob/main/README.md)、[v2.42.1 说明](https://github.com/rui314/mold/releases/tag/v2.42.1)（2026-09）；Lattimore [Wild 增量设计](https://davidlattimore.github.io/posts/2024/11/19/designing-wilds-incremental-linking.html)（2024-11）；MSVC [/INCREMENTAL](https://learn.microsoft.com/en-us/cpp/build/reference/incremental-link-incrementally)；lld [COFF Driver.cpp](https://github.com/llvm/llvm-project/blob/main/lld/COFF/Driver.cpp) | mold：先把全量链接做到接近 `cp`，增量“不再紧迫”；Wild：“不移动未变的东西”，节内预留空间，失败退回全量，尚未实现；MSVC：填充 + 跳转 thunk；lld-link 接受 `/INCREMENTAL`，源码里只用于“import library 内容未变就不重写” |
| 回收 | — | rustc [persist/fs.rs](https://github.com/rust-lang/rust/blob/master/compiler/rustc_incremental/src/persist/fs.rs)；[Nix GC](https://nix.dev/manual/nix/2.28/package-management/garbage-collection)；Bazel [disk cache GC](https://bazel.build/remote/caching)（7.4）；[Rust 1.88](https://blog.rust-lang.org/2025/06/26/Rust-1.88.0/) | rustc：会话目录加锁，发布后删旧的，只留最新一份；Nix：从 profile 各代可达的保留，其余回收；Bazel：按大小/年龄空闲时回收；Cargo：`~/.cargo` 按访问时间（下载 3 月、本地 1 月） |

## 2. 关键证据

| 量 | 数字 | 来源 |
|---|---|---|
| rustc 2016 alpha，无改动重编 | syntex-syntax 全量的 22%，regex 16%；futures-rs（机器码少）只快 3% | Woerister 2016 |
| rustc 指纹代价 | “增量可能比非增量慢的主要原因”；每次会话整文件重写，占总时间“几个百分点” | dev-guide |
| 忽略 span 哈希的上限（check 基准） | style-servo −34.1%，tokio-webpush-simple −24.7%，clap-rs −24.4% | #47389 引 PR #56287（2018-11） |
| 相对 span 默认开启 | instructions 主基准无变化；次要基准 −1.5% | PR #115656 perf 结果 |
| rustc CGU 数 | 非增量默认 16，增量 256（“缓存更细”） | codegen-units 文档 |
| rust-analyzer 引入 durability 前 | 每次编辑 `src/lib.rs` 都要重验 std 相关查询，约 300 ms | matklad 2023 |
| Zig 0.14（`-fno-emit-bin`） | 50 万行首次 14 s，改动后重分析 63 ms | 0.14 notes |
| Zig 2026（x86_64-linux） | Fizzy 首次约 5 s，每次改动 50–70 ms；一次 37 ms 更新中 sema 1.2 ms、codegen 0.24 ms、链接 0.17 ms，约 30 ms 花在重算引用图（图并未变） | mlugg 2026 |
| Swift 跨模块粒度 | 模块 A 加一个顶层 `private func`，下游 1,114 个测试文件重编 1,110 个；改函数体、注释重编 0 个 | [swift #92617](https://github.com/swiftlang/swift/issues/92617)（2026-09，用户报告） |
| Swift 编译缓存键 | batch 作业键含主文件分批，分批随 `-j` 变，不同核数机器互不命中 | [swift-driver #2249](https://github.com/swiftlang/swift-driver/issues/2249)（2026-11，用户报告） |
| 调试信息占对象体积 | llvm-project `-O0 -g` 7.177 GB vs `-g0` 0.743 GB（约 90%） | Rastogi 2023 |
| 链接速度（Threadripper 7980X，debug） | Chromium 145：lld 16.64 s、wild 3.98 s、mold 1.65 s；中位数 mold 比 lld 快 4.9×、比 wild 快 1.9× | mold README（2026-08 测） |
| MSVC 增量链接的前提 | `/OPT:REF`、`/OPT:ICF`、`/ORDER`、增删 `.obj`、改链接选项都强制全量；产物含填充和 thunk | MSVC 文档 |
| Buck2 | 内部测试比 Buck1 快 2× | Buck2 博客 2023 |

## 3. 对 kfrc 的结论

| 做法 | 结论 | 理由 |
|---|---|---|
| (a) 键只含读过的东西、答案键 | 采纳（维持） | rustc/Salsa 自动记读、Zig 分“类型/值/布局”三种依赖、Buck2 dep files 都是同一原则；Swift #92617 说明粗粒度键（整模块指纹）代价极大 |
| (a′) early cutoff 必须基于中间结果哈希 | 采纳（明确写出） | à la carte：只记终端输入的深 trace 失去 early cutoff。kfrc 机器码键用 R 图哈希（中间结果）而非 CheckedBody/源码哈希，正确；需写成规则防止退化 |
| (a″) 负依赖 | 改编 | matklad 2026：Rust 的 trait 解析依赖“别处不存在冲突 impl”。`Resolve` 等孔位答案的计算需记录所查 trait 的 impl 集指纹，否则新增 impl 会漏失效 |
| (b) 源位置隔离 | 采纳，加边界 | 先例：Zig `update_line_number` + 声明内相对行号、rust-analyzer 不存位置、Unison 名字不进哈希、LLVM CAS 按函数切行表。反例/边界：rustc 因 span 同时携带 hygiene 而无法整体分离（#47389 中 bjorn3 2025）；Woerister 认为漏失效比多失效更糟；`line!()`/`file!()` 的值可被程序计算，是语义输入 |
| (c) S 图不存 | 采纳 | Zig 不缓存 AIR/MIR，只缓存每文件 ZIR 和输出中的机器码；dev-guide“不想把每个结果都存盘”。可再加“只存哈希”的验证型索引：S 图哈希 → 机器码，在 R 图变而 S 图不变时仍可截断 |
| (d) 内容寻址 + 根清单回收 | 采纳，改为两级 | REAPI 的 AC/CAS 两级让“键 → 结果清单 → 内容块”去重；Nix 根可达回收与“最近 N 次构建清单”同构；加 `salt` 式命名空间作废中毒结果；回收需与并发构建加锁（rustc 会话锁） |
| durability 分级 | 采纳 | sysroot 和 registry crate 极少变化；按 Salsa 分级可跳过整片验证（ra 的 300 ms） |
| 增量链接（原地打补丁） | 不采纳（首版） | 与 C6 可达性裁剪、答案键别名共享（相当于 ICF）冲突，MSVC 遇 `/OPT:REF`/`ICF` 即退回全量；mold 数据表明全量链接可接近拷贝速度；Zig 能做是因为链接器与编译器一体，可留作 dev profile 的后续项 |
| 输出内容未变不重写 | 采纳 | lld-link 对 import library 的做法；避免按 mtime 判断的外层工具级联重建 |

## 4. 具体改法（只建议）

1. [cache.md](../../design/build/cache.md)#源位置隔离：补三条——`#[track_caller]`/panic 的 `Location` 走 `loc.*` 旁表；`line!()`/`column!()`/`file!()` 展开的值是语义输入，进 CheckedBody 哈希；调试行表按片段切分，片段内行号相对定义起始行，起始行和文件名（先经 remap-path-prefix 规范化）进旁表。hygiene 不分离（与 [design/build/cache.md](../../design/build/cache.md)#键必须包含 一致）。
2. [cache.md](../../design/build/cache.md)#键只含读过的东西：加“early cutoff 只看中间结果哈希”规则；加 `Resolve` 等孔位答案的负依赖（trait 的 impl 集指纹）；加可选的 S 级截断索引（S 图哈希 → 机器码片段，只存哈希）。
3. [cache.md](../../design/build/cache.md)#存储形式：改成两级（键 → 结果清单 → 内容块），清单可整批用命名空间作废；回收 = 最近 N 次根清单可达 + 字节上限，进行中构建持锁，其产物视为根。
4. [design/build/cache.md](../../design/build/cache.md)#键必须包含：加反向清单“键不得含”：jobs/批次/CGU 划分、绝对路径、未声明的环境变量、时间戳（swift-driver #2249、sccache、Bazel `--action_env`）。
5. [design/build/pipeline.md](../../design/build/pipeline.md)#产物依赖：加 durability 两级——sysroot/registry crate 按（工具链哈希, lockfile 条目）一次验证；工作区 crate 逐项验证。
6. [design/build/cache.md](../../design/build/cache.md#失效测试)#缓存：加场景——文件首插一行注释（机器码片段应 100% 命中，只重写 `Location` 表和行表）；`line!()` 使用处失效；新增无关 impl 与新增冲突 impl；上游新增私有顶层项（下游不应失效，对照 Swift #92617）；jobs=1/8、不同检出目录得到相同键；故意漏记一个读的变异测试必须被抓到。
7. [cache.md](../../design/build/cache.md)#指标：加无改动构建的总耗时和“验证/哈希”耗时占比（rustc：指纹是增量变慢的主因；Zig：37 ms 中 30 ms 在重算未变的引用图）。
8. [compile-time.md](../../design/goals/compile-time.md)#关键路径：注明链接走确定性拼接 + 快速全量链接，不做原地增量；输出字节未变时不重写文件。

未找到原始来源：Woerister 2017 年专讲 red-green 的博客（2017 年材料只有 PR #44901、roadmap issue）；Swift interface hash 对注释/空白的处理（只有 #92617 用户报告“改注释重编 0 个”）；Xcode 26 compilation caching 的 Apple 官方文档（只有第三方文章，二手来源）；Swift 5.5 起跨模块增量（只有论坛帖，二手来源）；lld ELF 增量链接能力的官方说明；Carbon 增量编译的实测数字。
