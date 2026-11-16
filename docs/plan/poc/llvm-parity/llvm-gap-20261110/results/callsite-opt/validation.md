# 验证过程与测量边界

本轮新建任务沙箱 `imj3ukm7eqxdk2x0im3uh`，未重连或回收其他任务的实例。HK 原记录的 0820 入口已不存在，实际核对了同目录的 codex_gpt56sol canary，仍调用 code-harbor entrypoint，environment 为 E2B。HK SDK 2.37.1，本地兼容 SDK 2.34.0；凭据只进入客户端参数。实际使用 Rust 模板 `r6g8s7tmv7hjdgpouxk9`，8 vCPU、16 GiB，安装并核对 Rust 1.94.1 的 rustc-dev、rust-src、llvm-tools。

公开仓库归档缺少 ignored 的五个 `.crate` 输入。首次 suite prepare 失败后停止重跑，按 suite.json 完整列出所需 archive，从固定版本地址下载，逐一核对 SHA-256 后恢复。

W4 的整数子范围测试先在基线失败，旧实现不能消去符号不同或范围不同的私有 load。实现后六项 private-memory 回归通过。真实 Rust backend fixture 的 10,006 个输入逐项断言并比较 LLVM 输出，checksum 为 `2733a14bdc619b28`。首版整字节重建从 30 条增至 36 条指令，随后补充纯整数位片段合并；新增重建测试先失败，修复后 semantic_instcombine 14 项通过，原生重建收敛到两条指令。

W5 的新增 pointer native 测试首次把 physical Load 放进 Specialized 图，违反层级契约。静态核对全部新增内存节点后改为 LoadValue，继续经过 verify → layout lowering → verify → machine lowering → object → C 链接执行；九项 compare_branches 测试通过。此修正只涉及测试 fixture。

W3 首次定向构建发现 TypeOracle trait import 缺失，修复后 semantic_inline 29 项通过。静态检查又发现常量裁枝可能把所有 Return 删除，既有 splice 尚不能处理该形态；新增回归实际复现 `WrongArity`，补上保守回退后 31 项通过。同批恢复无法准备的调用在 eligibility 扫描前的廉价预算判断，保留已缓存的 free 候选语义。

all-v1 的 workspace 为 899 passed、1 ignored；补上 W3 noreturn 与预算用例后的 allv2 为 901 passed、1 ignored。allv2 的 enum、integer、private-integer-lanes 原生差分、四库 corpus 和 13 项 checksum 均通过。allv2 与 all-v1 的四库所有 object 和最终 driver 逐字节相同，继承关系有独立回执。allv2 后端 SHA-256 为 `2e6e4f2ca6089df41263e33a45c29e303a10f2751b1909b1b51a5d5ddcc284a3`，源码 overlay 为 `880a98c5414d5161b754f0e0737542b32258421c0531c12429e3ba31c1454519`。

allv2 在固定 CPU7 的 21 轮矩阵中，timestamp parse / baseline 为 0.642718，95% 区间 [0.637491, 0.646803]；semver requirement 为 1.049305 [1.046611, 1.052282]，同 inode 控制组为 1.001033 [0.994450, 1.002016]，因此该候选没有作为最终结果验收。四库编译 wall 几何平均为 0.996252，各库区间跨 1；编译指令分别增加约 1.35%、0.87%、0.31%、0.62%。这些数据属于 allv2，不充作后续组合的测量。

去掉 W5 的 core 仅保留 W3v2 与 W4b，后端为 `5d41543e2372a7bad428b2b549b8e6d9e47f8b987d1629aaa1cf8e9ea3e26278`。两库七轮筛选中，requirement / baseline 为 1.005412，控制组为 1.004380；timestamp parse 为 0.640333。不过 duration format 为 1.057012 [1.037668, 1.073647]，仍需布局诊断，不能将 core 的此轮状态写作全部验收通过。

初版静态 inventory 用符号名作字典键，会覆盖不同 CGU 的同名 local 函数。semver 的物理函数有 171 个，初版只保留 144 个不同符号名；其中 into_searcher 在最终链接文件中有多个地址。已停止使用这份 name-only inventory 作物理布局根因证据，保留原文件并补物理索引。整对象、整 executable 的 SHA 身份比较不受此索引问题影响；W3 与基线的 semver 最终二进制逐字节相同这一事实仍成立。

按物理函数校准的诊断和最终选定候选的验收将在完成后单独追加。各轮原始样本不混池，初筛、控制实验与正式测量均保留各自标签、后端及源码指纹。
