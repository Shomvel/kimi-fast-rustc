# T05 定义身份、作用域与模块解析 — 细化

`M1 · A · 依赖：T04` · 概要见 [m1-vertical-chain.md](../m1-vertical-chain.md#t05) · rustc 笔记：[macros-resolve/name-resolution](../../research/rustc/macros-resolve/name-resolution.md)、[imports](../../research/rustc/macros-resolve/imports.md)、[borrowck/lifetimes-origin](../../research/rustc/borrowck/lifetimes-origin.md)

## 设计要点

**三态查找从 M1 起就需要。** 查找返回 `Determined(Res) | Failed | Undetermined`。即使没有宏，待定的单项导入和 glob 也会产生 `Undetermined`；一次性全局扫描式的实现到 T23 接宏时会被推倒重来。

**按片段驱动。** “收集定义 + 建模块图”按片段进行（对应 rustc 的 `DefCollector` + reduced graph）。模块记录未展开的宏调用集合；M1 这个集合恒为空，但驱动循环已经是不动点形式。

**导入不动点在 M1 就做：** 单项导入、glob 推送、重复定义、导入环、glob 环、受限遮蔽产生的歧义。finalize 复核作为内部不变量检查保留。

**晚期解析（函数体内）**照搬 rustc 的 rib 规则：块内项先于局部变量可见；局部变量和标签按 macro_rules 规范化比较 ctxt；类型/值/宏三个命名空间。

**签名级生命周期**也在这里产出（来自 borrowck 笔记）：生命周期省略规则在 AST 上执行，early-bound/late-bound 判定，`'static`。M1 若函数不收发引用，结果只有 `'static`，但接口在 M1 定义。

## DefKey 与 hygiene

rustc 的 `DefKey` 不带 hygiene：同名定义的消歧编号按创建顺序分配，“完成顺序不影响 DefKey”依赖展开顺序确定。kfrc 采用同样做法，若将来并行展开，按展开树中的位置定序。[ids.md](../../design/ids.md) 按此修订。

## 产物

```rust
pub struct ResolvedCrate {
    pub defs: IndexVec<DefId, DefData>,            // DefKey、kind、父模块、可见性、span
    pub modules: …,                                 // 模块图与各命名空间的绑定
    pub path_res: HashMap<HirId, Res>,              // 每个路径表达式/类型的解析结果
    pub local_res: HashMap<HirId, LocalBindingId>,  // 体内变量引用 → 绑定
    pub fn_lifetimes: HashMap<DefId, LifetimeInfo>, // 省略结果、early/late 标记
}
```

## 测试

多模块（`mod a { pub fn f() } use a::f;`）、glob 导入与遮蔽、导入环报错、重复定义、私有项不可见、遮蔽（`let x = 1; let x = x + 1;`）、前向/递归/互相递归调用、块内项先于同名局部变量。拒绝类别与 rustc 错误码比对（E0425、E0432、E0433、E0603、E0428、E0252 等）。

## 完成标志

以上正反例通过；改变源文件中项的顺序不改变任何 `DefKey`；三态接口有单元测试覆盖 `Undetermined → Determined` 的推进。
