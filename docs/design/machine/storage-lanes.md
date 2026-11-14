# 存储 lane

完整契约见 [表示、存储与 ABI](../ir/representation.md)。实现位于
`crates/kfrc-cranelift/src/export/promote.rs` 和 `memory.rs`。

存储的根是图的栈槽、以寄存器 lane 到达的聚合参数与调用结果、存储地址的 Phi。
常量偏移的读写、生命周期标记、按值传递、间接返回与常量长度复制可以使用 SSA lane。
收集的字节范围允许重叠，读取提取位，写入保留范围外的位，ABI 也通过相同接口读取和写入字节。
每条 lane 是 Cranelift 变量，SSA 构造负责循环和分支的块参数。

常量长度 `memcpy`/`memmove` 在不超过布局展开预算且返回指针未被使用时可以传递 lane。
源与目的可以使用不同的 lane 划分，也可以只有一侧是 lane；所有源字节先读后写。
这包含复制的原始字节，既不按 Rust 类型解码，也不要求填充或 `MaybeUninit` 内部字节有效。

地址被存储、比较、传作指针或用变量偏移访问的存储仍用内存。
存储地址 Phi 使用 lane 的条件仍是：所有输入均可提升、Region 不在环上、合并后不再直接访问原输入。
否则 Phi 仍是指针块参数，各输入保留内存，以维持别名关系。
只被间接返回且所在块没有其他效果的 Phi 沿入边写到调用方的存储，各变体可以保留各自的写入范围。

`KFRC_CRANELIFT_STORAGE_REPORT=1` 每个函数输出一行 `kfrc-storage {...}`，包含提升数量、内存数量及其原因。
`KFRC_CRANELIFT_PROMOTE=0` 保留所有存储在内存中，用于独立差分。
