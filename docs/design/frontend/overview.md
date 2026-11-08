# 前端产物与 Body → SoN 交接

## 现状

[frontend.rs](../../../src/frontend.rs) 的 `parse` 直接返回 `Graph`，一个 `Parser` 同时管理 token、局部绑定、`ScalarType`（只有 `I64/U64/Bool/Unit`）、常量求值、函数签名和 SoN 控制状态。`if`、`match`、短路、循环各自维护绑定状态，各自拼 Phi——这正是 if 错编译的根源。

[mir.rs](../../../src/mir.rs) 是单基本块、i64、单次赋值的文本解析器，不是 rustc MIR 适配器，也不是计划中的 Typed Body。

新路径：变量赋值和控制流先表示在 Body 里，再由独立的 SoN builder 统一构造 SSA。

## 前端产物

逻辑产物，具体类型名由 T02 冻结。

| 产物 | 输入 | 保证 | 还不保证 |
|---|---|---|---|
| ParsedModule | 源文件、edition、源身份 | token/AST、精确源范围、解析错误 | 名字含义、类型、宏已展开 |
| ExpandedModule | token tree、宏定义、hygiene、cfg | 本轮可解析的定义和展开来源 | 导入/宏依赖已稳定 |
| ResolvedInterface | 定义表、命名空间、可见性 | 路径映射到定义身份，接口依赖可枚举 | 函数体已检查 |
| TypedBody | 已解析接口、类型约束、trait obligations | 每个表达式/Place 有类型，控制流和检查点显式 | 所有权合法、drop 已展开 |
| CheckedBody | TypedBody、move/loan/region 分析 | 受支持语义的借用约束成立，初始化与退出路径明确 | — |
| （单态化） | R 级图、具体泛型参数 | 在图上回答孔位、按答案键特化（[generics-specialization.md](../ir/generics-specialization.md)）；不再单独产出实例化的 Body | — |

每个产物带状态（成功 / 错误 / 等待依赖）、依赖指纹、目标与 edition 配置、来源。只有成功且依赖已满足的产物能被缓存命中使用。诊断位置不是定义身份，但宏 hygiene 不能随 span 一起丢掉。

## 循环与等待

- 调用和递归：先登记声明与类型，再构造函数体。
- 任务可返回“依赖未就绪”，但不能占着 worker 同步等待。
- 定义循环、常量循环、trait 循环各按自己的语言规则处理，不用一个通用 SCC 算法统一代替。

## Body → SoN 交接

- 类型检查阶段记录隐式借用、解引用、数值转换、方法选择；降低阶段只执行这些决定。
- 区分“未推导”与“错误类型”；错误恢复值不进入 codegen。
- SoN builder 接收 CheckedBody（泛型体不先实例化），构建 R 级图；单态化在图上进行（[generics-specialization.md](../ir/generics-specialization.md)）。
- SoN builder 只接收已验证的 Body，为所有前驱统一构造值和效果合并；它不重新决定名字、隐式转换、借用合法性或析构时机。
- 循环前驱、回边和不可达退出在图封闭前确定。
- 泛型 Body 不需要为每个实例重新检查语法树，但实例化需要具体 substitution、布局、调用解析、drop glue/vtable 需求。实例缓存键包含这些实际依赖，而不只是函数名和类型字符串。
