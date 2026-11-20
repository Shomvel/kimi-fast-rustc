# 实施单元与优先级

每单元目标45–90分钟，完成一次集中静态排查后，用1–2个有判别力的测试入口验收。数量取完整调查首错分布；单函数后续错误、crate级优化和链接会继续影响实际解锁。优先级先看能否让整个crate真实通过，再看覆盖面和工作量。U1在ripgrep实际净解锁2个target units（log/walkdir），默认A/B为10/33→12/33，无回退；cargo中仅有U1首错的14个target units经定向默认A/B也全部从失败变为成功生成object。其余排序是证据约束的预期收益，仍须下一单元实际验收。

| 顺序 | 单元 | 修改边界 | 预期收益与排序理由 | 验收入口 |
|---|---|---|---|---|
| U1 | 基础原子+spin hint | MIR intrinsic adapter、crate私有helpers、Cranelift原子指令 | load/store/xadd/xsub/xchg/CAS/fence在多crate共用；只做load/store/xsub会留下log的CAS和pause。已纳入同一简单helper结构；已实际解锁ripgrep log/walkdir且整库运行一致；cargo定向14/14从失败变成功，包含once_cell/bytes/arc_swap/nix与gix子库 | 原生8线程RMW/CAS、release/acquire与seqcst对照；ripgrep默认逐crate A/B |
| U2 | syscall所需C variadic调用 | fn pointer/direct ABI对c_variadic的调用点支持，保留C variadic定义拒绝 | ripgrep memmap2在U1后只剩此类；已完成cargo首错中还有filetime、两种getrandom、memmap2、openssl-sys，共5个正式units候选（后续错误仍可能遮挡）；需要区分“调用变参函数”与“定义变参函数”，SysV整型参数调用可作窄单元 | 最小syscall/mmap对照；memmap2实际库构建和mmap读写 |
| U3 | scalar/zero-sized aggregate常量 | producer→Body/R/S常量构造，不弱化verify | ripgrep grep-searcher在U1后只剩常量；cargo已观察git2-curl/glob/gix-hash/icu_locale_core/unicode-bom共5个units候选；regex-syntax/标准容器也受益 | RefCell<Vec<_>>/空wrapper/嵌套常量差分；真实grep-searcher parse/search fixture |
| U4 | unreachable-control机器导出 | layout后CFG/schedule与machine exporter | cargo已观察gix-dir/gix-path/os-info共3个仅剩此类候选；多个库被同一“control node unreachable”挡住；先锁定单一最小模式，批量检查相邻边界 | 调查代表函数最小化→LLVM行为；对应真实crate默认构建 |
| U5 | 其余原子bitwise RMW | helper op与MIR映射，and/or/xor/nand/min/max按调查需要 | 复用U1，工作量较小；可能解锁仅有原子剩余的cargo依赖 | 多width/sign旧值与新值对照；受影响crate默认构建 |
| U6 | 一般union的最小storage子集 | Body/S语义storage、field reinterpret与layout | anyhow/bstr/memchr等共用；cargo已观察gix-discover/packetline/revision/worktree四个仅剩此类候选；先限定初始化字段→读同字段、全位初始化整数重读，padding/provenance保留 | 独立union初始化/覆盖/读写差分；memchr/anyhow代表函数 |
| U7 | SIMD值传输与vector ABI | S值表示、boundary transport、CLIF向量寄存器 | memchr/regex相关函数很多，但已有intrinsic scalar expansion不代表值流与ABI可用。先传输，后运算 | __m128i roundtrip/跨LLVM边界；单个真实memchr函数 |
| U8 | 实际需要的x86 SIMD intrinsic小组 | converter SIMD/LLVM intrinsic +CLIF | 在U7后按memchr/regex/cargo crypto使用频率分组，每单元3–5个同类intrinsic，保留target feature契约 | 对应std::arch scalar oracle；真实crate功能测试 |
| U9 | f32/f64标量ABI与纯传输 | ScalarAbi寄存器类、Body/S类型/常量、CLIF signatures和load/store | 为浮点运算铺路；这一步不承诺ryu/serde_json解锁 | LLVM caller↔kfrc callee双向参数/返回/stack超寄存器；bit roundtrip含NaN/-0 |
| U10 | 浮点基本运算与比较 | 独立float opcode/constant folding/CLIF | f64语义gate涉及广，但正确处理NaN/有序无序比较/负零需要明确契约 | scalar oracle覆盖边界与随机位样本；textwrap真实路径 |
| U11 | 浮点转换与最小libm小组 | cast（饱和/unchecked分开）、runtime symbol映射 | ryu/serde_json仍需u128与转换；不能把ABI完成记为全支持 | LLVM差分边界/NaN/溢出；相关单crate默认构建 |
| U12 | i128的ABI与传输 | ScalarAbi/two-lane lowering、常量/load/store | u128 gate高频，传输与运算分别验收；先禁止未实现运算 | LLVM双向调用、位模式roundtrip；调查代表函数 |
| U13 | i128基础运算小组 | carry add/sub、比较、shift；除法另单元 | 为整数formatting、hash/typeid和浮点格式化依赖铺路 | LLVM生成式边界/随机对拍；被解锁crate真实构建 |
| U14 | DST tail与size_of_val | producer动态大小/对齐事实、ArcInner尾部计算 | Arc<dyn Trait>/Arc<[T]>/Arc<str>均受影响，已支持virtual call不能替代DST布局 | Arc/Weak clone/drop对照计数；混合对齐的动态tail |
| U15 | TLS语义路径 | 固定effect ThreadLocalAddress的S生产、verify与layout保留 | 机器层已有ELF GD支持，但S builder拒绝；RandomState与cargo上下文依赖需要 | 原生多线程TLS隔离/析构；RandomState真实调用 |
| U16 | enum/Phi poison修复 | lower_layout/values与variant_lanes目的节点形态 | 基线已发现4个regex-automata函数panic；单元先最小化，再修所有同类写入，不以容错或set_input扩容代替语义修复 | 最小nested enum/Phi LLVM对拍；regex-automata同4函数无panic |
| U17 | 非unit main入口 | shell_entry Start/Termination contract | 最终rg返回ExitCode；依赖通过后仍需此步骤。支持Result还涉及report与stderr | unit/ExitCode/Result程序退出码与输出；最终rg真实命令 |
| U18 | 余下LLVM intrinsic/inline asm小组 | 对实际cpuid/xgetbv等建立目标实现或受限asm支持 | cargo crypto/feature detection会触及，不能跳过CPU探测假设所有feature存在 | rustc/std原生实现对照；实际依赖集成 |
| U19 | catch_unwind在abort配置下的契约 | MIR intrinsic与运行时状态 | 即使panic=abort也会实例化线程入口catch_unwind；需要按该配置定义语义，不默认宣称完整unwind | 线程正常返回/abort行为与LLVM一致；真实线程负载 |

U6–U19是每个子集的入口单元，不承诺一单元完成全部类型能力。进入下一单元前重新统计首错与默认构建结果；一个类别被移除后，新出现的错误应归入后续缺口，不能当作已完成能力的性能回退。

## 新发现：默认优化的规模问题

独立Linux调查在gix的`Result<FullName, ...>::map_err<clone::fetch::Error, ...>`函数上花费超过20分钟。低频gdb采样定位到kfrc自定义的`third_party/cranelift-codegen/src/machinst/copy_cleanup.rs:Equalities::copy -> Copies::equalities -> run`；CPU持续占用，尚未量化总运行时间或证实不收敛。`machinst/compile.rs:78`在`opt_level=none`时明确跳过该优化，已有`KFRC_CRANELIFT_OPT=none`开关可用于独立诊断。后续应将该优化的工作量/收敛预算与数据结构作为一个45–90分钟单元：保留原分配输出作为可验证的保守路径，核对同类正向must analysis、join placement、coalescing的复杂度。用实际慢函数、原copy-cleanup生成式测试和正常小函数对照，确保编译进展与行为一致。能力首错调查与默认speed模式的性能限制分开统计。
