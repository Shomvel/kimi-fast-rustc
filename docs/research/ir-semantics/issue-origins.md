# walkthrough 发现的语义根源：哪些问题先有语义就能避免

对象是 [findings.md](../walkthroughs/findings.md) 的 60 条发现。“能避免”指：写语义时必须回答的问题恰好就是这条发现；“能检测”指检查模式解释器在 walkthrough 程序上会直接报错。判断是本调研的推断。

## 健全性 14 条

| # | 问题（简） | 先有语义 | 对应语义规则 |
|---|---|---|---|
| F11 | Release fence 只排序 `Unknown` | 部分 | 顺序语义观察不到；但“分区 = 逃逸前别的线程看不到的字节”这一定义会逼出“fence 之后才逃逸”的情形（[execution.md](execution.md#fence-与原子)） |
| F12 | `Load`、`VariantField` 无控制依赖，L 级可把 load 提到 null 判断前 | 能避免 | 固定 `Load` 与 `Load.spec` 之分；`Load.spec` 的可解引用条件（[execution.md](execution.md#投机-load)） |
| F13 | 所有引用都 `dereferenceable` | 能避免 | `Frozen`/`Unique` 是以 protector 为依据的 UB 条件，只对 `&Freeze`/`&mut Unpin` 成立（[memory.md](memory.md#分区两类地位不同)） |
| F14 | 机器码键漏掉具体类型布局与被调者摘要 | 能检测 | ⟦R⟧ 以答案表为参数；解释器记录读过的孔位与摘要（[refinement.md](refinement.md#r-的语义以答案表为参数)） |
| F15 | 原子只在 `Unknown`，与根规则矛盾 | 能避免 | 分区是**字节**的属性（归属表），不是操作种类的属性 |
| F24 | 上游 R 图带着上游会话的 panic 策略 | 能避免 | ⟦R⟧ 不能读会话状态，`PanicStrategy` 必须是答案 |
| F46 | 逃逸是流敏感的，分区却静态指定 | 能避免 | 逃逸是动态事件，事件后字节归属 `Unknown` |
| F47 | `Reborrow` 子分区逃逸不传给父分区 | 能避免 | 同上：归属表按字节，不按分区名 |
| F48 | `&[T]` 的读被投机上提越界 | 能避免 | `Load.spec` 只落在 sized pointee 的静态大小内 |
| F49 | 根规则没覆盖 Phi/Select、cast、聚合字段 | 能检测 | 动态分区标签沿值传递，与静态根集合比较 |
| F50 | 调用只看指针实参，没看聚合字段里的指针 | 能避免 | 调用的效果定义为“实参值可达的全部指针” |
| F51 | CheckedBody 键不含 impl 集合 | 无关 | 前端缓存问题 |
| F52 | 优化器折叠 `Location` 内容 | 无关 | 这是代码身份规则（[设计红线](../../design/README.md#设计红线) 2），不是语义：读 `Location` 在语义上合法 |
| F53 | “优化配置”字段未列出 | 部分 | 语义的参数（panic 策略、overflow-checks、target features）就是必须进键的配置；代码生成相关的（unwind 表、路径重映射）不在语义内 |

合计：能避免 9 条，能检测 2 条，部分 2 条，无关 2 条（F51、F52）。

## 其他发现中与语义直接相关的

| # | 问题（简） | 对应语义规则 |
|---|---|---|
| F01 | `ValidRange` 被 S 级私下读取 | 同 F14 的答案读取审计 |
| F16 | `Discriminant` 与 `VariantOf` 不打通 | 语义定义 `Discriminant(v) = 判别值表[VariantOf(v)]`，规范化规则就是这条定义 |
| F23 | 写回已知当前值的 store 删除 | 依赖 `Refine` 的事实来源（S6）：`Proven` 才能自由使用 |
| F26 | `Landing` 无异常值 | `Exception` 值的语义（G7） |
| F35 | 删除未逃逸的 `Alloc`/`Dealloc` | 分配不可观察（G2） |
| F39 | 拷贝亲和合并要求“编码重写是恒等” | S→L 的值关系 `encode`（[refinement.md](refinement.md#s--l布局降低就是-encode)） |
| F43 | `Box::new(call())` 的分配能否前移 | 分配不可观察 + 被调者不发散不 unwind；有了语义可直接写成 refinement 条件 |
| F45 | 数组值的动态下标 | 值域里数组是 `Tuple`，动态下标需要地址，与“只有取地址才进内存”一致 |

## 结论

14 条健全性发现里 11 条落在三处语义空白：**load 何时可以执行**（F12、F13、F48）、**分区是谁的属性、何时改变**（F15、F46、F47、F49、F50）、**R 图依赖什么**（F14、F24，F53 部分）。这三处分别对应 [execution.md](execution.md) 的投机 load、[memory.md](memory.md) 的字节归属表、[refinement.md](refinement.md) 的 ⟦R⟧_A。walkthrough 是在“手工模拟语义”；有了解释器，这些模拟可以变成可重复的检查。
