# 全函数调查分类

计数单位为单态化函数/静态项的首个失败；intrinsic观察单列，不能相加当作失败总数。

## ripgrep

33/33 个target units报告完整；35422个项，1369个首错。

| 缺口 | 首错项数 | 涉及target units | 涉及package versions | 代表函数 |
|---|---:|---:|---:|---|
完整报告的Cranelift配置：speed=33 units

| SIMD | 548 | 13 | 13 | packed::teddy::generic::Teddy::<8>::verify::<core::arch::x86_64::__m128i> |
| i128-u128 | 241 | 10 | 10 | util::prefilter::Builder::ascii_case_insensitive |
| float-other | 171 | 4 | 4 | alloc::collections::btree::node::Handle::<alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Mut<'_>, std::string::String, serde_json::Value, alloc:: |
| union | 134 | 8 | 8 | memchr::memmem::searcher::Searcher::new::<memchr::arch::all::packedpair::DefaultFrequencyRank> |
| float-ABI | 83 | 6 | 6 | util::NiceDuration::fractional_seconds |
| unreachable-control | 71 | 4 | 4 | alloc::collections::btree::search::<impl alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Mut<'_>, alloc::boxed::Box<[u8]>, usize, alloc::collectio |
| aggregate-constant | 31 | 9 | 9 | <std::sys::stdio::unix::Stderr as std::io::Write>::write_all |
| DST-tail | 30 | 7 | 7 | alloc::sync::Arc::<dyn ahocorasick::AcAutomaton>::drop_slow |
| DST-size_of_val | 11 | 3 | 3 | <alloc::sync::Weak<dyn ahocorasick::AcAutomaton, &alloc::alloc::Global> as core::ops::Drop>::drop |
| TLS | 10 | 5 | 5 | std::hash::RandomState::new::KEYS::{constant#0}::{closure#1} |
| verification-other | 9 | 2 | 2 | core::arch::x86_64::_mm256_permute2x128_si256::<33> |
| atomic_or | 8 | 2 | 2 | std::sync::atomic::AtomicUsize::fetch_or |
| unwind | 7 | 3 | 3 | std::thread::lifecycle::spawn_unchecked::<'_, {closure@crates/cli/src/process.rs:282:32: 282:39}, process::CommandError>::{closure#1} |
| uninhabited-value | 5 | 3 | 3 | <std::result::Result<std::vec::in_place_drop::InPlaceDrop<regex_syntax::hir::Hir>, !> as std::ops::FromResidual<std::result::Result<std::convert::Infallible, !>>>::from_residual |
| compiler-panic | 4 | 1 | 1 | meta::strategy::ReverseSuffix::try_search_half_start |
| LLVM-intrinsic | 2 | 1 | 1 | core::arch::x86_64::_mm256_shuffle_epi8 |
| volatile-memory | 2 | 1 | 1 | std::ptr::read_volatile::<std::mem::MaybeUninit<walk::Message>> |
| C-variadic | 1 | 1 | 1 | os::MmapInner::remap |
| runtime-entry | 1 | 1 | 1 | executable runtime adapter |

每类涉及的crate与代表错误：

- **union**：aho-corasick@1.1.3, anyhow@1.0.87, bstr@1.10.0, globset@0.4.15, grep-regex@0.1.13, regex-automata@0.4.7, regex-syntax@0.8.4, ripgrep@14.1.1。代表：`SoN construction: unsupported: union value is outside the semantic path: memchr::memmem::searcher::PrefilterKind#9f269fbae2ea58ae-31918044440eb324`。
- **unreachable-control**：aho-corasick@1.1.3, regex-automata@0.4.7, ripgrep@14.1.1, serde_json@1.0.128。代表：`machine lowering: malformed graph: control node n102 is unreachable`。
- **SIMD**：aho-corasick@1.1.3, bstr@1.10.0, crossbeam-utils@0.8.20, globset@0.4.15, grep-cli@0.1.11, grep-regex@0.1.13, ignore@0.4.23, memchr@2.7.4, regex-automata@0.4.7, regex-syntax@0.8.4, ripgrep@14.1.1, serde@1.0.210, termcolor@1.4.1。代表：`SoN construction: unsupported: SIMD value is outside the semantic path: core::arch::x86_64::__m128i#67c1bd76a8cc8674-84f70887d6d2298a`。
- **verification-other**：aho-corasick@1.1.3, memchr@2.7.4。代表：`semantic verification: graph verification failed: TypeMismatch { node: NodeId(0), detail: "type is outside the specialized value graph subset" }`。
- **i128-u128**：aho-corasick@1.1.3, anyhow@1.0.87, bstr@1.10.0, globset@0.4.15, memchr@2.7.4, regex-automata@0.4.7, ripgrep@14.1.1, ryu@1.0.18, serde@1.0.210, serde_json@1.0.128。代表：`SoN construction: unsupported: unsupported semantic type u128`。
- **DST-tail**：aho-corasick@1.1.3, bstr@1.10.0, globset@0.4.15, grep-regex@0.1.13, ignore@0.4.23, regex-automata@0.4.7, ripgrep@14.1.1。代表：`SoN construction: unsupported in bb0: unsized field requires a producer-proven static slice/string tail`。
- **DST-size_of_val**：aho-corasick@1.1.3, ignore@0.4.23, regex-automata@0.4.7。代表：`MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<dyn ahocorasick::AcAutomaton>``。
- **LLVM-intrinsic**：aho-corasick@1.1.3。代表：`MIR conversion: LLVM intrinsic `llvm.x86.avx2.pshuf.b``。
- **atomic_or**：crossbeam-epoch@0.9.18, ripgrep@14.1.1。代表：`MIR conversion: intrinsic `atomic_or` is not implemented`。
- **TLS**：crossbeam-utils@0.8.20, globset@0.4.15, ignore@0.4.23, regex-automata@0.4.7, ripgrep@14.1.1。代表：`SoN construction: unsupported: TLS is outside the semantic path`。
- **unwind**：grep-cli@0.1.11, ignore@0.4.23, ripgrep@14.1.1。代表：`MIR conversion: intrinsic `catch_unwind` is not implemented`。
- **aggregate-constant**：grep-cli@0.1.11, grep-printer@0.2.2, grep-searcher@0.1.14, ignore@0.4.23, regex-automata@0.4.7, regex-syntax@0.8.4, ripgrep@14.1.1, serde_json@1.0.128, termcolor@1.4.1。代表：`SoN construction: unsupported in bb9: representation-encoded aggregate constant requires semantic constant decoding`。
- **float-ABI**：grep-printer@0.2.2, ripgrep@14.1.1, ryu@1.0.18, serde@1.0.210, serde_json@1.0.128, textwrap@0.16.1。代表：`MIR conversion: floating-point scalar ABI is not implemented`。
- **uninhabited-value**：grep-regex@0.1.13, ignore@0.4.23, ripgrep@14.1.1。代表：`layout lowering: unsupported in layout lowering: cannot reconstruct an inhabited value of Never`。
- **volatile-memory**：ignore@0.4.23。代表：`MIR conversion: intrinsic `volatile_load` is not implemented`。
- **C-variadic**：memmap2@0.9.4。代表：`MIR conversion: C-variadic functions are not implemented`。
- **compiler-panic**：regex-automata@0.4.7。代表：`compiler panic in owned graph pipeline: inline nodes have at most two inputs`。
- **runtime-entry**：ripgrep@14.1.1。代表：`kfrc: the PoC executable adapter currently supports `fn main() -> ()`; other Termination return types are not supported yet`。
- **float-other**：ripgrep@14.1.1, serde@1.0.210, serde_json@1.0.128, textwrap@0.16.1。代表：`SoN construction: unsupported: unsupported semantic type f64`。

### 精确错误签名

同类别的不同失败层/错误签名单列，尤其保留SoN与Cranelift panic差异。

| 错误签名 | 项数 | target units | 代表crate / 函数 |
|---|---:|---:|---|
| SoN construction: unsupported: SIMD value is outside the semantic path: core::arch::x86_64::__m256i#fba485d6d644d376-331e338a37203e0b | 202 | 4 | aho-corasick@1.1.3 / packed::teddy::generic::Teddy::<8>::verify::<core::arch::x86_64::__m256i> |
| SoN construction: unsupported: unsupported semantic type u128 | 202 | 7 | aho-corasick@1.1.3 / util::prefilter::Builder::ascii_case_insensitive |
| SoN construction: unsupported: unsupported semantic type f64 | 169 | 4 | ripgrep@14.1.1 / alloc::collections::btree::node::Handle::<alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Mut<'_>, std::string::Stri |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::arch::x86_64::__m128i#67c1bd76a8cc8674-84f70887d6d2298a | 166 | 5 | aho-corasick@1.1.3 / packed::teddy::generic::Teddy::<8>::verify::<core::arch::x86_64::__m128i> |
| SoN construction: unsupported: SIMD value is outside the semantic path: std::arch::x86_64::__m128i#67c1bd76a8cc8674-84f70887d6d2298a | 88 | 5 | globset@0.4.15 / hashbrown::raw::RawTableInner::resize_inner::<std::alloc::Global> |
| MIR conversion: floating-point scalar ABI is not implemented | 82 | 6 | grep-printer@0.2.2 / util::NiceDuration::fractional_seconds |
| SoN construction: unsupported: union value is outside the semantic path: std::sync::lazy_lock::Data<std::backtrace::Capture, {closure@std::backtrace::helper::lazy_resolve::{closure#0}}>#90d1a56733746958-823052a7bacf9 | 64 | 2 | anyhow@1.0.87 / error::<impl Error>::from_adhoc::<alloc::string::String> |
| SoN construction: unsupported: union value is outside the semantic path: memchr::memmem::searcher::SearcherKind#31d3ada2ca368391-4f0195beac66192b | 45 | 3 | aho-corasick@1.1.3 / memchr::memmem::Finder::<'_>::new::<alloc::vec::Vec<u8>> |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::i8x16#4de202288ac8fa56-9422adfea3fe85ee | 40 | 10 | aho-corasick@1.1.3 / core::arch::x86_64::_mm_alignr_epi8::<13> |
| SoN construction: unsupported in bb0: unsized field requires a producer-proven static slice/string tail | 30 | 7 | aho-corasick@1.1.3 / alloc::sync::Arc::<dyn ahocorasick::AcAutomaton>::drop_slow |
| MIR conversion: unsupported scalar ABI ScalarAbi { bits: 128, extension: None } | 23 | 7 | aho-corasick@1.1.3 / std_detect::detect::cache::test |
| machine lowering: malformed graph: control node n124 is unreachable | 21 | 4 | aho-corasick@1.1.3 / alloc::collections::btree::navigate::<impl alloc::collections::btree::node::Handle<alloc::collections::btree::node::NodeRef<alloc::collections::btree: |
| machine lowering: malformed graph: control node n108 is unreachable | 18 | 4 | aho-corasick@1.1.3 / alloc::collections::btree::navigate::LazyLeafRange::<alloc::collections::btree::node::marker::Dying, alloc::boxed::Box<[u8]>, usize>::init_front |
| semantic verification: graph verification failed: TypeMismatch { node: NodeId(7), detail: "semantic aggregate constants require value constructors" } | 18 | 5 | grep-printer@0.2.2 / std::cell::RefCell::<std::vec::Vec<u8>>::new |
| machine lowering: malformed graph: control node n102 is unreachable | 17 | 4 | aho-corasick@1.1.3 / alloc::collections::btree::search::<impl alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Mut<'_>, alloc::boxed::Box< |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::i8x32#db30f0c4957e1ff0-7946e52d8322e478 | 17 | 4 | aho-corasick@1.1.3 / core::arch::x86_64::_mm256_alignr_epi8::<13> |
| SoN construction: unsupported: union value is outside the semantic path: core::escape::MaybeEscapedCharacter<10>#9370681a58d76310-d6e95c04666c705d | 16 | 6 | anyhow@1.0.87 / <context::Quoted<&mut core::fmt::Formatter<'_>> as core::fmt::Write>::write_str |
| machine lowering: unsupported in machine lowering: 128-bit integers | 16 | 1 | regex-automata@0.4.7 / core::ptr::drop_in_place::<core::option::Option<meta::wrappers::HybridEngine>> - shim(Some(core::option::Option<meta::wrappers::HybridEngine>)) |
| machine lowering: malformed graph: control node n169 is unreachable | 12 | 4 | aho-corasick@1.1.3 / alloc::collections::btree::navigate::LazyLeafRange::<alloc::collections::btree::node::marker::Dying, alloc::boxed::Box<[u8]>, usize>::take_front |
| SoN construction: unsupported: TLS is outside the semantic path | 10 | 5 | crossbeam-utils@0.8.20 / std::hash::RandomState::new::KEYS::{constant#0}::{closure#1} |
| SoN construction: unsupported in bb9: representation-encoded aggregate constant requires semantic constant decoding | 10 | 5 | grep-cli@0.1.11 / <std::sys::stdio::unix::Stderr as std::io::Write>::write_all |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::i64x4#b53314c51c8f39bb-8a3b8ce4aa781344 | 9 | 2 | aho-corasick@1.1.3 / core::arch::x86_64::_mm256_permute2f128_si256::<33> |
| MIR conversion: intrinsic `atomic_or` is not implemented | 8 | 2 | crossbeam-epoch@0.9.18 / std::sync::atomic::AtomicUsize::fetch_or |
| MIR conversion: intrinsic `catch_unwind` is not implemented | 7 | 3 | grep-cli@0.1.11 / std::thread::lifecycle::spawn_unchecked::<'_, {closure@crates/cli/src/process.rs:282:32: 282:39}, process::CommandError>::{closure#1} |
| SoN construction: unsupported: union value is outside the semantic path: memchr::memmem::searcher::PrefilterKind#9f269fbae2ea58ae-31918044440eb324 | 6 | 3 | aho-corasick@1.1.3 / memchr::memmem::searcher::Searcher::new::<memchr::arch::all::packedpair::DefaultFrequencyRank> |
| semantic verification: graph verification failed: TypeMismatch { node: NodeId(4), detail: "type is outside the specialized value graph subset" } | 6 | 1 | memchr@2.7.4 / arch::generic::memchr::One::<core::arch::x86_64::__m128i>::new |
| compiler panic in owned graph pipeline: inline nodes have at most two inputs | 4 | 1 | regex-automata@0.4.7 / meta::strategy::ReverseSuffix::try_search_half_start |
| semantic verification: graph verification failed: TypeMismatch { node: NodeId(0), detail: "type is outside the specialized value graph subset" } | 3 | 1 | aho-corasick@1.1.3 / core::arch::x86_64::_mm256_permute2x128_si256::<33> |
| SoN construction: unsupported: union value is outside the semantic path: core::escape::MaybeEscapedCharacter<4>#85c3c6670cf6ff0b-5bbda9fdbd4456f7 | 3 | 3 | aho-corasick@1.1.3 / <util::debug::DebugByte as core::fmt::Debug>::fmt |
| layout lowering: unsupported in layout lowering: cannot reconstruct an inhabited value of Never | 3 | 3 | grep-regex@0.1.13 / <std::result::Result<std::vec::in_place_drop::InPlaceDrop<regex_syntax::hir::Hir>, !> as std::ops::FromResidual<std::result::Result<std::convert::Infa |
| SoN construction: unsupported in bb15: representation-encoded aggregate constant requires semantic constant decoding | 3 | 2 | ignore@0.4.23 / std::io::append_to_string::<{closure@<std::io::BufReader<std::fs::File> as std::io::BufRead>::read_line::{closure#0}}> |
| SoN construction: unsupported: SIMD value is outside the semantic path: std::simd::Simd<i8, 1>#babfc436dcd0b2f3-c5ad70aee1feef5b | 3 | 3 | ripgrep@14.1.1 / std::simd::Simd::<T, N>::splat::splat_rt::<i8, 16> |
| SoN construction: unsupported: SIMD value is outside the semantic path: std::simd::Simd<u8, 1>#a121cd8074903a21-87cddfd4304b71c6 | 3 | 3 | ripgrep@14.1.1 / std::simd::Simd::<T, N>::splat::splat_rt::<u8, 16> |
| SoN construction: unsupported: SIMD value is outside the semantic path: std::simd::Simd<i8, 16>#9bf4305c7d6ea96a-95134b9d5703dc97 | 3 | 3 | ripgrep@14.1.1 / std::str::pattern::simd_contains::{closure#3} |
| SoN construction: unsupported: SIMD value is outside the semantic path: std::simd::Simd<i8, 64>#a299f5919a1a8597-e8a8de458c297305 | 3 | 3 | ripgrep@14.1.1 / core::core_simd::masks::mask_impl::Mask::<i8, 16>::to_bitmask_integer |
| SoN construction: unsupported: SIMD value is outside the semantic path: std::simd::Simd<u8, 16>#e1f034a248ac678d-7599364c536942c0 | 3 | 3 | ripgrep@14.1.1 / std::str::pattern::simd_contains |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<dyn util::prefilter::PrefilterI>` | 2 | 2 | aho-corasick@1.1.3 / <alloc::sync::Weak<dyn util::prefilter::PrefilterI, &alloc::alloc::Global> as core::ops::Drop>::drop |
| layout lowering: unsupported in layout lowering: uninhabited variant construction | 2 | 2 | ignore@0.4.23 / std::sync::OnceLock::<regex_automata::meta::Regex>::initialize::<{closure@std::sync::OnceLock<regex_automata::meta::Regex>::get_or_init<{closure@crate |
| machine lowering: malformed graph: control node n111 is unreachable | 2 | 1 | serde_json@1.0.128 / alloc::collections::btree::search::<impl alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Mut<'_>, alloc::string::Str |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::u16x16#6bdca214a316638c-202837c97954ac3 | 1 | 1 | aho-corasick@1.1.3 / core::arch::x86_64::_mm256_srli_epi16::<4> |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::u16x8#fc223c7741df9da1-60c99891d594a8ad | 1 | 1 | aho-corasick@1.1.3 / core::arch::x86_64::_mm_srli_epi16::<4> |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<dyn ahocorasick::AcAutomaton>` | 1 | 1 | aho-corasick@1.1.3 / <alloc::sync::Weak<dyn ahocorasick::AcAutomaton, &alloc::alloc::Global> as core::ops::Drop>::drop |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<dyn packed::teddy::builder::SearcherT>` | 1 | 1 | aho-corasick@1.1.3 / <alloc::sync::Weak<dyn packed::teddy::builder::SearcherT, &alloc::alloc::Global> as core::ops::Drop>::drop |
| MIR conversion: LLVM intrinsic `llvm.x86.avx2.pshuf.b` | 1 | 1 | aho-corasick@1.1.3 / core::arch::x86_64::_mm256_shuffle_epi8 |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::i64x2#5c6c7e16162ee2bd-14b51f0e3e172e | 1 | 1 | aho-corasick@1.1.3 / core::arch::x86_64::_mm256_broadcastsi128_si256 |
| MIR conversion: LLVM intrinsic `llvm.x86.ssse3.pshuf.b.128` | 1 | 1 | aho-corasick@1.1.3 / core::arch::x86_64::_mm_shuffle_epi8 |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::u16x8::splat::JustOne#f0b8cb745c36772d-a22730049bf12b18 | 1 | 1 | aho-corasick@1.1.3 / core::core_arch::simd::u16x8::splat |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::u16x16::splat::JustOne#91bbd583e2014d34-cdd7cb045a9982b2 | 1 | 1 | aho-corasick@1.1.3 / core::core_arch::simd::u16x16::splat |
| MIR conversion: intrinsic `volatile_load` is not implemented | 1 | 1 | ignore@0.4.23 / std::ptr::read_volatile::<std::mem::MaybeUninit<walk::Message>> |
| MIR conversion: intrinsic `volatile_store` is not implemented | 1 | 1 | ignore@0.4.23 / std::ptr::write_volatile::<std::mem::MaybeUninit<walk::Message>> |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<dyn for<'a, 'b> std::ops::Fn(&'a std::path::Path, &'b std::path::Path) -> std::cmp::Ordering + std::marker::Send + std::marker::Sync>` | 1 | 1 | ignore@0.4.23 / <std::sync::Weak<dyn for<'a, 'b> std::ops::Fn(&'a std::path::Path, &'b std::path::Path) -> std::cmp::Ordering + std::marker::Send + std::marker::Sync, |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<dyn for<'a, 'b> std::ops::Fn(&'a std::ffi::OsStr, &'b std::ffi::OsStr) -> std::cmp::Ordering + std::marker::Send + std::marker::Sync>` | 1 | 1 | ignore@0.4.23 / <std::sync::Weak<dyn for<'a, 'b> std::ops::Fn(&'a std::ffi::OsStr, &'b std::ffi::OsStr) -> std::cmp::Ordering + std::marker::Send + std::marker::Sync, |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<dyn for<'a> std::ops::Fn(&'a walk::DirEntry) -> bool + std::marker::Send + std::marker::Sync>` | 1 | 1 | ignore@0.4.23 / <std::sync::Weak<dyn for<'a> std::ops::Fn(&'a walk::DirEntry) -> bool + std::marker::Send + std::marker::Sync, &std::alloc::Global> as std::ops::Drop> |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<[crossbeam_deque::Stealer<walk::Message>]>` | 1 | 1 | ignore@0.4.23 / <std::sync::Weak<[crossbeam_deque::Stealer<walk::Message>], &std::alloc::Global> as std::ops::Drop>::drop |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::i32x8#c87367e184f5fe05-4dfcb0351916e3d6 | 1 | 1 | memchr@2.7.4 / core::arch::x86_64::_mm256_or_si256 |
| MIR conversion: C-variadic functions are not implemented | 1 | 1 | memmap2@0.9.4 / os::MmapInner::remap |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<dyn meta::strategy::Strategy>` | 1 | 1 | regex-automata@0.4.7 / <std::sync::Weak<dyn meta::strategy::Strategy, &std::alloc::Global> as core::ops::Drop>::drop |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<[u8]>` | 1 | 1 | regex-automata@0.4.7 / <std::sync::Weak<[u8], &std::alloc::Global> as core::ops::Drop>::drop |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<str>` | 1 | 1 | regex-automata@0.4.7 / <std::sync::Weak<str, &std::alloc::Global> as core::ops::Drop>::drop |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::simd::Simd<i8, 1>#babfc436dcd0b2f3-c5ad70aee1feef5b | 1 | 1 | regex-syntax@0.8.4 / core::simd::Simd::<T, N>::splat::splat_rt::<i8, 16> |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::simd::Simd<u8, 1>#a121cd8074903a21-87cddfd4304b71c6 | 1 | 1 | regex-syntax@0.8.4 / core::simd::Simd::<T, N>::splat::splat_rt::<u8, 16> |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::simd::Simd<i8, 16>#9bf4305c7d6ea96a-95134b9d5703dc97 | 1 | 1 | regex-syntax@0.8.4 / core::str::pattern::simd_contains::{closure#3} |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::simd::Simd<i8, 64>#a299f5919a1a8597-e8a8de458c297305 | 1 | 1 | regex-syntax@0.8.4 / core::core_simd::masks::mask_impl::Mask::<i8, 16>::to_bitmask_integer |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::simd::Simd<u8, 16>#e1f034a248ac678d-7599364c536942c0 | 1 | 1 | regex-syntax@0.8.4 / core::str::pattern::simd_contains |
| kfrc: the PoC executable adapter currently supports `fn main() -> ()`; other Termination return types are not supported yet | 1 | 1 | ripgrep@14.1.1 / executable runtime adapter |
| machine lowering: malformed graph: control node n98 is unreachable | 1 | 1 | ripgrep@14.1.1 / std::collections::BTreeMap::<std::borrow::Cow<'_, [u8]>, alloc::collections::btree::set_val::SetValZST>::first_key_value |
| machine lowering: unsupported in machine lowering: values of type `f32` | 1 | 1 | serde@1.0.210 / std::option::Option::<(__private::de::content::Content<'_>, __private::de::content::Content<'_>)>::take |
| machine lowering: unsupported in machine lowering: values of type `f64` | 1 | 1 | serde_json@1.0.128 / core::mem::replace::<value::Value> |
| definition specialization: floating-point scalar ABI is not implemented | 1 | 1 | textwrap@0.16.1 / std::option::Option::<&f64>::copied |

### MIR intrinsic调用观察（包含受更早错误遮挡的调用）

| intrinsic | 调用点数 | target units |
|---|---:|---:|
| assert_inhabited | 353 | 18 |
| size_of_val | 352 | 21 |
| ptr_offset_from_unsigned | 309 | 20 |
| cold_path | 304 | 22 |
| align_of_val | 299 | 19 |
| atomic_cxchg | 225 | 8 |
| arith_offset | 166 | 18 |
| atomic_xsub | 160 | 12 |
| abort | 142 | 13 |
| atomic_load | 117 | 17 |
| copy | 106 | 13 |
| rotate_left | 96 | 4 |
| atomic_xadd | 77 | 9 |
| atomic_cxchgweak | 75 | 3 |
| saturating_add | 62 | 15 |
| atomic_store | 57 | 10 |
| saturating_sub | 55 | 15 |
| select_unpredictable | 51 | 9 |
| atomic_fence | 48 | 12 |
| cttz_nonzero | 43 | 7 |
| atomic_xchg | 40 | 6 |
| simd_shuffle | 38 | 5 |
| simd_eq | 33 | 10 |
| typed_swap_nonoverlapping | 31 | 9 |
| write_bytes | 29 | 18 |
| ctlz | 28 | 10 |
| simd_bitmask | 28 | 13 |
| compare_bytes | 27 | 18 |
| ctlz_nonzero | 23 | 8 |
| ptr_offset_from | 19 | 12 |
| is_val_statically_known | 18 | 17 |
| cttz | 17 | 13 |
| simd_or | 15 | 9 |
| simd_lt | 12 | 10 |
| atomic_or | 12 | 2 |
| raw_eq | 8 | 7 |
| simd_and | 8 | 6 |
| atomic_singlethreadfence | 8 | 2 |
| simd_reduce_all | 8 | 4 |
| black_box | 7 | 3 |
| catch_unwind | 7 | 3 |
| ctpop | 5 | 4 |
| simd_extract | 4 | 1 |
| unchecked_funnel_shl | 4 | 4 |
| disjoint_bitor | 4 | 4 |
| bswap | 4 | 2 |
| simd_gt | 4 | 4 |
| simd_shr | 2 | 1 |
| fabsf64 | 2 | 2 |
| exact_div | 1 | 1 |
| volatile_load | 1 | 1 |
| volatile_store | 1 | 1 |
| caller_location | 1 | 1 |
| fabsf32 | 1 | 1 |

### 只移除一种首错后的候选范围

下表只按已观察首错计算；同函数的后续缺口和crate级变换仍可能阻止编译。

| 唯一首错类别 | target units |
|---|---|
| atomic_or | crossbeam_epoch-d8b2f35468da2361 |
| aggregate-constant | grep_searcher-1e1b28c0e70925d3 |
| C-variadic | memmap2-b78196097baf81e2 |
