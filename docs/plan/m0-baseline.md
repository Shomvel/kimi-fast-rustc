# M0：版本、范围与基线

**目标：** 把输入、工具链和已知错误固定下来，让后续每一步都能从干净环境复现。

**出口：**

- 源码、工具链、rustc 参考版本和目标项目都有固定身份（commit、哈希、`rustc -Vv`）。
- 已知 if 错编译能从冻结源码重新构建后复现（或记录不复现的差异）。
- 一个成功用例和一个预期拒绝用例有完整记录。
- 共享 ID/类型/Body/SoN/ABI 契约冻结。

**通过后仍不代表：** 任何新语言能力；历史测试数就是本次通过数。

正式参考版本未固定前，不开始依赖 rustc 内部细节的大规模移植。版本选择规则见 [sources/README.md](m0/t00-freeze-inputs.md)。

---

<a id="t00"></a>
## T00 固定版本、来源和能力清单

`M0 · I · 依赖：无`

细化：[m0/t00-freeze-inputs.md](m0/t00-freeze-inputs.md)

- **做什么：** 构建 manifest 和 `docs/plan/inputs/provenance/` 来源记录；固定官方 rustc 源码、oracle 二进制、sysroot、目标项目 lockfile、edition、target 和配置；写出支持能力矩阵。本地派生 rustc 与官方实现的差异单列。
- **完成标志：** 另一个人在独立环境能取得全部输入；版本与目标项目 MSRV 相符；没有用本地 commit 冒充官方 tag。

<a id="t01"></a>
## T01 从干净源码复现已知错编译

`M0 · E · 依赖：T00`

细化：[m0/t01-reproduce-miscompile.md](m0/t01-reproduce-miscompile.md)

- **做什么：** 冻结构建 [有状态 if 回归](m0/t01-reproduce-miscompile.md)，不复用旧 debug 二进制；同时列出 if/match/短路/loop 中同类风险点。
- **完成标志：** 参考与候选的完整命令、ELF 哈希和真实执行输出；若不复现，保存差异定位。

<a id="t02"></a>
## T02 冻结共享数据与阶段契约

`M0 · I · 依赖：T00`

细化：[plan/m0/t02-shared-contracts.md](m0/t02-shared-contracts.md)

- **做什么：** 字段级定义 [共享 ID](../design/ids.md)、类型驻留、Place/Typed Body、InstanceKey、TargetSpec/FnAbi、SoN 值/控制/效果、产物发布和错误分类。未支持的类型和效果要能表示为“拒绝”。
- **完成标志：** 同一个例子能在 AST → Body → 实例 → SoN → Machine 各层追踪身份和类型；各角色确认输入输出；不存在两套 TypeId 或函数编号。

<a id="t03"></a>
## T03 源级差分与证据 runner

`M0 · E · 依赖：T00, T01`

细化：[m0/t03-differential-runner.md](m0/t03-differential-runner.md)

- **做什么：** `tests/` 下的 fixture 和拟建 `tools/` runner，支持：接受、拒绝、真实 x86 执行、结果比较、证据保存。协议见 [差分测试](README.md#验收)。
- **范围：** 只做源级 runner。逐 pass 的 IR 解释器差分依赖解释器本身，接入放在 T09（骨架来自 T07）和 T17（首版），不作为本任务的完成条件。
- **归因实验：** runner 支持 [M0 归因实验](../research/runtime-profile/m0-experiment.md) 的臂与 workload，阈值在拿到数据前冻结。
- **完成标志：** 成功、拒绝、T01 已知失败三类样例可在干净目录重放；故意给错结果时 runner 判失败；超时、跳过、unsupported 不算成功。
