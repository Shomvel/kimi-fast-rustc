# workload、内核占比与验收 workload 的问题

定义在 [workloads.sh](scripts/workloads.sh)。语料由 [setup-sandbox.sh](scripts/setup-sandbox.sh) 生成：目录语料 = Cargo registry 源码（rg 与 Cargo 0.99.0 的依赖）+ Rust `library/`，34,349 个文件、590 MB；单文件 `mid.txt` = 全部 `.rs` 按路径排序拼接后的前 256 MiB（走 mmap）。

## ripgrep（15 个）

| 名字 | 参数要点 | 要测的路径 |
|---|---|---|
| official | `-n --no-messages 'fn\s+main'` 搜 ripgrep 源码树 | kfrc 验收 workload（kfrc VALIDATION.md 同款，commit 138723c） |
| dir-files | `-j1 --files --sort path` 目录语料 | 遍历 + ignore/glob |
| dir-regex | `-j1 -c 'impl\s+\w+\s+for'` 目录语料 | 前期 mirx “regex” |
| dir-mt | `-c 'fn\s+\w+'` 目录语料，默认线程 | 前期 mirx “multithread” |
| lit | `-c PhantomData` 单文件 | memmem |
| lit-i | `-c -i phantomdata` | 大小写变体 → Teddy |
| lit40 | `-c -F -f` 40 个标识符 | Teddy |
| word500 | `-c -F -f` 500 个高频标识符 | 实际选了 lazy DFA，不是 Aho-Corasick |
| dfa | `-c '[a-z][0-9][a-z][0-9]'` | 无字面量 → 稠密 DFA |
| suffix | `-c '[A-Z][a-z]+Error'` | 后缀字面量 + 反向 DFA |
| unicode | `-c '\b\p{Lu}\w+Error\b'` | Unicode 类与词边界 |
| lines | `-n 'fn '` | 大量匹配行的输出 |
| ctx | `-n -C2 'panic!\('` | 上下文输出 |
| json | `--json TODO` | JSON 输出 |
| help | `--help` | 启动 + 文本排版 |

## Cargo（7 个）

| 名字 | 参数 | 输入 |
|---|---|---|
| metadata-nodeps | `metadata --format-version 1 --no-deps --offline --locked` | 本轮用 Cargo 0.99.0 源码树作输入；T00 已定 0.95.0，M0 起改用 0.95.0 源码树；kfrc 验收 workload |
| metadata-full | 同上去掉 `--no-deps` | 同上：全部 manifest + 带锁解析 |
| tree | `tree --offline --locked` | 同上 |
| lockfile | `generate-lockfile --offline` | Cargo 0.95.0 源码副本：从零解析 |
| noop-build | `build --release --offline --locked` 已构建的 ripgrep | 指纹检查 |
| package-list | `package --list --allow-dirty --offline` | ripgrep 源码树 |
| build-help | `build --help` | 启动 + clap |

lockfile 不用 0.99.0：它的 `Cargo.lock` 钉住了已 yank 的 chacha20 0.10.1，离线重新解析会失败。

## 内核占比（A 臂，[kernel-share.sh](scripts/kernel-share.sh)，每项连跑 20 次平均）

| workload | wall ms/run | user ms/run | sys ms/run | sys/(user+sys) |
|---|---:|---:|---:|---:|
| official | 6.50 | 2.00 | 7.00 | 78% |
| dir-files | 152.50 | 56.00 | 95.50 | 63% |
| dir-regex | 502.50 | 109.00 | 387.50 | 78% |
| dir-mt | 103.00 | 199.00 | 527.00 | 73% |
| lit | 43.00 | 15.00 | 28.00 | 65% |
| lit-i | 60.50 | 31.00 | 29.50 | 49% |
| lit40 | 173.00 | 136.00 | 37.00 | 21% |
| word500 | 567.00 | 529.50 | 36.50 | 6% |
| dfa | 489.00 | 456.50 | 32.50 | 7% |
| suffix | 52.00 | 21.50 | 30.00 | 58% |
| unicode | 57.50 | 24.00 | 33.00 | 58% |
| lines | 116.00 | 81.00 | 35.00 | 30% |
| ctx | 63.00 | 25.50 | 37.00 | 59% |
| json | 63.00 | 24.50 | 38.00 | 61% |
| help | 3.00 | 1.50 | 1.50 | 50% |

| workload | wall ms/run | user ms/run | sys ms/run | sys/(user+sys) |
|---|---:|---:|---:|---:|
| metadata-nodeps | 27.00 | 19.00 | 8.00 | 30% |
| metadata-full | 322.50 | 214.50 | 110.50 | 34% |
| tree | 264.00 | 167.00 | 98.50 | 37% |
| lockfile | 615.00 | 461.50 | 154.50 | 25% |
| noop-build | 47.50 | 31.00 | 16.50 | 35% |
| package-list | 34.00 | 22.50 | 11.00 | 33% |
| build-help | 4.50 | 1.00 | 3.50 | 78% |

- rg 的目录 workload 内核占 63–78%（任务描述中的约 95% 在本机型上没有复现）；单文件 256 MiB 的 mmap 缺页也让 sys 占到 49–65%。只有 lit40/word500/dfa 以用户态为主。
- 用户态里还有一块不是 rustc 编的：glibc 的 malloc/free 与 mem*，Cargo 还有 libcrypto（见两份 hotspots）。

## 验收 workload 的问题

| | rg official | Cargo metadata-nodeps |
|---|---|---|
| 墙钟（本机型） | 6.5–14 ms | 27–32 ms |
| 内核占比 | 78% | 30% |
| 用户态中非 rustc 代码 | allocator 17.9% + libc mem* 等 | libcrypto 33.0% + pthread 12.3% + allocator 12.0% + mem* 6.2% |
| A 臂墙钟 spread（30/20 轮） | 308.1% | 54.4% |
| S0（无显式 SIMD）| 指令 3.117×，墙钟 1.013 | — |
| C（V+U+R+J）| 指令 1.120×，墙钟 1.003 | 指令 1.006×，墙钟 1.000 |

1. **墙钟几乎看不见编译器质量。** rg official 去掉全部显式 SIMD 指令数涨到 3.1×，墙钟中位数只差 1.3%，淹没在噪声里；kfrc VALIDATION 中同一二进制两臂就差 0.7%。
2. **反过来也成立：** 一个慢 3× 的后端可能在这两个 workload 上“通过” ≤1.0，而在真实搜索上慢 2–10×。
3. 建议：保留这两个 workload 做端到端冒烟，但 ≤1.0 的判定用本页的完整 workload 集，主指标用 `instructions:u` 与 `cycles:u`（用户态），墙钟只作辅助，并按 [性能验收协议](../../plan/README.md#验收) 用单侧置信上界判定。M0 设计见 [m0-experiment.md](m0-experiment.md)。
