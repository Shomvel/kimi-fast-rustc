# 集中优化：实现与最终验收

本轮完成地址公共表达式消除、L层布尔事实消费、u64 checked结果共享、小型product边界表示和自然循环入口16 B对齐。以接手时的ranges工作树为start基线；此前未提交改动属于基线，不重复计算收益。接手基线提交为 `abc59a0`，本轮集中优化提交为 `add2206`；提交保存在本地，尚未推送或合并PR。

## 最终实现

- **地址与条件清理**：仅合并同SSA来源、类型、偏移和语义标志的纯PtrOffset；L阶段对已知bool折叠分支，保留固定操作与其他用户。base64 encode重复查表基址消失。
- **checked算术**：u64 MulOverflow显式输出低位乘积，在支配关系允许时复用乘法/加法结果；overflow先成为布尔SSA，不让flags跨指令保存。signed和窄整数维持原路径。
- **边界表示**：含1–2个直接标量字段的product继续用正常标量解码，减少&str快照搬运；解码仍在固定点捕获值，不借用会被修改的ABI输入。
- **循环布局**：从已有scheduler支配分析取得自然循环头，最多15 B多字节NOP对齐。分支长度和padding联合计算；labels、calls、PC-relative和TLS重定位统一更新。后置critical-edge搬运块不再误认为循环。

最终源码覆盖文件为`/Users/shom/Downloads/kfrc-focused-opt-20261110/accepted-source.json`，19文件SHA见[accepted-manifest.json](accepted-manifest.json)。最终后端指纹与验证方法见[验收摘要](measurement-summary-accepted.json)。

## 运行结果

同一真实E2B内串行交错21轮，工作量为套件默认4倍；区间为逐round配对比值中位数的4000次bootstrap，只描述本次实验内部波动。负数表示耗时下降。公共driver及部分泛型/std由LLVM编译，结果是四库后端差分，不是全程序KFRC替换。

| 负载 | 耗时变化 | 95%区间 | 候选 / LLVM |
|---|---:|---|---:|
| semver / parse | -3.84% | [-4.81%, -2.92%] | 3.53× |
| semver / req | -2.64% | [-3.57%, -2.17%] | 6.67× |
| semver / identifier | -0.40% | [-1.26%, +0.08%] | 1.69× |
| base64 / encode | -3.59% | [-3.76%, -3.44%] | 7.92× |
| base64 / decode | -0.84% | [-1.37%, -0.67%] | 6.75× |
| base64 / short | -3.61% | [-4.44%, -3.14%] | 3.65× |
| unicode-segmentation / graphemes-mixed | -0.87% | [-2.29%, +0.86%] | 1.33× |
| unicode-segmentation / graphemes-ascii | +6.76% | [-0.02%, +14.53%] | 1.14× |
| unicode-segmentation / graphemes-rev | -0.96% | [-3.21%, +1.19%] | 3.72× |
| humantime / dur-parse | -1.36% | [-3.26%, +4.63%] | 8.39× |
| humantime / dur-format | -2.68% | [-4.16%, -1.43%] | 1.13× |
| humantime / ts-parse | +0.55% | [+0.44%, +0.99%] | 2.95× |
| humantime / ts-format | -5.01% | [-5.46%, -4.63%] | 1.63× |

上表保留完整13负载原始矩阵，未悄悄替换其中的ASCII样本。ASCII该轮波动较大、区间跨1；热路径机器码/地址及动态指令核查后，另做21对独立确认，natural/start为**1.00037 [0.99016,1.01687]**，未复现稳定回退。原完整矩阵的等权几何平均为0.98539，受该波动影响；不把两次实验样本混池，也不声称跨机器普遍加速。

最后的自然循环识别修复与selected候选的四库全部object和最终driver逐字节一致，因此继承这12项已有运行证据；ASCII独立确认另列于最终摘要。identifier此前稳定回退已由仅改变布局的诊断实验定位，并在最终21轮恢复至基线；不得把自然循环识别修复说成ASCII加速原因。详见[回退诊断](regression.md)及[Unicode静态证据](static-unicode.md)。

## 编译成本与产物

最终natural后端重新完成每库21对完整编译，并确认每轮产物与相应参考object哈希一致。wall几何平均0.99428，约下降0.57%，整体近似持平；多数区间较宽，不宣称大幅编译加速，更未达到历史50%目标。Unicode/humantime的wall区间上界超过2%，仍保留该不确定性；没有稳定的>2%回退证据。

| crate | 完整编译wall变化 | 95%区间 | 编译指令变化 |
|---|---:|---|---:|
| semver | -1.53% | [-4.82%, +1.15%] | +0.05% |
| base64 | -1.87% | [-5.55%, -0.01%] | -1.62% |
| unicode-segmentation | +1.18% | [-1.55%, +3.07%] | +0.05% |
| humantime | -0.04% | [-3.66%, +3.90%] | -0.03% |

最终静态模式（包括新增对齐NOP）：

| 函数 | start | 最终 |
|---|---|---|
| base64 internal_encode | 3595 B / 855指令 / 360 B显式栈预留 | 2279 B / 622指令 / 72 B显式栈预留 |
| semver numeric_identifier | 729 B / 215指令 | 719 B / 212指令；重复imul消失，保留一次mul |
| humantime UTF-8 expect helper | 136 B / 43指令 / 48 B显式栈预留 | 114 B / 36指令 / 32 B显式栈预留 |

符号内指令数含NOP，显式sub rsp不是完整frame。最终四库.text相对start依次为+9 / −1324 / +183 / −603 B（semver/base64/Unicode/humantime）；object为+8 / −1296 / +128 / −1360 B。体积与运行时间分别验收，不能由静态缩小直接推导速度。

## 正确性与可复查证据

- 最终Linux workspace：**883通过、1忽略**。
- 四库corpus与LLVM一致，13项workload checksum一致。
- upstream integration：semver34、base6413、Unicode8通过。完整upstream unit仍受既有fabsf64、atomic_xsub、atomic_load未实现阻塞，未宣称全套通过。
- 原生checked算术覆盖边界、10000随机输入、跨循环与兄弟分支；新增布局测试覆盖short/near边界、padding、call/地址/TLS重定位、真实循环及非循环回跳。
- enum/UTF-8边界与整数差分fixture通过；语义正确性、机器码与性能分别留证据。

[validation.md](validation.md)记录最终指纹与测试；[evidence-receipt.json](evidence-receipt.json)记录两份归档及逐文件核验、沙箱释放状态。完整日志/objects/backend保存在`/Users/shom/Downloads/kfrc-focused-opt-20261110`。

后续最大的运行缺口仍是固定切片长度的调用点特化、小数组搬运及字段转发；本轮没有通过全局提高内联预算换取收益。剩余总体LLVM差距仍约3倍，不能把这批局部改善写成已追平LLVM。
