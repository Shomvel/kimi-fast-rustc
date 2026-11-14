# T10 新 driver 串行纵向链 — 细化

`M1 · E · 依赖：T09` · 概要见 [m1-vertical-chain.md](../m1-vertical-chain.md#t10)

## 现状

`kfrc native <input> -o <out> [--emit=asm|obj] [--jobs N]`（[native_cli.rs:21](../../../src/native_cli.rs#L21)）走 `frontend::parse → optimizer_pipeline → placement → native/parallel_native → native_program/native_object`，带 `native_cache`。`kfrc native-batch` 处理多个 `.kfr`。顶层 `kfrc rustc …`/wrapper 模式启动真实 rustc（LLVM 参考路线）。

## 新入口

选择方式在 T02 冻结；建议 `kfrc native --frontend=v2 <input.rs> …`，不改变现有命令的默认行为。

串行阶段：

```text
read → syntax::parse → resolve → semantics::check → body::build
     → instantiate（M1：非泛型，恒等） → son::build → son::optimize（M1：只开纯值规则）
     → machine::lower → encode → object::emit → link（可选）
```

- 每个阶段的错误映射到 [错误分类](../../design/ids.md)，CLI 退出码区分：源程序错误 1、未支持 2、内部错误 101、外部工具失败 3（T02 冻结具体值）。
- `--frontend=v2` 下不读、不写现有 `native_cache`（schema `native-x86_64-elf-v2`），避免新旧链共享缓存。
- 来源清单：每次运行写 `kfrc-provenance.json`（输入摘要、kfrc 版本、阶段耗时、产物哈希、每个对象的生产者）。

## 测试

- 公共 CLI 跑通 T03 的全部 `run` 用例与 `reject` 用例。
- 给 `--frontend=v2` 一个旧 `.kfr` 专有语法的文件 → 明确报“未支持”，不改走旧链。
- `--emit=obj` 产物中的每个目标函数符号由 kfrc 生成（检查来源清单与 `readelf -s`）。

## 完成标志

源码 → 对象 → 链接 → 执行经公共 CLI 完成；诊断、来源清单、退出码可从 CLI 取得。
