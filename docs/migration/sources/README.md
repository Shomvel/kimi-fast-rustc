# rustc 来源

| 文件 | 内容 |
|---|---|
| [rustc-map.md](rustc-map.md) | 每个语义领域参考哪些 rustc 模块、耦合程度、移植方式 |
| [provenance.md](provenance.md) | 每次移植要留下的来源记录、许可检查 |

本计划 PR 没有复制任何 rustc 源码。

## 版本：有两棵树，必须分清

| | 本地派生树 | 官方 1.94.1 |
|---|---|---|
| 身份 | `src/version = 1.94.1`，HEAD `acea19e8bf9280a1bc7fa606d3b7670d0fa7d734`（提交标题为 P2b metadata 流水线修复），无 Git remote | annotated tag 对象 `4c540d92e07b57f8a961905e3c391feaa0a8cc95` → commit [`e408947bfd200af42db322daf0fadfe7e26d3bd1`](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1)（经 GitHub Git Data API 核实） |
| 本计划中的用途 | 详细实现阅读 | 文档中的公开链接（路径导航） |
| 注意 | 含本地修改（P1/P2b 流水线等），不是官方 1.94.1，不能给它拼官方链接 | 移植前须与本地树对比相应文件，不把本地修改归因到官方 |

**产品采用的参考版本尚未选定。** 历史 benchmark 使用的 1.99.1 工具链不代表该版本源码已验证，也不能与 1.94.1 的内部类型混用。不同模块不要各取不同 nightly 的私有表示。

## M0 选版本的完成条件

- 按目标项目的 MSRV、edition、sysroot 和可复现工具链选定一个精确的官方源码 commit；固定 oracle 的 `rustc -Vv`、源码归档摘要和适用的本地补丁。
- 官方源码、oracle 二进制、目标 sysroot、目标项目源码/lockfile 都有固定身份。
- 本地派生树的 P1/P2b 等改动与官方实现分开保存，采用与否单独可追溯。
- 所选版本满足冻结项目需求；不满足就先重选，再批量移植。
- 第一批移植闭包的来源清单、许可和回归样例，另一个人在没有本地历史目录的情况下也能取得。
