# 内建宏：注册机制、format_args 降低、derive 与目标项目清单

源码：`compiler/rustc_builtin_macros/src/`（`lib.rs`、`format.rs`、`assert.rs`、`edition_panic.rs`、`source_util.rs`、`env.rs`、`cfg.rs`、`deriving/`），降低在 `compiler/rustc_ast_lowering/src/format.rs`，lang item 表 `compiler/rustc_hir/src/lang_items.rs`。

## 注册与绑定

- [`register_builtin_macros`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/lib.rs#L64) 按名字登记展开器：
  - bang（`LegacyBang`，[L76](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/lib.rs#L76)）：`asm` `assert` `cfg` `cfg_select` `column` `compile_error` `concat` `concat_bytes` `const_format_args` `core_panic` `env` `file` `format_args` `format_args_nl` `global_asm` `include` `include_bytes` `include_str` `iter` `line` `log_syntax` `module_path` `naked_asm` `option_env` `pattern_type` `std_panic` `stringify` `trace_macros` `unreachable`；
  - 属性（`LegacyAttr`，[L110](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/lib.rs#L110)）：`derive` `derive_const` `test` `bench` `test_case` `cfg_eval` `cfg_accessible` `global_allocator` `alloc_error_handler` `define_opaque` `eii*` `autodiff_*`；
  - derive（`LegacyDerive`，[L131](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/lib.rs#L131)）：`Clone` `Copy` `ConstParamTy` `Debug` `Default` `Eq` `Hash` `Ord` `PartialEq` `PartialOrd` `CoercePointee` `From`；另有 `quote`（内建过程宏）和 contracts 属性。
- **名字不是魔法**：库里用 `#[rustc_builtin_macro]` 声明同名宏项，名称解析照常找到它（前奏、`core::format_args` 路径等），[`compile_macro`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/macros.rs#L1156) 发现 `builtin_name` 后换成登记的展开器；找不到登记项报 `CannotFindBuiltinMacroWithName`。`#![no_implicit_prelude]` 下前奏中只放行内建宏；内建宏的 `ExpnData.hide_backtrace` 为真（诊断不提示 `-Zmacro-backtrace`）。
- `println!`、`print!`、`format!`、`write!`、`writeln!`、`vec!`、`assert_eq!`、`matches!`、`todo!`、`dbg!`、`thread_local!` 等是 library 里的普通宏（推断：library 不在本 checkout，但上面的内建清单里没有它们）。`panic!` 是内建 `std_panic`/`core_panic`，按调用处 edition 分派到 `$crate::panic::panic_2015!`/`panic_2021!`（[edition_panic.rs L17](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/edition_panic.rs#L17)）；edition 判定 [`use_panic_2021`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/edition_panic.rs#L70) 跳过带 `allow_internal_unstable(edition_panic)` 的中间宏。
- 急切展开：`format_args!`、`concat!`、`env!`、`include*!` 的参数先完整展开（[`expr_to_spanned_string`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/util.rs#L83)），参数里的宏暂时不可解析就 `Retry`。serde 用到 `include!(concat!(env!("OUT_DIR"), "/private.rs"))`，三层嵌套都要支持。

## `format_args!` → `ast::FormatArgs`

- [`parse_args`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/format.rs#L71)：格式串表达式 + 位置参数 + `name = expr`；记录格式串是否直接是字面量（`is_direct_literal`）。
- [`make_format_args`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/format.rs#L159)：急切展开得到字符串，`rustc_parse_format` 切分；`{x}` 找不到同名参数时**仅当格式串是源码字面量**才隐式捕获变量 `x`（标识符带格式串的 span，[L380–L401](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/format.rs#L380)）；校验未用/越界参数。
- 产物 [`FormatArgs { span, template: Vec<FormatArgsPiece>, arguments, uncooked_fmt_str, is_source_literal }`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ast/src/format.rs#L44)，piece = `Literal(Symbol)` | `Placeholder { argument, span, format_trait, format_options }`；以 `ExprKind::FormatArgs` 留在 AST 中（[L1056](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/format.rs#L1056)）。`format_args_nl!` 追加 `\n`。

## `FormatArgs` → HIR（1.94 的形状）

[`lower_format_args`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ast_lowering/src/format.rs#L12) 给 span 加 `DesugaringKind::FormatLiteral`；`-Z flatten-format-args` 缺省为真（[options.rs L2380](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_session/src/options.rs#L2380)），先把嵌套 `format_args!` 和字面量参数内联进模板。[`expand_format_args`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ast_lowering/src/format.rs#L279)：

1. 只有字面量：`<core::fmt::Arguments>::from_str("…")`（原本有参数则用 `from_str_nonconst`）。
2. 否则生成字节码模板：字面量分块（长度 <0x80 用 1 字节，否则 `0x80` + u16 LE，按 UTF-8 边界切）；占位符首字节 `0xC0 | 位`：非缺省选项时附 u32 flags（fill 字符、`+`/`-`、`#`、`0`、debug hex、width/precision 是否存在、对齐），width/precision 为 u16（可为参数间接）；参数下标不连续时附 u16 下标；末尾 `0`。
3. 参数：`super let args = (&a0, &a1, …); super let args = [<Argument>::new_display(args.0), <Argument>::new_lower_hex(args.1), …];` 每个唯一的 `(参数, trait)` 一项（[`make_argument`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ast_lowering/src/format.rs#L232)；width/precision 参数用 `from_usize`）。
4. 调用：编译器生成的 `unsafe { <Arguments>::new(b"…", &args) }`。

类型是 lang item `FormatArguments`/`FormatArgument`（[lang_items.rs L332](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir/src/lang_items.rs#L332)），方法按名字查。实际输出见 [tests/ui/unpretty/exhaustive.hir.stdout L403–L411](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/unpretty/exhaustive.hir.stdout#L403)。

## derive

- 公共框架 [`TraitDef`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/deriving/generic/mod.rs#L201)/[`MethodDef`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/deriving/generic/mod.rs#L236) → [`expand_ext`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/deriving/generic/mod.rs#L485) → [`create_derived_impl`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/deriving/generic/mod.rs#L593)：生成带 `#[automatically_derived]` 的 impl，每个类型参数加 trait 约束；enum 方法先比较 `::core::intrinsics::discriminant_value(self)`（[L1265](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/deriving/generic/mod.rs#L1265)）。在 AST 上工作，不经过 token。
- `Clone`（[clone.rs L11](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/deriving/clone.rs#L11)）：同容器也 derive 了 `Copy`（`has_derive_copy`）且无类型参数时生成 `*self` 加 `AssertParamIsClone` 断言；否则逐字段 clone；union 要求 `Copy`。`#[derive(Clone)] #[derive(Copy)]` 这个顺序拿不到 Copy 信息（源码 FIXME #124794）。
- `Debug`（[debug.rs L11](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/deriving/debug.rs#L11)）：无字段 → `Formatter::write_str`；≤5 字段 → `Formatter::debug_struct_field{N}_finish`/`debug_tuple_field{N}_finish`；更多 → `debug_struct_fields_finish`/`debug_tuple_fields_finish`。这些是 core 的内部辅助方法。

## 其他常用内建

| 宏 | 行为要点 |
|---|---|
| [`assert!`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/assert.rs#L17) | 有消息 → `if !cond { panic!(msg) }`，panic 路径按 edition 选 `$crate::panic::panic_2021!` 或调用处的 `panic!`；无消息 → `$crate::panicking::panic("assertion failed: " + pprust::expr_to_string(cond))`，**消息文本来自 rustc 的 pretty printer** |
| [`stringify!`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/source_util.rs#L79) | `pprust::tts_to_string`，同样依赖 pretty printer 的空格规则 |
| [`line!`/`column!`/`file!`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/source_util.rs#L30) | 用最外层调用处（`expansion_cause`）；`column!` 从 1 起 |
| [`include!`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/source_util.rs#L106)/`include_str!`/`include_bytes!` | 相对路径以调用处源文件目录为基准（[`resolve_path`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/base.rs#L1388)，不卫生）；文件计入依赖 |
| [`env!`/`option_env!`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/env.rs#L20) | 先查 `--env-set`，再查进程环境；写入 `env_depinfo` |
| [`cfg!`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/cfg.rs#L20) | 布尔字面量 |
| [`concat!`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/concat.rs#L10) | 急切展开后拼接字面量 |
| [`asm!`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/asm.rs#L580) | 模板 + 操作数解析（[`parse_asm_args`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_parse/src/parser/asm.rs#L169)），产物是 `InlineAsm` 节点，需要后端支持 |

## ripgrep 15.2.0 的实际用量

扫描范围：ripgrep 各 crate `src/` + `build.rs`，以及 Linux 默认 features 下 normal/build 闭包中 25 个 registry crate 的 `src/`/`build.rs`（813 个 `.rs` 文件，含 `#[cfg(test)]` 代码，计数是上界）。方法：bang 调用用 `grep -oE '\b[a-z_][a-z0-9_]*!\s*[(\[{]'` 计数，派生拆分 `#[derive(...)]` 列表计数，属性按 `#[name` 前缀计数；T31 冻结 library 时可用同样方法核实 std 的需求。

- 派生：`Debug` 797、`Clone` 655、`PartialEq` 212、`Eq` 199、`Copy` 156、`Default` 84、`Hash` 30、`PartialOrd`/`Ord` 各 19；`Serialize`/`Deserialize`/`Error` 只出现在 doc 注释或测试中。
- 内建 bang：`cfg!` 190、`format_args!` 48（直接写的；`println!`/`write!` 等间接使用更多）、`stringify!` 62、`include_bytes!` 33、`concat!` 24、`include_str!` 17、`compile_error!` 14、`asm!` 7（全在 zmij，x86_64 下 `asm!("/*{0}*/", inout(reg) x)` 这类空模板优化屏障）、`module_path!` 6、`include!` 5、`env!` 4、`option_env!` 1。
- 属性：`#[cfg]` 4592、`#[inline]` 3121、`#[cfg_attr]` 1262、`#[repr]` 390、`#[target_feature]` 126、`#[macro_export]` 64、`#[macro_use]` 24、`#[test]` 1349（测试）。
- 库宏（`macro_rules`）：`assert!`/`assert_eq!`/`vec!`/`write!`/`matches!`/`cfg_if!`（275）以及大量 crate 内部宏（`tri!`、`forward_to_deserialize_any!`、`impl_*`…）。
