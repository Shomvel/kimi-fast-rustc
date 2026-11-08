# enum 布局、niche 与判别值

数据结构见 [layout.md](layout.md)。链接前缀固定到 `e408947b`。

## 入口分流（`layout_of_struct_or_enum`，[layout.rs#L277](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs#L277)）

1. **absent 变体**：uninhabited 且所有字段都是 1-ZST（[#L31](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs#L31)）。非 `repr(C)` 时 absent 变体不占判别值。uninhabited 但有非 ZST 字段的变体仍要留空间（部分初始化）。
2. 没有 present 变体的 enum → `never_type()`（`Variants::Empty`）。
3. 只有一个 present 变体且不禁止 enum 优化（非 `repr(C)`、无 `repr(int)`）→ 走 struct 路径，`Variants::Single{index}`，**没有 tag**。例：`enum E{A(u32), B(!)}` 就是一个 u32。
4. 其余进入 `layout_of_enum`（[#L559](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs#L559)），packed 直接报 `ReprConflict`。算两种布局再挑。

## niche-filling 布局（[#L577](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs#L577)）

`repr(C)`/`repr(int)` 或变体数 < 2 时不算。

1. 每个变体 `univariant(AlwaysSized)`；`largest_variant_index` 取 size 最大者（`max_by_key` 并列取最后一个，[#L604](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs#L604)）。它成为 `untagged_variant`。
2. `niche_variants` = 第一个到最后一个“需要编码”（非最大、非 absent）的变体下标闭区间；`count = end - start + 1`（中间的 absent/最大变体也占号）。
3. 只用**最大变体自己的 `largest_niche`**，`Niche::reserve(count)` 从 valid_range 外侧挪出 `count` 个值，返回 `niche_start`（[lib.rs#L1950](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/lib.rs#L1950)）。挪哪一端：尽量让结果落在 0（`Option<&T>` 的 None = 0），避免越过 0；`count>1` 时很难拿到 0。
4. 其他变体必须放在 niche 之前（`size <= niche_offset`）或 niche 之后（对齐后不超过总 size），否则放弃此方案。挪到后面的变体整体加偏移、`backend_repr` 改 `Memory`。
5. `backend_repr`：总 size/align 与最大变体相同且其他变体全是 ZST 时，`Scalar` 换成带新 valid_range 的标量；`ScalarPair` 中 niche 那一半保留、另一半 `to_union()`；否则 `Memory`。
6. `TagEncoding::Niche{untagged_variant, niche_variants, niche_start}`，`tag_field = 0`，enum 自身 `fields` 只有一个字段（tag）在 `niche_offset`。

## tagged 布局（[#L732](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs#L732) 之后）

1. **判别值集合**：`repr(C)` 时取全部，否则只取 inhabited 变体；按 `discr_type` 有符号时先做符号扩展（[#L741](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs#L741)）。排序后找相邻值间最大的空隙（含回绕），空隙之外就是 tag 的 valid_range `[min, max]`。
2. **最小 tag 整数** `discr_range_of_repr(min,max)`（[ty/layout.rs#L78](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/layout.rs#L78)）：有 `repr(int)` 就用它；否则 unsigned/signed 取较小的拟合，偏好 unsigned；`repr(C)` 下限 `c_enum_min_size`（x86_64 Linux = `c_int` = 32 位），Rust repr 下限 I8。
3. **变体布局**：每个变体 `univariant(Prefixed(tag_size, prefix_align))`。`prefix_align` 默认是 tag 对齐；`repr(C)` 时取所有字段最大对齐，得到 C 的 `struct{tag; union{...}}`（[#L808](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs#L808)）。
4. **tag 加宽**（[#L880](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs#L880)）：非 `repr(C)`/`repr(int)` 时，tag 加宽到“各变体第一个非 1-ZST 字段的最小对齐”对应的整数，并把紧贴 tag 的字段偏移挪到新宽度后。所以 `enum E{A(u32),B(u32)}` 的 tag 是 u32，不是 u8。
5. **backend_repr**：tag 占满整个 size → `Scalar(tag)`；否则若所有有数据的变体都恰好一个同 offset 的标量字段（同 primitive；同宽整数不同符号取第一个；同宽 int/ptr 混合取 ptr）且布局与 `scalar_pair(tag, prim)` 一致 → `ScalarPair`（某变体无字段时第二元素为 `Scalar::Union`）；否则 `Memory`（[#L926](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs#L926)）。选中 Scalar/ScalarPair 后，`Memory` 变体被改写成同一 repr 并补齐 size/align。
6. `largest_niche` = tag 本身的剩余值；`TagEncoding::Direct`。

## 二选一（[#L1076](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs#L1076)）

size 更小者胜；size 相等时 niche 更大的胜；仍相等选 tagged（codegen 更简单）。所以 `Option<bool>`、`Option<Option<bool>>` 都是 1 字节，`Result<u32, ()>`（tagged 8 字节 vs niche 不可用）走 tagged。

## 判别值读写

- **读**（CTFE：`read_discriminant`，[discriminant.rs#L52](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_const_eval/src/interpret/discriminant.rs#L52)；codegen：`codegen_get_discr`，[operand.rs#L419](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_ssa/src/mir/operand.rs#L419)）
  - `Single{index}`：常量，不读内存。`Empty` 读是 UB。
  - `Direct`：读 tag 字段，按 tag 的符号（bool 当无符号）扩展/截断到判别值类型。tag 可能比 typeck 的判别值类型窄。
  - `Niche`：`rel = tag.wrapping_sub(niche_start)`（tag 宽度内回绕，指针先转整数）；`rel <= end-start` 时变体 = `start + rel`，否则为 `untagged_variant`。niche 编码的变体，**判别值 = 变体下标**（`TagEncoding` 文档的不变量）。
  - CTFE 额外拒绝：tag 是指针且可能为空、解码到 untagged 变体本身、解码到 uninhabited 变体。
- **写**（`tag_for_variant`，[#L230](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_const_eval/src/interpret/discriminant.rs#L230)；codegen `codegen_set_discr`，[place.rs#L255](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_ssa/src/mir/place.rs#L255)）：`Direct` 写截断后的判别值；`Niche` 对 untagged 变体什么都不写，其他写 `(idx - start).wrapping_add(niche_start)`。写字段与写判别值的先后：codegen 处理 enum 聚合时先逐字段写入变体、最后 `codegen_set_discr`（[rvalue.rs#L181](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_ssa/src/mir/rvalue.rs#L181)–[#L195](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_ssa/src/mir/rvalue.rs#L195)），反过来 niche 会被字段覆盖。

## tricky 边界

- 判别值类型（`repr.discr_type()`，默认 `isize`）≠ tag 类型；显式判别值溢出由 typeck/lint 处理，layout 若算出比 typeck 更宽的 tag 直接 panic（[#L880](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs#L880) 前）。
- 负判别值 + 无 repr：例如 `-1` 和 `1` 选 i8 而不是 u8。
- `repr(C)` 的 fieldless enum 是 4 字节；`repr(C, u8)` 是 `struct{u8 tag; union}`；`repr(u8)` 带字段是“每个变体都是以 u8 开头的 repr(C) struct”的 union（字段不重排）。
- niche 只来自最大变体；其他变体里的 niche 被丢弃（`layout.largest_niche = None`）。
- `UnsafeCell` 隐藏 niche：`Option<UnsafeCell<NonZero<u32>>>` 是 8 字节，`Option<NonZero<u32>>` 是 4 字节；`RefCell<&()>` 加 `Option` 会变大（[tests/ui/layout/unsafe-cell-hides-niche.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/layout/unsafe-cell-hides-niche.rs)）。
- 指针 niche 只排除 0；`Option<&T>` 的 None 一定是 null，这是 FFI 保证（[tests/ui/abi/nullable-pointer-ffi-compat.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/abi/nullable-pointer-ffi-compat.rs)）。
- `bool`/`char`/enum tag 的 valid_range 也被外层 enum 继续消耗：`Option<Option<Ordering>>` 等可无限嵌套直到值用完。

## 对 kfrc 的含义

- Typed Body 里的 `Discriminant(place)`/`SetDiscriminant` 在降低到 SoN 时展开成上面的整数运算；SoN 不需要 enum 概念，只需要带宽度的整数、比较、select/分支。
- `match` 降低按**判别值**（不是变体下标）分支；对 niche enum 二者相同，对 `Direct` enum 显式判别值可以任意。
- 判别值依赖常量求值（显式 `= expr`），而 enum 布局依赖判别值：布局查询要能请求“只求判别值、不求 enum 本身布局”的常量求值，避免环。
