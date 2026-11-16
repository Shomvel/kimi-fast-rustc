# Decode 调用边界与私有复制优化

2026-11-11。本轮通用编译器优化使 base64 decode 耗时下降 **63.30%**，short 下降 **18.24%**；四库13项等权配对中位比的几何平均下降 **8.87%**。代价是四库 full 编译墙钟增加 **4.94%**，humantime 时间戳解析增加 **6.17%**。实现和完整验证保存在独立 worktree，尚未替换正式性能基线；全套运行耗时仍为 LLVM 的约 **2.52倍**。

基准是 exact accepted18r2，source tree `99032438e4f499480f41dbd25e87ff4ac3de39a4a07aa11a584a3c61c0ad0a08`，backend `47a06f80c001a20a6bb00dba6451d6410b93501d0626529c5e2b9047a6262f97`。库源码、输入和工作负载不变。

## 实现

- rustc 的 `inline(always)` 属性传入单次共享内联会话，优先处理这些调用。只豁免 ordinary Size/Cost/UnknownCost；合法性、递归、noinline、深度、调用和数据身份守卫保留。
- 参数准备复用原有 fuel，读取带 witness 的已选 variant payload 中的标量事实。原参数和原全值观察保留。
- 已观察值的条件简化复用同一 CFG/dom 证明，仅替换当前 If 的使用。第一次 RequireDefined、循环动态实例与 variant witness 保护保留。
- 编译入口的共享 max_nodes 从2000改为4096。这同时扩大图分配、准备 clone、成本分析及相关访问预算；不是只增加最终输出节点。其他预算不变，不重置账户。
- Layout cleanup 对1至8字节的已初始化私有普通整数复制查询现有 writer/Forward，在原执行点写准确字节，随后消除临时槽。部分初始化、寿命重启、地址逃逸、未知写入、限制值与无法证明的来源保留原复制。

这是 issue #11 中“保留事实并让下游消费”的具体衔接：无需先移植完整 LLVM SROA 或再从机器码恢复对象关系。实现按 IR 形态工作，没有 base64 符号特判。

## 已验证结果

不含私有复制增量的 closure 候选已通过7202组 LLVM/parent/candidate 完整输出差分、原corpus，以及正常构建和唯一Observe的14个完整对象字节对齐。原driver、CPU7、排除1组预热后9组 AB/BA：耗时比中位数为 **0.5088794617**，即耗时减少 **49.1121%**、约1.965倍速度。

该结果是原库不变的编译器收益。源码表达上限探针曾达到63.77%，仅用于定位机会，不计为此实现收益。此前只展开两处的 inline-always-v2 实测慢30.37%，已拒绝。

私有复制组合使用两套全新 target 构建，43项定向与相邻回归全部通过。正常构建与唯一Observe的14个完整对象相同；decode主循环四个8字节临时槽和四个Layout memcpy均消除，直接写准确6字节输出。

该组合已通过7202组完整buffer差分与原corpus。原driver decode10000、CPU7、排除1组预热后9组三臂轮转：组合/accepted18r2配对耗时比中位数 **0.3691747681**，即耗时减少 **63.0825%**、约 **2.709倍**速度；组合/closure比值 **0.7270486578**，即消除临时复制的这一步额外减少耗时 **27.2951%**。30/30执行退出0且checksum42229339，没有为越过50%阈值重复该screen。

候选源tree `9235520b2dd380f635c5c51d76baa2c890b9c84b61d2de9f10841dafeef495a3`，backend `6c25173e5b00a7056f891748beaf80294fba00e71fd3a4de15e9d81a0616521c`。全workspace回归已实际执行1137项通过、0失败、1项原有ignored计时测试。四库13项与编译成本已完成筛选，结果见下表；该候选保留在独立worktree，未替换既有性能选择或推送。

## 范围和验收

七组原生LLVM差分（私有整数lane、enum边界、整数intrinsic、通用内存、通用标量、byte DSE、induction）以及四库检查器/corpus/13项checksum均通过。四库13项逐项运行时间、full与metadata编译成本和代码体积均已测量。单项decode减少约一半不代表整套减少一半，也不代表已对齐LLVM。单项回退和编译成本必须单列。

base64上游unit harness已有缺失intrinsic `fabsf64` 的阻断不能写成测试通过。首次组合验证还曾因tar mtime0与共享target误复用旧testbinary，实际执行0个新测试；验证器停止全链并封存，随后改用全新workspace/backend targets恢复。性能样本来自实际构建且指纹绑定的产物。

完整证据保存在本机 `/Users/shom/Downloads/kfrc-callsite-opt-20261110/generic-parity-20261110`；冻结源与补丁在其父目录的 `decode-private-copy-v1-frozen`。完整原始结果与验封回执随本报告保留。

## 四库完整筛选

同一E2B、CPU7、accepted18r2/候选/LLVM三方案，每项排除1轮预热后测9轮。390条记录中351条为正式测量；每条保留原checksum。下表为同轮候选/基准耗时比的中位数，小于1表示耗时减少。

| 库 / workload | 候选 / accepted18r2 | 候选 / LLVM |
|---|---:|---:|
| semver / identifier | 0.991594 | 1.920394 |
| semver / parse | 1.002006 | 3.670685 |
| semver / req | 1.001515 | 6.635673 |
| base64 / decode | 0.366991 | 2.412637 |
| base64 / encode | 1.002545 | 4.887780 |
| base64 / short | 0.817639 | 2.468989 |
| unicode-segmentation / graphemes-ascii | 1.004063 | 1.072470 |
| unicode-segmentation / graphemes-mixed | 1.004878 | 1.328627 |
| unicode-segmentation / graphemes-rev | 0.990973 | 3.181115 |
| humantime / dur-format | 0.966077 | 1.154645 |
| humantime / dur-parse | 0.979427 | 7.780514 |
| humantime / ts-format | 0.994587 | 1.628222 |
| humantime / ts-parse | 1.061661 | 1.796979 |

13项等权配对中位比几何平均为 **0.911321601**，即耗时下降 **8.87%**。对LLVM的同口径比值为 **2.516440**；全套尚未对齐LLVM。decode在本次全局窗口减少 **63.30%**，short减少 **18.24%**。

时间戳解析 `ts-parse` 增加 **6.17%**，九组比值均大于1（约1.040至1.072）。这是本轮明确代价，不能用总均值掩盖。其余接近1的小差异不作稳定收益或无回退承诺。本轮不为改善该项追加微调或计时。

正常objects的text：semver 30829→30780B，base64 23597→25723B，unicode 18520→18210B，humantime 49835→46959B；合计122781→121672B（减少1109B，约0.90%），base64单库增加2126B（约9.01%）。编译成本见下节；完整对象对齐和归档回执在最终证据中列明。


## 编译成本与选择

编译与运行分别执行。同一沙箱8CPU、原flags，accepted18r2/候选/LLVM三方案，四库full与metadata各排除1轮预热后测9轮。每个值为同轮候选/accepted18r2比值的中位数。

| 库 | full墙钟 | full指令 | metadata墙钟 | metadata指令 |
|---|---:|---:|---:|---:|
| semver | 1.054932 | 1.062998 | 0.999708 | 1.000130 |
| base64 | 1.124125 | 1.198121 | 1.002065 | 1.000008 |
| unicode-segmentation | 1.017141 | 1.007170 | 1.000221 | 0.999929 |
| humantime | 0.984269 | 0.961578 | 0.997184 | 1.000048 |
| 每轮四库串行总和 | **1.049420** | **1.051311** | **1.001198** | **1.000004** |

候选以四库full墙钟约增加4.94%、指令约增加5.13%，取得13项运行平均耗时下降8.87%。base64单库full墙钟增加12.41%、指令增加19.81%；时间戳解析增加6.17%。metadata总体接近不变。本轮不能称运行、编译、大小三指标均改善，也不能称全局无回退。

实现保存在本会话的独立worktree，完整冻结源、差分和已测backend均可追溯。保留该结果供整体选择；既有accepted18r2对照和原工作树的其他候选均保留，不擅自合并或替换正式性能基线。本轮停止追加微调和重复计时。

## 最终验证回执

正常构建的14个base64对象与唯一Observe逐字节相同。全局筛选390条runtime、240条compile均校验产物与输入指纹；其中1500对完整对象逐字节等于各方案正常reference。全局归档SHA256为 `4c30df7968c16e3e4aeff1c444800b32422e845fae978a92a4fd9dcf0e7aa1fa`，2989个members已验封。

源码tree、workspace/native/四库正确性、完整逐轮值与验封状态保存在 [证据索引](results/decode-throughput/README.md)。实验中首个共享缓存错误和workspace ignored guard错误都按原状态保留，未计为编译器错误或测试通过。

源码、已测backend和8个原始证据归档均已在本机验封。本任务E2B沙箱已释放，2026-11-11 10:17:36 UTC 经控制面确认停止；详见证据索引中的 `sandbox-release-receipt.json`。
