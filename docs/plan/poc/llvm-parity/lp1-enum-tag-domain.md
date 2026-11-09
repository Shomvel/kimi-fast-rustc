# LP1 直接 tag 枚举在 tag 值域上运算

## 问题

L 层把枚举的“当前变体”统一表示为**变体序号**（0..n）。当 `TagEncoding::Direct` 的
判别值不是 0..n 时，会出现三条 O(n) select 链：

1. 读：raw tag → 序号（`decode_tag_index`）；
2. 取判别值或 `as` 转换：序号 → 判别值（`variants.rs` 中 `select_scalar` 后的循环）；
3. 写：动态序号 → tag（`encode_variant` 末尾）。

每个 select 又用掩码算术实现（约 6 条指令），所以 64 个变体的 `Alignment` 读一次判别值约需
3,900 条指令，而且是无分支地**全部执行**。`Layout`、`RawVec` 的每次扩容和释放都会经过它。

## 目标形态

对 `Direct` 编码，L 层的变体标量直接取 raw tag（判别值域），不转换成序号：

| 操作 | 现在 | 目标 |
|---|---|---|
| 读判别值／`as` 转换 | raw → 序号 → 判别值 | raw 做一次整数转换 |
| `VariantSwitch` | 序号与 0..n 比较 | raw 与各判别值常量比较；交给 LP5 的 switch 降低 |
| 构造已知变体 | 存常量（已如此） | 不变 |
| 构造动态变体 | 序号 → tag 的 select 链 | 来源本就是 raw tag 时直接存；只有真正来自序号的才查表 |
| 复制／传参 | 解码再编码 | 原样搬 tag 字节 |

只有确实需要序号的场合（例如按序号索引的表）才做一次映射，用 LP5 的查找表实现，不用 select 链。

Niche 编码也按同一原则检查：`decode_tag_index` 的 Niche 分支已是“减起点 + 比较”，
确认读判别值、比较和 switch 都不再经过序号往返。

## 实现要点

- 变体标量的“值域”（序号或 raw tag）作为 L 层 `Value` 的一部分，在
  `lower_layout/variants.rs`、`variant_switch.rs`、`variant_encoding.rs` 和
  `representation.rs` 之间一致传递；不要在每个使用点各自猜测。
- `variant_tags` 缓存已记录 raw，可作为起点。
- 识别“只用于判别值”的 fieldless enum：整个值就是一个带有效范围事实的整数。
  有效范围在 LP6 的边界检查消除中可用，作为事实，不生成运行时检查。

## LLVM 参考

LLVM 根本没有序号这一层：rustc 生成的 IR 直接对 tag 整数做 `switch`／`icmp`，
`Alignment::as_usize` 就是一次 load 加整数转换。查找表见
[SimplifyCFG.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Utils/SimplifyCFG.cpp)
中 `switch-to-lookup` 的查找表部分。

## 验收

- 结构：`Alignment::as_usize` 等价的 fixture 降到常数条指令（目标 ≤ 4）；增加变体数量时指令数不随之增长。
- 非连续判别值、负判别值、`#[repr(u8/i32)]`、niche、`Option<Alignment>` 走三臂差分。
- semver：`finish_grow`、`RawVec::drop`、`pad_to_align` 体积与 runtime 前后对照，按 LP9 报告。


## 2026-11-09 实现进展（candidate14）

已实现直接 tag 数据流，candidate14 已完成 E2B 正确性与性能测量。源码和执行回执位于
`evidence/poc-llvm-parity-lp1-20261109/`。

- `VariantScalar::{Direct, Index}` 明确标记值域。直接编码的构造、表示常量、
  读取、Undefined、Phi、payload 守卫和写回共享 tag 类型；niche 与 Single
  继续使用序号，niche 的原始 tag 缓存仍供 switch 使用。
- `Discriminant` 对直接 tag 只作整数转换；`VariantSwitch` 与真实 tag 常量比较。
  显式 `VariantOf` 保留序号语义，已知构造走常量快路；动态序号查询的映射仍待
  LP5 改为查找表，此次不宣称消除了所有 select 链。
- 新增 2／16／64 变体规模回归，检查 Phi 和存取后读判别值的比较、访存与
  选择运算数量不随变体数增长；另覆盖窄有符号／无符号 tag 和显式序号查询。
- 三臂 fixture 增加 64 个 Alignment 判别值、`repr(u8/i8/i32)`、
  `Option<Alignment>`、构造／参数 Phi、循环、内存复制与带 payload 的枚举。
- 回归暴露了 L 解释器的常量未按宽度截断，而 R／S 解释器已截断的不一致。
  修复 L 常量入口后，本机 workspace 测试通过；负判别值的解释器差分纳入回归。

E2B 已通过三臂语义输出（每臂 466 行）、semver 34 项原版测试、7 套 legacy 语料，
以及优化开／关各 200 个生成函数 × 6 组输入。

此轮不修改寄存器分配与机器栈帧。实测按值 fixture 在 2／16／64 个变体下
均为 30 条机器指令，普通判别值路径的 O(n) 映射已消除，但 ≤ 4 条目标未达。
性能与完整验收边界见 [evidence.md](evidence.md#candidate14lp1-直接-tag-数据流)。
LP1 主体已经落地；严格结构门槛仍待 LP2，显式动态序号映射仍待 LP5。
