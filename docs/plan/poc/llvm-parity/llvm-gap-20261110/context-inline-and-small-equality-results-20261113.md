# 2026-11-13 优化收尾

本轮沿 LLVM parity review 的“先简化再定价”方向，验证了调用点常量长度准备；该实验没有改变 humantime 终码，已从工作树撤出，完整冻结源码和原始验证产物保留。随后只保留一条独立的 x86 小比较规则：通过范围内的重叠尾读取减少加载段数，目前默认关闭。

调用点内联实验 `candidate-context-inline-v3`：

- 18 项单元测试、24 项内联语义测试在开关关闭/开启时分别通过；真实原生差分 unset/off/on 每态 169,328 项，输出及输入 transcript 一致。
- 七种长度的 `equal` 从 60 节点变为六个 37 节点、一个 48 节点，仍超过现有 free24。humantime 的13个对象、可执行文件及 LLVM caller 在四态全部逐字节相同；未进行性能测量。
- 实验生产源码和其专用测试已撤出当前工作树；保留本轮新增的原生差分用例供独立后端规则验证。撤出前逐文件核验独占改动，未触碰原有用户/其他轮次改动。
- 实际归因与7张prepared图保存于 `research/context-inline-v3-attribution/attribution.md`；最终Unit仍为2028字节并保留16个equal重定位。
- 完整原始产物499成员全部哈希核验；root 重新检查测试输出、真实测试二进制、三态原生 transcript、四态对象与可执行文件，见 `validation/root-context-outcome-check.json`。源码树 `61a4673f0178f91963249f6c9085c1ddcac34e01f95d53ffd144c1692141b216`。

独立后端规则 `candidate-small-equality-overlap-v2`：

- 只处理已认证 Bcmp、常量长度≤32；精确读取范围和原 effect 顺序保持。仅当段数严格减少时选重叠尾部，7字节从4+2+1变为4@0+4@3；15字节从8+4+2+1变为8@0+8@7。同段数沿用旧计划。
- `KFRC_SMALL_EQUALITY_OVERLAP=1` 显式开启，默认关闭；尚未进行性能计时，不将少加载段数声称为耗时收益。
- 源码树 `03413eb9b3a606ba9bc6f113f1d53298bd73c4774f6201fc98b9687ec3991096`，1390文件，相对上一轮已验收基线只有3个后端源码/测试文件及3个原生差分文件改动。
- 定向远端验证通过：helper2项、small_copies关闭/开启各8项，共18项；真实原生LLVM/kfrc对拍关闭/开启各169,328项，输入transcript与输出精确一致。
- 四库5种构建模式的corpus与13负载校验和通过；unset/0的全部50对象和4个可执行文件精确等基线，LLVM caller对象恒等；开启后的单线程/诊断产物精确等于普通开启模式。
- 开启后仅humantime的1个对象变化，库text为36,557→36,517字节（少40字节），其余三库对象不变。这是确定性代码体积结果；没有性能计时，也未重跑完整workspace/旧17native套件。
- 后端二进制SHA256：`4f02f8ee6174e770be21be43a78bd42626e357067428879b7460d3c90fa2e41f`。
- 完整原始归档已下载，2,075 个成员的大小与 SHA256 全部核验。归档 SHA256 为 `800032b44ef203e5814885ee0ed24bffdcf6a19b6a20188b0b26bb9f1b720b1f`；主 agent 从原始产物独立复核测试二进制、原生 transcript、各模式对象及 text 差值，见 `validation/root-overlap-outcome-check.json`。
- 沙箱已释放，API 返回 `SandboxNotFoundException` 确认消失。最终回执为 `validation/overlap-stage1-final-receipt.json`，释放证明为 `validation/release-receipt.json`；无待下载产物或运行中沙箱。

本轮验证中的两类问题均保留原始证据：新 S 图测试误用 Layout 解释器入口，已修正共享测试 helper 并加明确成功返回值/调用轨迹断言；归一化 tar mtime 与共享 Cargo target 导致旧二进制复用，后续改为每个执行使用最初不存在的独立 source/target/backend/result，归档真实测试二进制。旧的无效执行未算通过。集成测试缺失类型 import 也已在冻结实验中修复。

未提交或推送代码。用户要求收尾后不再开新实验、正式计时或完整性能测评。
