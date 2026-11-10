# E2B 验证记录

本轮复用接手会话的 E2B 沙箱 `itmdyt5e3uvkqwmscvqme`，模板 `r6g8s7tmv7hjdgpouxk9`，8 vCPU / 16 GiB，CPU 暴露为 AMD EPYC。Rust 1.94.1 / LLVM 21.1.8，MIR0 sysroot，库 O3/CGU16；公共驱动仍由 LLVM 编译。凭证仅进入本地 SDK 客户端参数。

HK 原记录入口已不存在；实际核对了同目录 chenjun canary 入口，仍调用 code-harbor entrypoint、environment=e2b，HK SDK 2.37.1。本地使用既有兼容客户端。具体路径和状态见 environment-receipt.json；没有运行原 SWE benchmark。

## 正确性

- start workspace：869 passed，1 ignored；all workspace：877 passed，1 ignored，无失败。
- H1 semantic_boundary_transport：7 passed；address cleanup：4 passed；checked arithmetic：3 passed，含原生 C 边界/随机/循环/兄弟分支对照。
- start、H1、address、checked、H1+address、all 均通过四库 corpus 和13项 workload checksum，与 LLVM 一致。
- all 上游 integration：semver 34、base64 13、Unicode 8，通过；humantime 无独立 integration。已知 fabsf64/atomic_xsub/atomic_load 缺失的 upstream unit 构建未重复，不能写成全套 upstream 通过。
- H1 与 all 的 enum-boundary-snapshot LLVM 差分通过；all integer-intrinsics LLVM 差分通过。all 13文件最终源 overlay 与本地冻结文件逐字节一致。

## 第一轮测量与布局诊断

原始结果位于本地 `evidence/suite/runtime/`，21轮前后交错、4倍原工作量，perf单独运行，编译不与计时并发。完整21轮7arm矩阵中，all/start等权运行几何平均0.99526，但 semver identifier 配对1.16145、humantime ts-parse配对1.02392，故未直接接受该候选。详细区间见 measurement-summary-first.json。

semver identifier 的 driver object 相同，动态instructions几乎一致。固定all对象，只对目标section做诊断性64B alignment和32B前缀padding；原函数759B机器码及归一relocation不变，重新生成的LLVM driver object也逐字节一致，语料/checksum一致。11轮诊断中 layout32/all配对0.86267 [0.85871,0.87157]，layout32/start 0.99979 [0.98954,1.00258]。单次perf cycles由all977.1M降至805.3M，start802.8M，支持布局影响。该对象修改仅用于诊断，不能作为生产按符号名特判。

## 验证脚本修正

H1首次native fixture因脚本误用不存在的sysroot路径失败；静态核对后改用现有 `/workspace/shared-std-mir0/sysroot`，只重跑该fixture后通过。布局诊断准备时修正了输出文件不可覆盖与nm精确符号匹配两处脚本契约，均未进入性能测量；这些不是编译器语义失败。

## selected 候选阶段记录（最终结论见下节）

已选择16 B通用循环入口对齐，32/64 B筛选保留在runtime-first/*alignment-screen.json。selected重新构建后四库objects与align16筛选对象逐字节一致，17文件源码与selected-source.json一致。selected workspace 882 passed、1 ignored；三库integration 34/13/8、enum16 cases、integer500条旋转/漏斗边界与131872对饱和运算通过。

最终21轮13负载和四库21对完整编译完成，原始JSON见runtime/与compile-final.json。selected/start运行等权几何平均0.98539、编译wall几何平均0.97539；semver identifier回到0.99597，tsparse1.00548。Unicode ASCII为1.06760但95%区间[0.99980,1.14525]很宽，当时仍在静态核查；后续独立复核结果见下节。完整区间与编译成本见measurement-summary-selected.json。

完整归档已下载并逐文件校验1055个文件，回执见evidence-receipt.json。ASCII不确定性已由后续独立复核关闭；沙箱释放记录见文末。


## 最终自然循环策略验证

自然循环标记复用schedule已有loop信息，修复机器码布局中把任意后向边当循环入口的策略；最终候选19文件SHA-256为`ea9ed9e00a5caa56555810585fb9e6faed5b473a4134d34a83193c275d690562`。远端源逐文件匹配accepted-source.json。natural workspace：883 passed、1 ignored；四库corpus/13checksum、enum16 cases和integer500条旋转/漏斗边界+131872对饱和运算全部通过。

四库所有objects、driver objects与最终driver二进制均与selected16逐字节相同，见natural-identity.json。因此12项已明确的运行结果继承同二进制证据，未重复全矩阵。上游integration的34/13/8通过记录属于selected后端，保留其来源；最终natural有独立workspace、四库和native验证。

ASCII的独立21配对复核为natural/start **1.00037 [0.99016,1.01687]**，指令比0.99999974，cycles比0.99840。结合热路径字节/地址相同与旧样本的大幅波动，未复现稳定回退。此复核与原13负载测量分开保存，不混合原始样本，也不静默替换旧区间。

新后端完整编译21对重测：semver wall0.98472、base640.98126、Unicode1.01185、humantime0.99960，等权几何平均0.99428；指令比1.00055/0.98380/1.00054/0.99974。wall区间较宽，不宣称可靠的整体编译加速；没有已证实超过2%的编译回退。逐crate区间、RSS/CPU及来源见measurement-summary-accepted.json和compile-natural.json。旧selected编译结果不冒充最终natural编译结果。

最终补充归档190文件及start/natural实际后端已下载并核验，连同前述1055文件归档保存在本地，见evidence-receipt.json。沙箱已于2026-11-10 19:27:53（Asia/Shanghai）释放；释放前再次逐文件核验两份归档并确认start/natural后端已本地保存。
