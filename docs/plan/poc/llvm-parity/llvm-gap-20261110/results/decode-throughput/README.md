# Decode throughput 证据索引

此目录保存本轮实现的源码身份、测量结果和验封回执。结果解释见 [decode-throughput-results.md](../../decode-throughput-results.md)。

- `compiler-candidate-manifest.json` 是执行前冻结的原始 manifest，`validated:false` 和 `diagnostic_only:true` 保留原文。执行后的验证状态在其余独立回执中，不回写冻结输入。
- `correctness-summary.json` 记录同源 backend 的七组原生差分、四库检查器、原corpus与13项checksum。
- `workspace-guard-receipt.json` 绑定原始cargo exit0、1137通过、0失败和1项既有ignored计时测试。原wrapper误拒ignored的状态与日志保留，没有重复cargo或执行ignored计时测试。
- `final-semantic-verification.json` 汇总源码与backend身份、43项定向回归、1137项workspace回归、7202组完整buffer差分、七组原生差分、四库正确性及全局筛选。SHA256为 `81f50642801ccbfbb4e48793288a4772bedd64c2c46ed77f690bbbc4f932eafd`；它记录执行后的验证状态，未修改冻结输入。
- `targeted-observe-verification.json`、`decode-triad-verification.json`、`workspace-verification.json`、`native-four-library-verification.json` 绑定全部原始日志归档，列出本机完整路径与SHA256。
- `screen-selection.json`、`screen-provenance.json` 固定本次三方案、输入、flags、工具链、源/backend与正常对象。
- `screen-analysis.json` 保留逐项全部配对值；`screen-verification.json` 记录390条runtime、240条compile以及1500对完整objects的逐字节核验。`root-result-check.json` 是主代理对封存原始记录重新计算的编译总和与archive SHA核对。
- `evidence-index.json` 汇总8个已验封归档（35,878,574B、6171个manifest成员）、1846个冻结源文件与已测backend的本地路径及SHA256。
- `sandbox-release-receipt.json` 记录本任务沙箱已释放，2026-11-11 10:17:36 UTC 经控制面确认不存在。释放后的SDK健康查询503也原样保留；未重试kill或恢复沙箱。

测量环境为本任务的E2B沙箱 `imj3ukm7eqxdk2x0im3uh`，8 vCPU/16GiB、Rust1.94.1/LLVM21.1.8。库代码由候选backend产生；原测试driver使用LLVM编译并链接候选库。runtime固定CPU7，编译使用8CPU；性能与profile分开，本轮没有新增profile。源码、backend及原始证据全部下载验封后，该沙箱已释放。

完整源码tree为 `9235520b2dd380f635c5c51d76baa2c890b9c84b61d2de9f10841dafeef495a3`，backend为 `6c25173e5b00a7056f891748beaf80294fba00e71fd3a4de15e9d81a0616521c`。worktree的1846个冻结文件均逐hash匹配，新增结果文档独立于该源码输入。相对accepted18r2的准确补丁保存在 `/Users/shom/Downloads/kfrc-callsite-opt-20261110/decode-private-copy-v1-frozen/parent-diff.patch`；不能以worktree的HEAD差分重新计算本轮增量，因为HEAD还未包含accepted18r2的历史未提交优化。

原始大归档和Linux backend保存在本机 `/Users/shom/Downloads/kfrc-callsite-opt-20261110/generic-parity-20261110`。本目录不拷贝大型二进制或凭证。四库筛选不包含regex-syntax，也不代替因既有缺失intrinsic阻断的全部上游unit测试。
