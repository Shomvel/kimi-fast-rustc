# rustc 精读笔记

基于官方 rustc 1.94.1（commit `e408947bfd200af42db322daf0fadfe7e26d3bd1`）源码的实现笔记，供移植语义算法时查阅。每个主题一个目录，`README.md` 里有概览和“kfrc 最小闭包与移植建议”。

版本选择、模块总表和来源记录见 [sources/](../../plan/m0/t00-freeze-inputs.md)。

| 目录 | 主题 | 主要对应任务 |
|---|---|---|
| [borrowck/](borrowck/README.md) | NLL 借用检查、region 推导、loan、两阶段借用、生命周期来源、闭包 | T13、T16、T22 |
| [ownership/](ownership/README.md) | move path、初始化数据流、drop 显式化、作用域与 cleanup、临时值、match 降低 | T06、T09、T13、T22、T27 |
| [typeck/](typeck/README.md) | 推导上下文、expectation、coercion、autoderef 与方法查找、cast、模式 | T06、T12、T15、T21 |
| [traits/](traits/README.md) | trait 求解器、规范化、coherence、dyn | T12、T21、T25 |
| [macros-resolve/](macros-resolve/README.md) | 宏展开、macro_rules、hygiene、名称解析、过程宏、内建宏 | T04、T05、T23、T30 |
| [layout-abi-mono/](layout-abi-mono/README.md) | 布局、enum niche、调用约定、单态化、常量求值 | T08、T15、T17–T19、T24、T25、T27 |

## 阅读约定

- 链接都固定到 `e408947b`，行号经过核对。
- 标“推断”的内容没有在源码中直接验证。
- 笔记描述 rustc 怎么做；kfrc 怎么做以 [design/](../../design/README.md) 和 [plan/](../../plan/README.md) 为准。
