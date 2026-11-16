# 目标文件

`kfrc-object` 把已编码函数与数据对象写成 ELF 可重定位文件，使用 `object` crate。它不认识任何指令选择器。

- 每个函数一个 `.text.<符号>` 节，按 `EncodedFunction::align` 对齐，便于链接器按节回收。
- 每个数据对象一个节：只读且无重定位进 `.rodata.<符号>`，只读带重定位进 `.data.rel.ro.<符号>`，可写进 `.data.<符号>`，
  线程局部进 `.tdata.<符号>` 或（全零且无重定位时）`.tbss.<符号>`。
- 符号绑定：`Local`、隐藏的全局、默认可见的全局。
- 代码重定位：`Call` → PLT32，`PcRelative` → PC32，`GotPcRelative` → GOTPCREL，`Absolute64` → 64，
  `TlsLocalExec` → TPOFF32，`TlsGeneralDynamic` → TLSGD。数据重定位是 64 位绝对地址。
- 同一目标文件内的重复定义、同一符号既作线程局部又作普通存储，都是错误。

节、符号、重定位的顺序由输入顺序决定，与 worker 完成顺序无关。
