# W01 `?` 链与 `From` 转换的错误路径

## 1. 程序

```rust
pub struct FieldError { pub msg: String, pub pos: usize, pub code: u32 }   // 40 字节
pub enum Error { Field(FieldError), Missing(&'static str) }
impl From<FieldError> for Error { fn from(e: FieldError) -> Self { Error::Field(e) } }
#[inline(never)] pub fn next_value(&mut self) -> Result<u32, FieldError> { … }   // Reader 的方法
#[inline(never)] pub fn parse_config(r: &mut Reader) -> Result<Config, Error> {
    let a = r.next_value()?;  let b = r.next_value()?;
    let c = r.next_value()?;  let d = r.next_value()?;
    Ok(Config { a, b, c, d })
}
// 对照：parse_config_match 用 match 手写同一逻辑：Err(e) => return Err(Error::Field(e))
```

完整源码 [scripts/w01.rs](scripts/w01.rs)，测量脚本 [scripts/w01.sh](scripts/w01.sh)。`Result<u32, FieldError>`、`Error`、`Result<Config, Error>` 都借 `String` 容量的 niche，均为 40 字节。

## 2. rustc 现在怎么做（实测）

`Try::branch`、`FromResidual::from_residual`、`From::from` 都已被 MIR 内联，但每个字段的错误路径仍是 7 次整值赋值（match 版是 3 次：`_6 = move ((_3 as Err).0)`、`_7 = Error::Field(copy _6)`、`_0 = Err(move _7)`）：

```text
== MIR: first error block of parse_config (Storage lines removed)
      bb10: {
          _21 = move ((_3 as Err).0: FieldError);
          _22 = Result::<Infallible, FieldError>::Err(copy _21);
          _2 = ControlFlow::<Result<Infallible, FieldError>, u32>::Break(move _22);
          _4 = move ((_2 as Break).0: std::result::Result<std::convert::Infallible, FieldError>);
          _23 = copy ((_4 as Err).0: FieldError);
          _24 = Error::Field(copy _23);
          _0 = Result::<Config, Error>::Err(move _24);
```

`_2 = Break(move _22); _4 = move ((_2 as Break).0)` 就是“构造后立即投影”。逐 pass dump（`RUSTC_BOOTSTRAP=1 -Zdump-mir`）显示它为什么没被折叠：

```text
  after GVN: the first Break construction and the block it jumps to
      bb19: {
          _3 = ControlFlow::<Result<Infallible, FieldError>, u32>::Break(move _42);
          goto -> bb18;
      bb18: {
          _6 = discriminant(_3);
          switchInt(move _6) -> [0: bb3, 1: bb4, otherwise: bb2];
```

GVN 时 `Break` 与 `Continue` 在合并块 `bb18` 汇合再 switch，GVN 看不到已知变体；JumpThreading（排在 GVN 之后，见 `rustc_mir_transform/src/lib.rs` 的 `run_optimization_passes`）复制路径后，构造和投影才落在同一条直线路径上，之后不再有 GVN。

```text
== pre IR
191 inst  alloca=29  memcpy=29  lifetime=62  assume=0   call=4   invoke=0   landingpad=0  load=12   store=13   parse_config
103 inst  alloca=13  memcpy=13  lifetime=26  assume=0   call=4   invoke=0   landingpad=0  load=8    store=5    parse_config_match
  12parse_config sizes:    1 alloca 16B   28 alloca 40B    1 memcpy 16B   28 memcpy 40B
== o3 IR
79 inst  alloca=4   memcpy=4   lifetime=12  assume=0   call=4   invoke=0   landingpad=0  load=8    store=13   parse_config
60 inst  alloca=4   memcpy=4   lifetime=11  assume=0   call=4   invoke=0   landingpad=0  load=8    store=5    parse_config_match
  12parse_config sizes:    4 alloca 40B    4 memcpy 28B
  18parse_config_match sizes:    4 alloca 40B    4 memcpy 40B
== LLVM IR passes that change a function's alloca/memcpy count (start = unoptimized IR)
  parse_config: a29m29 >SROAPass> a28m28 >MemCpyOptPass> a28m4 >InstCombinePass> a4m4
  parse_config_match: a13m13 >SROAPass> a12m12 >MemCpyOptPass> a12m4 >InstCombinePass> a4m4
== x86 asm (rustc x86)
  3w0112parse_config: inst=75 movups=16 call=4 frame=48
  3w0118parse_config_match: inst=65 movups=10 call=4 frame=160
```

- 未优化 IR 中每个字段 7 次 40 字节 memcpy（28/4），与 MIR 的 7 次赋值一一对应（cargo `visit_map` 是 6 次，链的形状相同）。
- **O3 后错误路径仍有一次 40 字节拷贝。** 每次调用先写进自己的 40 字节临时槽，错误时再拷进 sret 槽：`?` 版被 SROA 拆成 `i64` + `i32` + 28 字节 memcpy，match 版是整段 40 字节 memcpy。拷贝时字节原样搬运，不改 tag 字——说明 `Err(Field(e))` 与 `Err(e)` 的编码逐字节相同，直接把 sret 指针交给 `next_value` 就能零拷贝。LLVM 没这么做（推断：MemCpyOpt 的 call-slot 只看与调用同块、无条件执行的 memcpy，这里的 memcpy 在错误分支里）。
- match 版的 4 个临时槽没有合并（frame 160 对 48）。脚本列出 O3 IR 的 lifetime 标记：match 版 `%_3` 只有 `end=1`（另 3 个槽是 2），推断栈着色因此保守处理。
- 版本差异：同一源码的 niche 常量，1.94.1 是 `-9223372036854775808`（Ok）/`-9223372036854775807`（Config Ok），1.99.1 是 `-1`/`-2`；1.99.1 在 aarch64 主机上的 IR 也是 `-1`/`-2`，所以差异来自版本而非目标。

## 3. kfrc 编译计划

**Body。** `?` 降成 `match Try::branch(e) { Continue(v) => v, Break(r) => return FromResidual::from_residual(r) }`，与 rustc 的 MIR 构建相同。所有 40 字节局部都没有被取地址，全部是 SSA 值。

**R 图（`parse_config` 无泛型参数，R = S；`branch`/`from_residual` 是具体实例，按 W02 细化 2 内联它们的 S 图）。** 第一个字段，内联之后、优化之前：

```text
n10 = Call(Reader::next_value) ctl=n2 mem=[P0:n3] (n1) : (Control, Control?, Effect[P0], Result<u32, FieldError>)
n11 = VariantSwitch ctl=n10.0 (n10.3) : (Control, Control)        // Ok / Err
n12 = VariantField[Ok, 0](n10.3) : u32
n13 = MakeVariant{ControlFlow<Result<Infallible, FieldError>, u32>, Continue}(n12)
n14 = VariantField[Err, 0](n10.3) : FieldError
n15 = MakeVariant{Result<Infallible, FieldError>, Err}(n14)
n16 = MakeVariant{ControlFlow<…>, Break}(n15)
n17 = Region(n11.0, n11.1)
n18 = Phi(n17) (n13, n16) : ControlFlow<Result<Infallible, FieldError>, u32>
n19 = VariantSwitch ctl=n17 (n18) : (Control, Control)            // Continue / Break
n20 = VariantField[Break, 0](n18) : Result<Infallible, FieldError>
n21 = VariantField[Err, 0](n20) : FieldError                       // from_residual
n22 = MakeVariant{Error, Field}(n21)                                // From::from
n23 = MakeVariant{Result<Config, Error>, Err}(n22)
n24 = Return ctl=n19.1 mem=[P0:n10.2] (n23)   // P0：M2 为 Unknown，M6 起为 Unique(r)
```


**R 级优化。**

| 步 | 规则 | 结果 |
|---|---|---|
| 1 | `VariantSwitch` 的输入是 `Phi`，且 Phi 的每个输入都是已知变体的 `MakeVariant` → 按前驱拆分（细化 D1） | `n11.0` 直接接 Continue 目标，`n11.1` 直接接 Break 目标；`n17`/`n18`/`n19` 删除 |
| 2 | `VariantField[Break,0](MakeVariant Break(n15))` → `n15`；`VariantField[Err,0](n15)` → `n14`；`VariantField[Continue,0](n13)` → `n12` | 构造后投影全部折叠，不依赖 pass 顺序 |
| 3 | DCE | 每个字段剩 `n10 n11 n12 n14 n22 n23 n24` |

结果：`Return(MakeVariant{Result<Config,Error>, Err}(MakeVariant{Error, Field}(VariantField[Err,0](n10.3))))`，move 链在值形式下不存在。**S 级**无孔位，S 图就是 R 图；`next_value` 不内联，只读它的摘要。

**L 级。** 三种 40 字节类型都是 `Memory`，用 value slot。

| 值 | 槽 | 依据 |
|---|---|---|
| `n10.3`（第 k 次调用的结果） | 原设计：每个调用一个槽 → 4 个 | 规则 1、4 |
| `n23` | sret 槽 `%0` | 规则 3：唯一使用是 Return |
| `n23` 的降低 | 查 `VariantEncoding`：`Err(Field(x))` 与 `Result<u32,FieldError>::Err(x)` 的 payload 偏移相同、都不写 tag | 变成整槽 `CopyMem(%0 ← slot(n10.3), 40)` |
| 合并 | sret 槽在最终 Return 前不被读写，与 `n10.3` 的活跃区间不冲突 → `slot(n10.3) := %0`，拷贝变成自拷贝后删除 | 细化 D2 |

**期望机器码（推断，x86，1.94.1 布局）：** 每个字段 `mov rdi, rbx; mov rsi, r14; call next_value; cmp [rbx], r15; jne .ret; mov ebp, [rbx+8]`（`r15` 预装 Ok 的 niche 常量），错误路径零指令；成功路径最后写 4 个 `u32` 和 Ok 标记。约 43 条，对 rustc 的 75 条；frame 0（rustc 48）。

## 4. 代价核算

| 项 | rustc | kfrc（推断） |
|---|---|---|
| 40 字节临时 | MIR 29 个局部；未优化 IR 28 个 alloca + 28 次 memcpy | 0（值）；L 级 4 个结果全落在 sret 槽 |
| 消除 move 链 | SROA、MemCpyOpt、InstCombine 三个 pass 改变计数 | 构建期窥孔 + 一次 Phi 拆分 |
| 图规模 | 191 条未优化 IR | R 图约 65 个节点（每字段约 15），优化后约 32 |
| 错误路径代码 | 每字段 6 条拷贝指令（`movups` ×4 + `movq` + `movl`） | 0 |
| 栈 | 48（`?` 版）/ 160（match 版，未合并） | 0：槽按 SSA 活跃区间分配，不依赖 lifetime 标记 |
| 缓存 | — | R 图哈希含被内联的 `branch`/`from_residual`/`from` 的图哈希；机器码还依赖 `Error` 的编码，见 D3 |

## 5. 设计细化

| # | 问题 | 证据 | 建议改法 | 文件 |
|---|---|---|---|---|
| D1 | R 级规则只有“`VariantSwitch` 已知变体 → 直接跳转”。`?` 内联后，`ControlFlow` 是经过合并点的 `Phi(MakeVariant Continue, MakeVariant Break)`，switch 的输入不是已知变体，聚合折叠永远等不到机会——这正是 MIR 的 phase ordering 问题换了个形式 | GVN 后的 dump：`bb19` 构造 `Break` 后 goto 合并块 `bb18` 再 switch；最终 MIR `bb10` 保留构造 + 投影 | R 级加“switch 穿过 Phi 拆分”：`VariantSwitch`/`If` 的输入是 Phi，Phi 的输入都是已知变体（或常量），且 Region 只被这个 switch 和 Phi 使用时，把各前驱直接接到对应目标，Phi 的使用者改用前驱的值。规则与聚合折叠在同一 worklist 里，互相产生种子 | optimization-pipeline.md（R 级表）、nodes.md |
| D2 | 原地构造规则 3 要求“值唯一的使用是 sret 返回”。这里被返回的是 `n10.3` 重新包装后的值，而 `n10.3` 在 Ok 路径上还有使用，规则 3 不适用；按规则 1–4 会分配 4 个槽、留下 4 次 40 字节拷贝——与 rustc O3 相同，没有赢 | O3 IR `4 memcpy 28B` + 标量 store；x86 `movups=16`；match 版 `4 memcpy 40B` | 规则 3 后加“**拷贝亲和合并**”：L 级 enum 降低后，两个槽之间如果只有整槽、同偏移的 `CopyMem`（包括重编码恒等的 `MakeVariant(VariantField(v))`），且目标槽（sret、`Indirect` 实参槽、Phi 槽）在源值活跃期间不被读写，就合并两槽。重编码恒等由 `VariantEncoding` 答案判定：payload 偏移相同，且目标变体不需要写 tag | ownership-drop-unwind.md（大值与原地构造） |
| D3 | D2 的结果依赖 `Error`、`FieldError` 的布局，但 `parse_config` 没有泛型参数、孔位表为空，现设计的机器码键里没有这些布局。给 `Error` 加一个变体，R 图哈希不变，而编码（和能不能零拷贝）可能变了 | 同一源码，1.94.1 用 `-9223372036854775808`/`…807`，1.99.1 用 `-1`/`-2`：编码不是源码层事实 | L 级对**所有**类型（不只是含 `Param` 的类型）的布局查询都登记成闭合孔位，进机器码键；与 W06 D2 的“闭合孔位”统一 | generics-specialization.md（孔位）、cache.md（机器码键） |
