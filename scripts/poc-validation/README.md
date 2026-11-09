# Linux PoC 验证

在 x86_64 Linux、Rust 1.94.1（`rustc-dev`、`rust-src`、`llvm-tools`）上运行：

```sh
bash scripts/poc-validation/run-linux.sh "$PWD" /absolute/result/directory all
```

`all` 依次构建后端、运行 workspace 测试、固定和生成式标量差分、Cargo
按包选择后端、真实可执行入口与 Vec 分配/扩容/drop、内存/聚合/enum/checked
arithmetic/常量与 panic 差分，以及 semver 原版集成测试。失败立即停止。
每步保留命令输出和真实退出码；单测串行执行，避免旧
FakeRustc 测试并发创建/启动临时脚本时的 `ETXTBSY` 竞争。也可将最后一个参数改为
`build`、`unit`、`scalar`、`cargo`、`entry`、`memory`、`semver` 单独复现对应阶段。
`regex-smoke` 是独立的 regex-syntax 公共 API 语料差分，只在 semver 通过后运行。
它不属于 `all`，也不能替代尚未执行的 regex-syntax 上游内部单元测试。

`integer-intrinsics.sh <backend.so> <fresh-result-directory>` 可独立编译原 P1
的 bitops/saturation 模块，复用原 driver 的检查函数，验证 500 个 rotate/funnel
边界输出及 131872 对饱和运算输入。它保留原模块源码并使用 LLVM driver 的 i128
数学参考，结果独立记录，不能代替仍有未通过用例的完整 P1 阶段。

`selection-intrinsics.sh <backend.so> <fresh-result-directory>` 原样导入 P1 的
`select.rs`，提取原 driver 的 `selection_probes` 及全部断言，单独核对 54 行
选择结果、聚合复制、指针、参数副作用和 Drop 计数。输入及生成 wrapper 都留存
哈希；结果与完整 P1 分开记录。

`tls-elf.py <completed-memory-result-directory>` 在 P1 行为差分通过后，核对
四个自有 raw TLS 静态变量：它们必须定义于 `FILE kfrc` 目标文件的 `SHF_TLS`
段，具有 `STT_TLS` 类型及各自的 `R_X86_64_TPOFF32` 引用；最终 driver 必须
保留同名 TLS 定义和 `PT_TLS`。原始 readelf、objdump 与 JSON 结果写入
`tls-elf/`。该检查对应 Linux x86-64 可执行程序的 local-exec 模型；行为验证
仍由 P1 的三线程初始化、读写隔离、持久状态及析构检查完成。

所有 PoC 结果的链路是 **rustc 前端 + kfrc 中后端 + rustc std**。
标量和内存测试的 driver 使用 LLVM，待测 library 使用 kfrc，并与同工具链的
LLVM library 比较。入口测试的可执行文件全部由 kfrc 编译后交 rustc 链接。
Cargo 测试通过 Cargo profile 的 `codegen-backend` 按包选择实际 backend；
通过读取目标文件 `FILE kfrc` 记录验证来源。

`snapshot.py <output>` 只打包显式白名单的源码、设计文档、examples、测试和验证脚本，记录
commit、当前补丁、每文件 SHA-256 和归档哈希。`sandbox_runner.py` 在 HK 主机
通过现有 SDK 管理本任务独立沙箱：凭证仅进入 SDK 选项；上传后核对哈希；命令
日志和退出码落盘；短查询不缩短租约。它不会启动 SWE benchmark，也不会回收
其他任务的实例。

本轮验证证据在 `evidence/poc-20261109/`；实际环境、源码快照、命令、结果与
沙箱释放状态以该目录中的记录为准。

## 语义值图迁移

`run-linux.sh <repo> <results> semantic` 运行
`crates/rustc_codegen_kfrc/tests/semantic-ir/run.sh`。同一份库源码在 LLVM、
唯一语义链关闭优化、唯一语义链开启优化三臂串行编译运行；
每臂使用相同 LLVM ABI provider 和 driver，核对预期值、完整输出、对象来源及
每个函数的 S 输入/L 输出 dump。覆盖嵌套字段、快照、聚合 Phi、数组、调用，
以及 Pair/Indirect 值双向跨后端传递、借用、动态下标、动态 enum 与 niche。
union/TLS/SIMD 范围外输入以整 crate 明确拒绝验收。该阶段不隐式加入旧 `all`，也不把新切片通过计作 semver 或完整 R/S 通过。
首片历史证据目录为 `evidence/poc-semantic-20261109/`；删除旧 IR 后的独立
验收记录在 `evidence/poc-semantic-only-20261109/`。

`walkthroughs.py --inputs <docs/research/walkthroughs/scripts> --backend <backend.so>
--out <fresh-directory>` 保持 W01-W10 原程序不变，以 LLVM、语义优化关闭、
语义优化开启三臂执行差分并保存逐函数 S/L 操作计数。W04 仅验证 panic=abort，
W08 仅验证基线多 crate 行为，不将其写成增量缓存验收。遇到首个错误立即停止。
当前 pipeline 未记录每 pass 节点访问次数，报告对此保留空值，不拿 rewrite/node
数量替代访问量。

`build-shared-std.sh <fresh-directory>` 以 LLVM O3、CGU16、panic=abort 和
`-Zbuild-std=std,panic_abort -Zmir-opt-level=0` 构建可供两臂共用的独立 sysroot。
`semver-perf.py --sysroot <shared-sysroot>` 同时为library和LLVM driver固定此sysroot、
记录所有库hash，保留LLVM O3/CGU16/ThinLocal口径。省略此参数时仍用预编译std，
属于旧诊断口径。该脚本记录墙钟、RSS、运行时与text，不包含LLVM中后端硬件指令。

`run-linux.sh <repo> <results> cli` 经公开 `kfrc native` 命令编译真实 Rust
库和可执行文件，验证借用/字段行为、跨后端 driver、S 图标记以及库与最终 ELF
中的 `FILE kfrc` 来源。

`legacy-corpus-run.py --corpus <generated-suites> --backend <backend.so>
--out <fresh-directory>` 消费 `legacy-corpus.py` 的六组原测试行为映射，逐组执行
LLVM/语义关闭优化/语义开启优化三臂。成功输入同时比较预期输出和 LLVM；
陷阱输入独立进程运行，要求非零退出且不超时，允许 panic abort 和硬件信号不同。

`feature-probes.py` 的真实 assume 用例同时经过 `std::hint::assert_unchecked`
和 `std::intrinsics::assume`。只执行满足 unsafe 契约的输入，三臂对拍明确输出；
额外检查 R/S 源图保留两路 Assume、发布 L 不保留要求和凭据节点。false/poison
行为由源解释器回归测试验收，不把执行 native UB 当作输出一致性证明。

`machine-attribution.py --disassembly <llvm-objdump-intel.txt> --symbol
'<exact demangled symbol>' --text-bytes <ELF symbol size> --out <receipt.json>`
统计同一函数的静态机器指令、mov、分支和显式内存读写。它核对解码总字节与
ELF 符号大小一致，排除符号后的对齐字节；未知内存指令直接报错。计数不包含
call/ret/push 的隐式栈访问，含 rbp 内存操作数的 mov 仅是栈帧代理，不证明是
寄存器溢出。这里的静态机器指令与编译过程的硬件退休指令计数分别报告。
