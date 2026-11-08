# T00 固定版本、来源和能力清单 — 细化

`M0 · I · 依赖：无` · 概要见 [m0-baseline.md](../m0-baseline.md#t00)

## 已知的候选输入（2026-11-08 核对）

| 输入 | 候选 | 依据 |
|---|---|---|
| kfrc 基线 | `44c9d80b02ab140d7edf5348b6946d8122d842b6` | 当前 main |
| rustc 参考源码 | 官方 1.94.1，commit `e408947bfd200af42db322daf0fadfe7e26d3bd1` | 见 [sources/](t00-freeze-inputs.md)；本地有干净 sparse checkout（compiler/、tests/ui/、rustc-dev-guide） |
| ripgrep | 15.2.0，commit `e89fff89ac9af12e8d4ce9d5fd07beb408ca730f`，`Cargo.lock` SHA-256 `7a7d39cd…a64`，59 个包，MSRV 1.85 | 历史 benchmark 工作区 |
| Cargo | 0.99.0，commit `797e8a9bca276c1c9f9f738d2a20f484fa4eea9d`，`Cargo.lock` SHA-256 `1f623862…e6c`，550 个包；workspace 写 MSRV 1.94，但 `cargo` 包本身要求 **1.96**，`libsqlite3-sys` 0.38.1 用了 `cfg_select!`，**rustc 1.94.1 编不过** | 同上；2026-11-08 E2B 实测 |

**待决：** Cargo 0.99.0 与 rustc 1.94.1 不兼容。二选一：目标改为 Cargo 0.95.0（运行时画像实测用的就是它），或参考 rustc 升到 ≥ 1.96（rustc 精读笔记、`ra-ap-*` 版本、sysroot 统计都要随之更新）。ripgrep 的 MSRV 1.85 两种都满足。

## 两棵 rustc 树要分清

| | 本地派生树 | 官方 1.94.1 |
|---|---|---|
| 身份 | `src/version = 1.94.1`，HEAD `acea19e8`，含 P1/P2b 流水线等本地修改，无 remote | commit `e408947bfd200af42db322daf0fadfe7e26d3bd1` |
| 用途 | 历史实验 | 文档链接、移植来源 |

移植前与官方实现对比相应文件，不把本地修改归因到官方。每次移植在 `docs/plan/inputs/provenance/` 留一份记录：来源 commit、文件与行范围、许可（MIT/Apache-2.0）、改动摘要、对应回归用例。

## 依赖 crate 的输入

kfrc 依赖 rustc 发布的 crate（决策 D1/D2），输入清单加上 `ra-ap-*` 的精确版本（`=0.148.0`）与“源码与官方 1.94.1 diff 为 0”的证据；来源记录分三类：移植代码、依赖 crate、仅设计参考。

## 交付物

1. `docs/plan/inputs/manifest.toml`（或同等格式），每个输入一条：

   ```toml
   [[input]]
   name = "rustc-src"
   kind = "source"           # source | toolchain | project | host
   origin = "https://github.com/rust-lang/rust"
   tag = "1.94.1"
   commit = "e408947bfd200af42db322daf0fadfe7e26d3bd1"
   archive_sha256 = "…"       # 源码归档或 git tree 哈希
   local_patches = []         # 本地派生树的补丁单列
   ```

2. oracle 工具链：`rustc -Vv`、`cargo -V`、sysroot 路径和 `rust-std` 组件哈希；同一版本在执行主机上的安装方式。
3. 目标项目：commit、lockfile 哈希、启用的 features（ripgrep 默认不开 `pcre2`）、构建 profile。
4. 能力矩阵 `docs/plan/capabilities.md`：每个语言特性一行，状态为 `supported / rejected / planned`，首版从当前 `frontend.rs` 实际支持的子集生成（`i64/u64/bool/()`、`as`、`if/match/while/for/loop`、const item、可见性修饰）。
5. 在执行主机上用 oracle 工具链以默认模式和 `-Znext-solver=globally` 各构建一次 ripgrep 与 Cargo（离线依赖缓存先准备好），记录是否都成功；ripgrep 已在本地验证两种模式都能编译，Cargo 尚未验证。
6. 本地派生树（`acea19e8`，含 P1/P2b 流水线改动）与官方 `e408947b` 的差异文件列表，标出与语义移植相关的部分（预期为空，需核实）。

## 已知的依赖形状（用于后续阶段规划）

- ripgrep 的 `Cargo.lock` 含过程宏链 `proc-macro2 → quote → syn → serde_derive`，但 `serde_derive` 只是 dev-dependency：Linux 默认 features 的 normal/build 依赖里没有过程宏 crate，有 10 个构建脚本（含 `cc`、`pkg-config`）。`pcre2-sys` 仅在 `pcre2` feature 下需要。
- Cargo 依赖大量 `-sys` crate：`curl-sys`、`libgit2-sys`、`libssh2-sys`、`libz-sys`、`openssl-sys`、`libsqlite3-sys`、`libnghttp2-sys`、`linux-raw-sys` 等（M5 的 C/C++ 链接清单从这里开始）。

## 完成标志

- 另一台干净机器只按 manifest 就能取得全部输入并校验哈希。
- `rustc -Vv` 输出、lockfile 哈希与 manifest 一致。
- 能力矩阵中每个 `supported` 条目至少对应一个现有测试文件。
