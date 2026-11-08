# Cargo 热点画像

A 臂（Cargo 0.95.0 stock release）用户态 cycles 采样，方法同 [hotspots-ripgrep.md](hotspots-ripgrep.md)，每份 1,045–30,561 个样本。

## 各 workload 的类别占比（% 用户态 cycles，`matrix 6` 原样输出）

| workload | C libraries (OpenSSL etc.) | allocator | libc mem*/str* | std/core other | libc pthread (called by C libraries) | hashing (SipHash etc.) | serde / JSON | hash/btree maps | TOML parse | CLI parsing/help | rest |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| build-help | 13.5 | 10.1 | 13.0 | 2.9 | 1.1 | 0.4 | 0.1 | 0.4 | 1.1 | 8.1 | 49.2 |
| lockfile | 0.1 | 20.0 | 8.1 | 14.8 | 0.6 | 4.5 | 15.9 | 8.7 | 0.0 | 0.0 | 27.4 |
| metadata-full | 1.2 | 18.2 | 8.5 | 7.1 | 1.3 | 6.8 | 1.7 | 3.8 | 6.0 | 1.4 | 44.1 |
| metadata-nodeps | 33.0 | 12.0 | 6.2 | 1.2 | 12.3 | 0.3 | 0.5 | 0.4 | 0.8 | 0.7 | 32.6 |
| noop-build | 22.1 | 10.9 | 5.3 | 3.8 | 8.3 | 4.8 | 0.8 | 2.0 | 2.6 | 0.0 | 39.4 |
| package-list | 28.8 | 7.4 | 2.8 | 2.7 | 11.3 | 3.0 | 0.8 | 1.0 | 1.4 | 0.1 | 40.7 |
| tree | 1.9 | 17.6 | 7.3 | 7.0 | 1.3 | 7.4 | 1.3 | 3.8 | 6.0 | 0.2 | 46.2 |

`rest` 里约三成到四成是“每个符号都不到 0.1%”的长尾；“resolver + semver” 在 metadata-full / tree / lockfile 分别是 4.2% / 5.2% / 5.8%（逐 workload 明细里，未达 6% 的阈值）。

## 按任务的热路径

| 任务（workload） | 热点 | 说明 |
|---|---|---|
| 启动（metadata-nodeps、noop-build、package-list、build-help） | libcrypto 13.5–33.0%，libc pthread 1.1–12.3%（metadata-nodeps 中 `pthread_rwlock_unlock` 7.22%、`rdlock` 4.89%），glibc malloc/mem* | C 库占用户态大头；推断是 curl/libgit2 的 OpenSSL 全局初始化（离线也执行），没有调用图，未证实 |
| TOML 解析（metadata-full、tree） | TOML 6.0%：`toml_parser::lexer::Lexer::into_vec` 0.91%、`Lexer::next` 0.83%、`toml::de::parser::value::on_scalar` 0.63% | winnow/toml_parser 的词法与状态机，散在很多小函数 |
| 依赖解析（lockfile） | serde/JSON 15.9%（`SliceRead::parse_str` 3.55%，读 index 缓存）、`InternedString::from` 3.65%、SipHash 4.5%、BTreeMap/HashMap 8.7%、resolver+semver 5.8% | 平坦；JSON 反序列化与字符串驻留比 resolver 本身还重 |
| 带锁解析（metadata-full、tree） | SipHash `Hasher::write` 3.89% + 2.63%、`PackageId::cmp` 1.63%、`im_rc` B 树查找 0.64% | 哈希与比较密集 |
| 文件系统（noop-build、package-list） | 用户态里只看到 libc 包装；内核占 33–35% | 见 [workloads.md](workloads.md) |
| git / 网络 | 离线 workload 不触发 | 前期也没有在线测量 |

**Cargo 没有单一热点。** 单个 Rust 符号最多 3.89%；解析类 workload（lockfile、metadata-full、tree）里 allocator 占 17.6–20.0%，libc mem* 占 7.3–8.5%。显式 SIMD 几乎不出现（memchr SIMD ≤0.4%），所以 S0 最多 1.035×（cycles）。

## 内联去掉后多出来的是什么

lockfile 的 L 臂（`-Clto=off`，cycles 1.268）与 A 的逐符号对比（[compare-profiles.sh](scripts/compare-profiles.sh)，“Δ” 是折算到 A 总 cycles 的百分点；同一函数在两边一个被 perf 还原、一个没有还原时会各占一行）：

| symbol | base % | arm % | arm % × ratio | Δ (pp of base) |
|---|---:|---:|---:|---:|
| cargo core::ops::function::FnOnce::call_once | 0.00 | 6.10 | 7.73 | +7.73 |
| cargo alloc::collections::btree::map::IntoIter<K,V,A>::dying_next | 0.00 | 5.07 | 6.43 | +6.43 |
| cargo _ZN71_$LT$core..hash..sip..Hasher$LT$S$GT$$u20$as$u20$core..hash..Hasher$GT$5write | 4.46 | 0.00 | 0.00 | -4.46 |
| libc.so.6 __memcmp_evex_movbe | 3.14 | 5.59 | 7.09 | +3.95 |
| cargo <core::hash::sip::Hasher<S> as core::hash::Hasher>::write | 0.00 | 3.04 | 3.85 | +3.85 |
| cargo <alloc::collections::btree::map::Iter<K,V> as core::iter::traits::iterator::Iterator>::next | 0.00 | 2.40 | 3.04 | +3.04 |
| cargo alloc::collections::btree::search::<impl alloc::collections::btree::node::NodeRef<BorrowType,K | 0.00 | 1.98 | 2.51 | +2.51 |
| cargo std::sync::poison::mutex::Mutex<T>::lock | 0.00 | 1.43 | 1.81 | +1.81 |

- 新出现的独立函数是闭包 `FnOnce::call_once`、BTreeMap 迭代器的 `next`/`dying_next`、B 树查找、`Mutex::lock`：都是 `#[inline]` 的小泛型函数，原本跨 CGU 内联进调用者。
- `memcmp` 变多：字符串比较原来在内联后被常量长度或前缀化简，推断。
- I 臂（cycles 1.301）的对比结论相同（多出 BTreeMap 迭代器、`Mutex::lock`、`RawVecInner::try_allocate_in`、`serde_json` 的 `parse_whitespace`）。
- 与 rg 不同，Cargo 的 I/L 代价不是 intrinsic：`core::arch` 被调用的占比只有 0.3–1.4%。

## 对 kfrc 的含义

- Cargo 的 runtime 主要取决于**内联覆盖面**（闭包、迭代器、集合、锁、serde 访问器），以及分配次数（allocator 18–20%）。kfrc 中层的堆分配消除和 move 转发（[design/goals/runtime.md](../../design/goals/runtime.md)）是有机会做到 <1.0 的地方。
- 启动类 workload 里 C 库占用户态 13.5–33.0%（另有 libc pthread 1.1–12.3%），kfrc 碰不到，这部分两边相同，会把比值拉向 1.0。
