# MaybeUninit 的有限语义支持（candidate07）

目标是编译未经修改的 regex-syntax 中标准库数组迭代器路径。仅接受 producer 通过 Rust `maybe_uninit` lang item 标注的 `MaybeUninit<T>`；名称相似的用户 union 和一般 union 继续拒绝。此标识不把 `MaybeUninit<T>` 全局改成 T，也不改变 T 的有效值集合。

## 类型与值

`RecordedTypes::record_maybe_uninit(wrapper, inner)` 登记语义身份，`SemanticTypes::maybe_uninit_inner` 查询内层类型。wrapper 仍是 union，不是同时拥有 `uninit` 和 `value` 两个活动字段的 product。字段地址可以按 producer 的真实字段表选择；读取 value 字段必须进入 T/ManuallyDrop<T> 的 typed 观察边界。

解释值 `Value::Uninit { bytes: Arc<Vec<Option<u8>>> }` 是独立拥有的表示快照。`Some(byte)` 表示已初始化字节，`None` 表示未初始化字节；长度等于 wrapper 的大小，包含 padding。复制、参数传递、返回、MU typed load/store 都保留字节和初始化状态，不把内部 None 当作普通值 poison，不验证 T 的 bool/char/enum 有效性。`MaybeUninit<bool>` 中字节 2 可以保存，但不是合法 bool。

MU 不是“任意 poison 都合法”的标记：未定义普通 T 值和 MU 的原始未初始化内容仍有不同表示。`Undefined : MU<T>` 构造全 None 快照；嵌套 product 和数组按自己的字段递归应用规则。

## 初始化与观察

- 从已初始化 T 构造 MU，先按 T 的原表示编码，再封装为 opaque MU；不为 padding 填零，也不通过读取其他 union 字段解释内容。
- 固定 `TransmuteValue` 先观察源类型的值：普通 T 的 poison 不能经 MU 编码变成合法 None 内容。这个观察只递归语义字段，不读取普通 aggregate 的 padding；MU 字段本身的原始 None 内容仍合法。
- MU load/clone/copy/return 不解码为 T。必须保留不同 MU 字节布局间的初始化信息，不能先 typed decode 再复制。
- MU 转为 T、读取 value 字段和 `assume_init`/`assume_init_read` 才按目标类型解码。非法 bool/char/enum 为 UB；普通 T 的未初始化部分遵循已有固定 transmute/typed observation 契约。真正的 UB 输入不承诺机器代码插入运行时 trap。
- 只有能证明同型、已初始化和没有中间修改的 `T → MU<T> → T` 才可值转发；不能将所有 wrapper 或任意等长 transmute 当作 identity。
- MU 存取和字节拷贝仍遵守地址、分配范围、对齐、存储生命周期与不可变存储检查。

现有解释器的指针仍采用地址位值模型。本批不声称添加完整 pointer provenance、一般 union type-punning、任意无效引用有效性检查或 Miri 等价性。R definition producer 可继续把 MU 标为 NotEligible，使用既有单态 S 路径；不以这一限制伪称完整 R 支持。

## 验收

独立机制验证先覆盖未初始化容器的安全返回/复制、非法 bool 字节存储与 typed 读取失败、padding 初始化 mask、独立快照、嵌套数组/聚合、部分初始化和原一般 union 负例。集中验证先取得 Rust 1.94.1 实际 instance MIR，再测数组迭代器 0/1/N、next/next_back、部分消费后 clone、早退/drop 计数，以及原始 regex-syntax 0.8.8 同一命令的 LLVM 对照。不能修改第三方源码或以 fallback 绕过 kfrc 路径。

已取得 Rust 1.94.1 的真实 `tcx.instance_mir`：`evidence/poc-semantic-only-20261109/results/instance-mir-driver/instance-mir/0001.mir`，实例参数 `[ClassBytesRange, 1_usize]`。其唯一基本块先 `copy _1 as [MaybeUninit<T>; N] (Transmute)`，再构造 `IndexRange { start: 0, end: N }`、`PolymorphicIter { alive, data }`、`MaybeDangling`、`ManuallyDrop`，最终返回 `IntoIter`。`new_unchecked` 等已在这份 MIR 中内联，MU 仍是返回值中存活的表示，不能把构图前拒绝归因于一个可删的死 union 临时。

独立 S 机制用例位于 `crates/kfrc-son/tests/semantic_maybe_uninit.rs`。覆盖全 None 容器的安全参数/返回、invalid bool 的容器存取与 typed-read 失败、char/enum 的延迟有效性检查、合法 bool roundtrip、普通 poison 禁止洗入 MU、部分初始化与 padding mask、快照独立性、数组迭代器形式的嵌套聚合、错误长度/值表示和假同名 union 拒绝。此处列出覆盖范围，不替代集中验证的实际运行日志。

## candidate08 的 DST 地址边界

candidate07 的真实数组迭代器 clone 首失败位于 `clone_into_new::<u64>`，实际 MIR 为 `evidence/poc-semantic-only-20261109/results/candidate07/mu-diagnosis/mir/0008.mir`。输入是 `&PolymorphicIter<[MaybeUninit<U>]>`；它先读取 sized `IndexRange` 前缀，再对 unsized slice 尾字段取地址，后续提取长度、转成元素指针、偏移并重新组成 slice 指针。这里需要保留字段的逻辑 owner 和 metadata，不能把 Rust `*mut DST` 伪造成 8 字节类型。

`TypeShape::PlaceAddress { pointee }` 表示 IR 内部的 thin 存储地址，当前目标固定 8 字节。它可以指向 DST，metadata 由 builder 的 place 单独携带；Rust `Reference`/`RawPointer` 仍使用 producer 给出的真实表示。字段地址按 owner 的真实字段 offset 计算；slice 索引按元素大小计算。形成 Rust 胖指针时必须显式把地址转换为真实 data 字段类型，并与 metadata 组成 `WidePtr`。

`is_unsized` 是 producer 认证的源事实，不从缺少 layout 推断。`static_slice_tail` 只认证最终为 slice/str、offset 静态且沿用 slice 长度的尾字段链；dyn Trait 等动态对齐尾字段继续拒绝。extern type 及其结构体尾链使用 `thin_unsized`，保留其 metadata 为 unit 的真实 thin 指针行为。unsized 类型都不能直接构造局部分配或按整值 load/store。
