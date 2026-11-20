# 全函数调查分类

计数单位为单态化函数/静态项的首个失败；intrinsic观察单列，不能相加当作失败总数。

## cargo

286/286 个target units报告完整；297805个项，21254个首错。

| 缺口 | 首错项数 | 涉及target units | 涉及package versions | 代表函数 |
|---|---:|---:|---:|---|
完整报告的Cranelift配置：none=3 units, speed=283 units

| float-other | 7092 | 28 | 27 | gix::open::<&std::path::PathBuf> |
| union | 5059 | 44 | 43 | memchr::memmem::searcher::Searcher::new::<memchr::arch::all::packedpair::DefaultFrequencyRank> |
| SIMD | 2443 | 61 | 60 | packed::teddy::generic::Teddy::<8>::verify::<core::arch::x86_64::__m128i> |
| i128-u128 | 2004 | 74 | 73 | util::prefilter::Builder::ascii_case_insensitive |
| float-ABI | 1059 | 37 | 36 | core::bool::<impl bool>::then_some::<f64> |
| unreachable-control | 761 | 35 | 34 | alloc::collections::btree::search::<impl alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Mut<'_>, alloc::boxed::Box<[u8]>, usize, alloc::collectio |
| atomic_xsub | 581 | 55 | 54 | <alloc::sync::Arc<dyn ahocorasick::AcAutomaton> as core::ops::Drop>::drop |
| aggregate-constant | 398 | 56 | 55 | std::io::append_to_string::<{closure@std::io::default_read_to_string<flate2::read::GzDecoder<&std::fs::File>>::{closure#0}}> |
| atomic_xadd | 282 | 42 | 41 | <alloc::sync::Arc<dyn util::prefilter::PrefilterI> as core::clone::Clone>::clone |
| atomic_load | 227 | 93 | 92 | core::sync::atomic::atomic_load::<*mut ()> |
| unwind | 147 | 13 | 12 | std::thread::scope::<'_, {closure@gix::parallel::in_parallel_with_slice<gix::gix_odb::gix_pack::cache::delta::tree::Item<gix::gix_odb::gix_pack::index::write::TreeEntry>, gix::gix_ |
| atomic_xchg | 124 | 27 | 26 | <std::sync::ReentrantLockGuard<'_, std::cell::RefCell<std::io::LineWriter<std::io::stdio::StdoutRaw>>> as std::ops::Drop>::drop |
| atomic_store | 109 | 52 | 51 | std::sync::atomic::atomic_store::<u64> |
| verification-other | 102 | 7 | 7 | core::arch::x86_64::_mm256_permute2x128_si256::<33> |
| compiler-panic | 98 | 12 | 12 | gix::status::index_worktree::<impl gix::Repository>::index_worktree_status_pathspec::<gix::status::index_worktree::Error, gix::bstr::BString, std::vec::Vec<gix::bstr::BString>> |
| atomic_or | 92 | 9 | 8 | std::sync::mpmc::list::Channel::<gix::status::gix_status::index_as_worktree_with_renames::function::Event<'_, (), gix::submodule::Status>>::disconnect_senders |
| uninhabited-value | 89 | 17 | 16 | util::context::GlobalContext::get_config_env::<std::string::String>::{closure#0} |
| atomic_cxchg | 84 | 41 | 40 | std::sync::atomic::atomic_compare_exchange::<usize> |
| TLS | 81 | 42 | 41 | std::hash::RandomState::new::KEYS::{constant#0}::{closure#1} |
| DST-tail | 71 | 19 | 18 | alloc::sync::Arc::<dyn ahocorasick::AcAutomaton>::drop_slow |
| LLVM-intrinsic | 62 | 25 | 24 | core::arch::x86_64::_mm256_shuffle_epi8 |
| C-variadic | 61 | 16 | 16 | curl::easy::Easy2::<curl::easy::handle::EasyData>::setopt_ptr |
| atomic_fence | 53 | 53 | 52 | core::sync::atomic::fence |
| DST-size_of_val | 51 | 27 | 26 | <alloc::sync::Weak<dyn ahocorasick::AcAutomaton, &alloc::alloc::Global> as core::ops::Drop>::drop |
| atomic_cxchgweak | 46 | 29 | 28 | std::sync::atomic::atomic_compare_exchange_weak::<*mut debt::list::Node> |
| volatile-memory | 32 | 10 | 10 | std::ptr::write_volatile::<u8> |
| assembly | 23 | 9 | 9 | optimizer_hide |
| atomic_singlethreadfence | 8 | 8 | 8 | std::sync::atomic::compiler_fence |
| semantic-place-type | 4 | 3 | 3 | core::hash::impls::<impl std::hash::Hash for *const dyn tracing::Callsite>::hash::<std::hash::DefaultHasher> |
| enum-phi-shape | 4 | 2 | 2 | hazardous::kem::ml_kem::internal::KeyPairInternal::<hazardous::kem::ml_kem::internal::MlKem512Internal>::keygen::<2, 800, 1632> |
| dyn | 3 | 2 | 2 | error_chain::{closure#2} |
| atomic_and | 3 | 2 | 2 | std::sync::atomic::AtomicUsize::fetch_and |
| static-linkage | 1 | 1 | 1 | INIT_CTOR |

每类涉及的crate与代表错误：

- **union**：aho-corasick@1.1.4, anyhow@1.0.100, bstr@1.12.1, cargo-credential-libsecret@0.5.5, cargo-util@0.2.27, cargo@0.95.0, erased-serde@0.4.9, gix-attributes@0.29.0, gix-commitgraph@0.31.0, gix-config@0.50.0, gix-credentials@0.34.1, gix-diff@0.57.0, gix-discover@0.45.0, gix-filter@0.24.0, gix-index@0.45.0, gix-object@0.54.0, gix-odb@0.74.0, gix-pack@0.64.0, gix-packetline@0.20.0, gix-pathspec@0.14.0, gix-protocol@0.55.0, gix-quote@0.6.1, gix-ref@0.57.0, gix-refspec@0.35.0, gix-revision@0.39.0, gix-status@0.24.0, gix-submodule@0.24.0, gix-transport@0.52.1, gix-url@0.34.0, gix-validate@0.10.1, gix-worktree@0.46.0, gix@0.77.0, globset@0.4.18, icu_properties@2.1.2, jiff@0.2.17, rand@0.9.2, rand_chacha@0.9.0, regex-automata@0.4.13, regex-syntax@0.8.8, rusqlite@0.38.0, toml@0.9.10+spec-1.1.0, toml_edit@0.24.0+spec-1.1.0, winnow@0.7.14。代表：`SoN construction: unsupported: union value is outside the semantic path: memchr::memmem::searcher::PrefilterKind#aa9f9ffbcc8dacf4-801ccfc7ead00cee`。
- **unreachable-control**：aho-corasick@1.1.4, cargo-util-schemas@0.12.0, cargo-util@0.2.27, cargo@0.95.0, clap_builder@4.5.53, crates-io@0.40.17, git2@0.20.3, gix-attributes@0.29.0, gix-command@0.6.4, gix-config@0.50.0, gix-credentials@0.34.1, gix-diff@0.57.0, gix-dir@0.19.0, gix-filter@0.24.0, gix-index@0.45.0, gix-odb@0.74.0, gix-pack@0.64.0, gix-path@0.10.22, gix-pathspec@0.14.0, gix-protocol@0.55.0, gix-ref@0.57.0, gix-refspec@0.35.0, gix-status@0.24.0, gix-submodule@0.24.0, gix-transport@0.52.1, gix@0.77.0, opener@0.8.3, os_info@3.14.0, pasetors@0.7.7, regex-automata@0.4.13, rusqlite@0.38.0, serde-value@0.7.0, serde_json@1.0.148, tracing-chrome@0.7.2。代表：`machine lowering: malformed graph: control node n102 is unreachable`。
- **SIMD**：aho-corasick@1.1.4, annotate-snippets@0.12.10, bitmaps@2.1.0, bstr@1.12.1, cargo-util-schemas@0.12.0, cargo-util@0.2.27, cargo@0.95.0, clap_builder@4.5.53, clap_complete@4.5.64, crates-io@0.40.17, crc32fast@1.5.0, crossbeam-utils@0.8.21, faster-hex@0.10.0, gix-attributes@0.29.0, gix-command@0.6.4, gix-config@0.50.0, gix-credentials@0.34.1, gix-date@0.12.0, gix-diff@0.57.0, gix-filter@0.24.0, gix-index@0.45.0, gix-lock@20.0.0, gix-negotiate@0.25.0, gix-object@0.54.0, gix-odb@0.74.0, gix-pack@0.64.0, gix-pathspec@0.14.0, gix-protocol@0.55.0, gix-ref@0.57.0, gix-shallow@0.7.0, gix-status@0.24.0, gix-submodule@0.24.0, gix-tempfile@20.0.0, gix-transport@0.52.1, gix-url@0.34.0, gix-validate@0.10.1, gix@0.77.0, globset@0.4.18, hashbrown@0.14.5, hashbrown@0.15.5, hashbrown@0.16.1, ignore@0.4.25, memchr@2.7.6, opener@0.8.3, pasetors@0.7.7, prodash@30.0.1, rand_chacha@0.9.0, regex-automata@0.4.13, regex-syntax@0.8.8, regex@1.12.2, rusqlite@0.38.0, serde_core@1.0.228, sha1@0.10.6, sha2@0.10.9, strsim@0.11.1, toml@0.9.10+spec-1.1.0, toml_edit@0.24.0+spec-1.1.0, toml_parser@1.0.6+spec-1.1.0, tracing-subscriber@0.3.22, zlib-rs@0.5.5。代表：`SoN construction: unsupported: SIMD value is outside the semantic path: core::arch::x86_64::__m128i#67c1bd76a8cc8674-84f70887d6d2298a`。
- **atomic_load**：aho-corasick@1.1.4, annotate-snippets@0.12.10, arc-swap@1.7.1, blake3@1.8.2, bstr@1.12.1, bytes@1.11.0, cargo-util@0.2.27, cargo@0.95.0, colorchoice@1.0.4, crc32fast@1.5.0, crossbeam-channel@0.5.15, crossbeam-epoch@0.9.18, crossbeam-utils@0.8.21, curl@0.4.49, dashmap@6.1.0, ed25519-compact@2.2.0, faster-hex@0.10.0, filetime@0.2.26, foldhash@0.1.5, foldhash@0.2.0, getrandom@0.2.16, getrandom@0.3.4, git2-curl@0.21.0, git2@0.20.3, gix-actor@0.37.0, gix-attributes@0.29.0, gix-command@0.6.4, gix-config@0.50.0, gix-credentials@0.34.1, gix-diff@0.57.0, gix-dir@0.19.0, gix-features@0.45.1, gix-filter@0.24.0, gix-glob@0.23.0, gix-hash@0.21.1, gix-ignore@0.18.0, gix-index@0.45.0, gix-lock@20.0.0, gix-object@0.54.0, gix-odb@0.74.0, gix-pack@0.64.0, gix-path@0.10.22, gix-pathspec@0.14.0, gix-protocol@0.55.0, gix-quote@0.6.1, gix-ref@0.57.0, gix-refspec@0.35.0, gix-shallow@0.7.0, gix-tempfile@20.0.0, gix-transport@0.52.1, gix-traverse@0.51.0, gix-url@0.34.0, gix-validate@0.10.1, gix-worktree@0.46.0, gix@0.77.0, globset@0.4.18, http-auth@0.1.10, ignore@0.4.25, jiff@0.2.17, jobserver@0.1.34, libgit2-sys@0.18.3+1.9.2, libssh2-sys@0.3.1, log@0.4.29, memchr@2.7.6, memmap2@0.9.9, nix@0.30.1, once_cell@1.21.3, openssl-sys@0.9.111, os_info@3.14.0, parking_lot@0.12.5, parking_lot_core@0.9.12, pasetors@0.7.7, prodash@30.0.1, rand@0.9.2, rand_chacha@0.9.0, regex-automata@0.4.13, regex@1.12.2, rusqlite@0.38.0, serde_json@1.0.148, sha1@0.10.6, sha2@0.10.9, sharded-slab@0.1.7, tempfile@3.24.0, thread_local@1.1.9, toml_parser@1.0.6+spec-1.1.0, tracing-chrome@0.7.2, tracing-core@0.1.36, tracing-log@0.2.0, tracing-subscriber@0.3.22, tracing@0.1.44, winnow@0.7.14, zlib-rs@0.5.5。代表：`MIR conversion: intrinsic `atomic_load` is not implemented`。
- **verification-other**：aho-corasick@1.1.4, cargo@0.95.0, der@0.7.10, icu_properties@2.1.2, memchr@2.7.6, rand_chacha@0.9.0, zlib-rs@0.5.5。代表：`semantic verification: graph verification failed: TypeMismatch { node: NodeId(0), detail: "type is outside the specialized value graph subset" }`。
- **i128-u128**：aho-corasick@1.1.4, anyhow@1.0.100, bitflags@2.10.0, bitmaps@2.1.0, blake3@1.8.2, bstr@1.12.1, cargo-credential-libsecret@0.5.5, cargo-util@0.2.27, cargo@0.95.0, clap_builder@4.5.53, clap_complete@4.5.64, crc32fast@1.5.0, crypto-bigint@0.5.5, curl@0.4.49, der@0.7.10, ed25519-compact@2.2.0, erased-serde@0.4.9, fastrand@2.3.0, foldhash@0.1.5, foldhash@0.2.0, gix-config@0.50.0, gix-date@0.12.0, gix-diff@0.57.0, gix-features@0.45.1, gix-filter@0.24.0, gix-fs@0.18.1, gix-hashtable@0.11.0, gix-object@0.54.0, gix-odb@0.74.0, gix-pathspec@0.14.0, gix-protocol@0.55.0, gix-tempfile@20.0.0, gix-transport@0.52.1, gix-url@0.34.0, gix-utils@0.3.1, gix-validate@0.10.1, gix@0.77.0, globset@0.4.18, idna@1.1.0, jiff@0.2.17, matchers@0.2.0, memchr@2.7.6, orion@0.17.11, p384@0.13.1, pasetors@0.7.7, powerfmt@0.2.0, prodash@30.0.1, rand@0.9.2, rand_chacha@0.9.0, rand_core@0.6.4, regex-automata@0.4.13, rusqlite@0.38.0, serde_core@1.0.228, serde_json@1.0.148, shlex@1.3.0, socket2@0.6.1, tempfile@3.24.0, time@0.3.44, toml@0.9.10+spec-1.1.0, toml_edit@0.24.0+spec-1.1.0, tracing-chrome@0.7.2, tracing-core@0.1.36, tracing-log@0.2.0, tracing-subscriber@0.3.22, typeid@1.0.3, unicode-ident@1.0.22, url@2.5.7, winnow@0.7.14, writeable@0.6.2, zeroize@1.8.2, zerovec@0.11.5, zlib-rs@0.5.5, zmij@1.0.8。代表：`SoN construction: unsupported: unsupported semantic type u128`。
- **DST-tail**：aho-corasick@1.1.4, bstr@1.12.1, cargo@0.95.0, clap_builder@4.5.53, clap_complete@4.5.64, gix-odb@0.74.0, gix-transport@0.52.1, gix@0.77.0, globset@0.4.18, ignore@0.4.25, pasetors@0.7.7, prodash@30.0.1, regex-automata@0.4.13, regex@1.12.2, tar@0.4.44, tracing-core@0.1.36, tracing-log@0.2.0, tracing@0.1.44。代表：`SoN construction: unsupported in bb0: unsized field requires a producer-proven static slice/string tail`。
- **atomic_xsub**：aho-corasick@1.1.4, arc-swap@1.7.1, bstr@1.12.1, bytes@1.11.0, cargo-util-schemas@0.12.0, cargo-util@0.2.27, cargo@0.95.0, clap_builder@4.5.53, clap_complete@4.5.64, crossbeam-channel@0.5.15, crossbeam-epoch@0.9.18, crossbeam-utils@0.8.21, curl@0.4.49, dashmap@6.1.0, fastrand@2.3.0, git2-curl@0.21.0, gix-config@0.50.0, gix-date@0.12.0, gix-diff@0.57.0, gix-dir@0.19.0, gix-discover@0.45.0, gix-features@0.45.1, gix-filter@0.24.0, gix-fs@0.18.1, gix-index@0.45.0, gix-odb@0.74.0, gix-pack@0.64.0, gix-ref@0.57.0, gix-status@0.24.0, gix-tempfile@20.0.0, gix-transport@0.52.1, gix@0.77.0, glob@0.3.3, globset@0.4.18, ignore@0.4.25, jiff@0.2.17, jobserver@0.1.34, once_cell@1.21.3, parking_lot@0.12.5, parking_lot_core@0.9.12, pasetors@0.7.7, prodash@30.0.1, regex-automata@0.4.13, regex@1.12.2, rusqlite@0.38.0, tar@0.4.44, toml@0.9.10+spec-1.1.0, toml_edit@0.24.0+spec-1.1.0, tracing-chrome@0.7.2, tracing-core@0.1.36, tracing-log@0.2.0, tracing-subscriber@0.3.22, tracing@0.1.44, walkdir@2.5.0。代表：`MIR conversion: intrinsic `atomic_xsub` is not implemented`。
- **DST-size_of_val**：aho-corasick@1.1.4, cargo-util@0.2.27, cargo@0.95.0, clap_builder@4.5.53, gix-diff@0.57.0, gix-lock@20.0.0, gix-odb@0.74.0, gix-pack@0.64.0, gix-ref@0.57.0, gix-tempfile@20.0.0, gix-transport@0.52.1, icu_collections@2.1.1, ignore@0.4.25, jiff@0.2.17, jobserver@0.1.34, pasetors@0.7.7, prodash@30.0.1, regex-automata@0.4.13, regex@1.12.2, rusqlite@0.38.0, rustfix@0.9.4, serde_json@1.0.148, tempfile@3.24.0, toml@0.9.10+spec-1.1.0, toml_edit@0.24.0+spec-1.1.0, tracing-core@0.1.36。代表：`MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<dyn ahocorasick::AcAutomaton>``。
- **atomic_xadd**：aho-corasick@1.1.4, arc-swap@1.7.1, bytes@1.11.0, cargo-util@0.2.27, cargo@0.95.0, clap_builder@4.5.53, clap_complete@4.5.64, crossbeam-epoch@0.9.18, crossbeam-utils@0.8.21, curl@0.4.49, git2-curl@0.21.0, gix-config@0.50.0, gix-date@0.12.0, gix-features@0.45.1, gix-filter@0.24.0, gix-index@0.45.0, gix-odb@0.74.0, gix-pack@0.64.0, gix-ref@0.57.0, gix-status@0.24.0, gix-tempfile@20.0.0, gix-transport@0.52.1, gix@0.77.0, globset@0.4.18, ignore@0.4.25, jiff@0.2.17, jobserver@0.1.34, parking_lot@0.12.5, parking_lot_core@0.9.12, pasetors@0.7.7, prodash@30.0.1, regex-automata@0.4.13, regex@1.12.2, rusqlite@0.38.0, toml@0.9.10+spec-1.1.0, toml_edit@0.24.0+spec-1.1.0, tracing-chrome@0.7.2, tracing-core@0.1.36, tracing-subscriber@0.3.22, tracing@0.1.44, url@2.5.7。代表：`MIR conversion: intrinsic `atomic_xadd` is not implemented`。
- **atomic_fence**：aho-corasick@1.1.4, bstr@1.12.1, cargo-util-schemas@0.12.0, cargo-util@0.2.27, cargo@0.95.0, clap_builder@4.5.53, clap_complete@4.5.64, crossbeam-channel@0.5.15, crossbeam-epoch@0.9.18, crossbeam-utils@0.8.21, curl@0.4.49, ed25519-compact@2.2.0, fastrand@2.3.0, git2-curl@0.21.0, gix-config@0.50.0, gix-date@0.12.0, gix-diff@0.57.0, gix-dir@0.19.0, gix-discover@0.45.0, gix-features@0.45.1, gix-filter@0.24.0, gix-fs@0.18.1, gix-index@0.45.0, gix-odb@0.74.0, gix-pack@0.64.0, gix-ref@0.57.0, gix-status@0.24.0, gix-tempfile@20.0.0, gix-transport@0.52.1, gix@0.77.0, glob@0.3.3, globset@0.4.18, ignore@0.4.25, jiff@0.2.17, jobserver@0.1.34, once_cell@1.21.3, parking_lot@0.12.5, parking_lot_core@0.9.12, pasetors@0.7.7, prodash@30.0.1, regex-automata@0.4.13, regex@1.12.2, rusqlite@0.38.0, tar@0.4.44, toml@0.9.10+spec-1.1.0, toml_edit@0.24.0+spec-1.1.0, tracing-chrome@0.7.2, tracing-core@0.1.36, tracing-log@0.2.0, tracing-subscriber@0.3.22, tracing@0.1.44, walkdir@2.5.0。代表：`MIR conversion: intrinsic `atomic_fence` is not implemented`。
- **LLVM-intrinsic**：aho-corasick@1.1.4, cargo@0.95.0, crc32fast@1.5.0, crossbeam-channel@0.5.15, dashmap@6.1.0, faster-hex@0.10.0, foldhash@0.1.5, foldhash@0.2.0, gix-features@0.45.1, gix-pack@0.64.0, gix-transport@0.52.1, gix@0.77.0, ignore@0.4.25, log@0.4.29, parking_lot@0.12.5, parking_lot_core@0.9.12, prodash@30.0.1, rand_chacha@0.9.0, sha1@0.10.6, sha2@0.10.9, tracing-chrome@0.7.2, tracing-core@0.1.36, tracing-subscriber@0.3.22, zlib-rs@0.5.5。代表：`MIR conversion: LLVM intrinsic `llvm.x86.avx2.pshuf.b``。
- **TLS**：annotate-snippets@0.12.10, cargo@0.95.0, clap_complete@4.5.64, crossbeam-channel@0.5.15, crossbeam-utils@0.8.21, curl@0.4.49, dashmap@6.1.0, fastrand@2.3.0, git2@0.20.3, gix-config@0.50.0, gix-diff@0.57.0, gix-features@0.45.1, gix-filter@0.24.0, gix-fs@0.18.1, gix-object@0.54.0, gix-odb@0.74.0, gix-pack@0.64.0, gix-pathspec@0.14.0, gix-protocol@0.55.0, gix-submodule@0.24.0, gix-tempfile@20.0.0, gix-transport@0.52.1, gix-utils@0.3.1, gix@0.77.0, globset@0.4.18, ignore@0.4.25, parking_lot@0.12.5, pasetors@0.7.7, prodash@30.0.1, rand@0.9.2, regex-automata@0.4.13, strsim@0.11.1, tempfile@3.24.0, thread_local@1.1.9, toml@0.9.10+spec-1.1.0, toml_edit@0.24.0+spec-1.1.0, tracing-chrome@0.7.2, tracing-core@0.1.36, tracing-log@0.2.0, tracing-subscriber@0.3.22, tracing@0.1.44。代表：`SoN construction: unsupported: TLS is outside the semantic path`。
- **atomic_store**：anstream@0.6.21, arc-swap@1.7.1, cargo-util@0.2.27, cargo@0.95.0, clap_builder@4.5.53, colorchoice@1.0.4, crossbeam-channel@0.5.15, crossbeam-epoch@0.9.18, crossbeam-utils@0.8.21, curl@0.4.49, dashmap@6.1.0, faster-hex@0.10.0, filetime@0.2.26, foldhash@0.1.5, foldhash@0.2.0, getrandom@0.2.16, getrandom@0.3.4, git2-curl@0.21.0, git2@0.20.3, gix-features@0.45.1, gix-filter@0.24.0, gix-index@0.45.0, gix-odb@0.74.0, gix-pack@0.64.0, gix-transport@0.52.1, gix@0.77.0, globset@0.4.18, ignore@0.4.25, jiff@0.2.17, jobserver@0.1.34, log@0.4.29, memchr@2.7.6, memmap2@0.9.9, nix@0.30.1, once_cell@1.21.3, parking_lot@0.12.5, parking_lot_core@0.9.12, pasetors@0.7.7, prodash@30.0.1, rusqlite@0.38.0, rustix@1.1.3, sha1@0.10.6, sha2@0.10.9, sharded-slab@0.1.7, tempfile@3.24.0, thread_local@1.1.9, tracing-chrome@0.7.2, tracing-core@0.1.36, tracing-log@0.2.0, tracing-subscriber@0.3.22, zlib-rs@0.5.5。代表：`MIR conversion: intrinsic `atomic_store` is not implemented`。
- **atomic_xchg**：anstream@0.6.21, arc-swap@1.7.1, cargo-util@0.2.27, cargo@0.95.0, clap_builder@4.5.53, crossbeam-utils@0.8.21, curl@0.4.49, git2-curl@0.21.0, gix-features@0.45.1, gix-filter@0.24.0, gix-odb@0.74.0, gix-pack@0.64.0, gix-transport@0.52.1, gix@0.77.0, globset@0.4.18, ignore@0.4.25, jobserver@0.1.34, once_cell@1.21.3, parking_lot@0.12.5, pasetors@0.7.7, rusqlite@0.38.0, sharded-slab@0.1.7, thread_local@1.1.9, tracing-chrome@0.7.2, tracing-core@0.1.36, tracing-subscriber@0.3.22。代表：`MIR conversion: intrinsic `atomic_xchg` is not implemented`。
- **atomic_cxchg**：arc-swap@1.7.1, bstr@1.12.1, bytes@1.11.0, cargo-util@0.2.27, cargo@0.95.0, crossbeam-channel@0.5.15, crossbeam-epoch@0.9.18, crossbeam-utils@0.8.21, dashmap@6.1.0, git2-curl@0.21.0, gix-diff@0.57.0, gix-features@0.45.1, gix-filter@0.24.0, gix-index@0.45.0, gix-lock@20.0.0, gix-odb@0.74.0, gix-pack@0.64.0, gix-prompt@0.12.0, gix-protocol@0.55.0, gix-ref@0.57.0, gix-shallow@0.7.0, gix-tempfile@20.0.0, gix-transport@0.52.1, gix@0.77.0, globset@0.4.18, ignore@0.4.25, jiff@0.2.17, jobserver@0.1.34, log@0.4.29, once_cell@1.21.3, parking_lot@0.12.5, parking_lot_core@0.9.12, pasetors@0.7.7, prodash@30.0.1, rusqlite@0.38.0, sharded-slab@0.1.7, thread_local@1.1.9, tracing-chrome@0.7.2, tracing-core@0.1.36, tracing-subscriber@0.3.22。代表：`MIR conversion: intrinsic `atomic_cxchg` is not implemented`。
- **atomic_cxchgweak**：arc-swap@1.7.1, cargo@0.95.0, crossbeam-epoch@0.9.18, dashmap@6.1.0, foldhash@0.1.5, foldhash@0.2.0, getrandom@0.3.4, gix-diff@0.57.0, gix-features@0.45.1, gix-index@0.45.0, gix-lock@20.0.0, gix-odb@0.74.0, gix-pack@0.64.0, gix-prompt@0.12.0, gix-protocol@0.55.0, gix-ref@0.57.0, gix-shallow@0.7.0, gix-tempfile@20.0.0, gix-transport@0.52.1, gix@0.77.0, ignore@0.4.25, jiff@0.2.17, parking_lot@0.12.5, parking_lot_core@0.9.12, prodash@30.0.1, tracing-chrome@0.7.2, tracing-core@0.1.36, tracing-subscriber@0.3.22。代表：`MIR conversion: intrinsic `atomic_cxchgweak` is not implemented`。
- **float-other**：cargo-credential@0.4.9, cargo-util-schemas@0.12.0, cargo@0.95.0, clap_builder@4.5.53, crates-io@0.40.17, curl@0.4.49, erased-serde@0.4.9, gix-diff@0.57.0, gix-features@0.45.1, gix-odb@0.74.0, gix-pack@0.64.0, gix@0.77.0, jiff@0.2.17, nu-ansi-term@0.50.3, num-traits@0.2.19, pasetors@0.7.7, prodash@30.0.1, rusqlite@0.38.0, serde-untagged@0.1.9, serde-value@0.7.0, serde@1.0.228, serde_core@1.0.228, serde_json@1.0.148, toml@0.9.10+spec-1.1.0, toml_edit@0.24.0+spec-1.1.0, tracing-chrome@0.7.2, tracing-subscriber@0.3.22。代表：`SoN construction: unsupported: unsupported semantic type f32`。
- **float-ABI**：cargo-credential@0.4.9, cargo@0.95.0, clap_builder@4.5.53, crates-io@0.40.17, curl@0.4.49, erased-serde@0.4.9, fastrand@2.3.0, git2@0.20.3, gix-diff@0.57.0, gix-features@0.45.1, gix-index@0.45.0, gix-lock@20.0.0, gix-pack@0.64.0, gix-status@0.24.0, gix-transport@0.52.1, gix@0.77.0, jiff@0.2.17, nu-ansi-term@0.50.3, num-traits@0.2.19, ordered-float@2.10.1, pasetors@0.7.7, prodash@30.0.1, rand@0.9.2, rusqlite@0.38.0, serde-value@0.7.0, serde_core@1.0.228, serde_json@1.0.148, strsim@0.11.1, time@0.3.44, toml@0.9.10+spec-1.1.0, toml_edit@0.24.0+spec-1.1.0, tracing-core@0.1.36, tracing-log@0.2.0, tracing-subscriber@0.3.22, zerocopy@0.8.31, zmij@1.0.8。代表：`MIR conversion: floating-point scalar ABI is not implemented`。
- **compiler-panic**：cargo@0.95.0, curl@0.4.49, gix-diff@0.57.0, gix-negotiate@0.25.0, gix-object@0.54.0, gix-odb@0.74.0, gix-protocol@0.55.0, gix-ref@0.57.0, gix-status@0.24.0, gix@0.77.0, regex-automata@0.4.13, tracing-subscriber@0.3.22。代表：`compiler panic in owned graph pipeline: inline nodes have at most two inputs`。
- **volatile-memory**：cargo@0.95.0, crypto-bigint@0.5.5, der@0.7.10, ed25519-compact@2.2.0, gix-pack@0.64.0, gix@0.77.0, ignore@0.4.25, orion@0.17.11, pasetors@0.7.7, zeroize@1.8.2。代表：`MIR conversion: intrinsic `volatile_store` is not implemented`。
- **aggregate-constant**：cargo-util-schemas@0.12.0, cargo-util@0.2.27, cargo@0.95.0, clap_builder@4.5.53, clap_complete@4.5.64, crossbeam-channel@0.5.15, curl@0.4.49, dashmap@6.1.0, der@0.7.10, git2-curl@0.21.0, git2@0.20.3, gix-commitgraph@0.31.0, gix-config@0.50.0, gix-credentials@0.34.1, gix-date@0.12.0, gix-diff@0.57.0, gix-features@0.45.1, gix-filter@0.24.0, gix-hash@0.21.1, gix-index@0.45.0, gix-object@0.54.0, gix-odb@0.74.0, gix-pack@0.64.0, gix-prompt@0.12.0, gix-quote@0.6.1, gix-ref@0.57.0, gix-refspec@0.35.0, gix-shallow@0.7.0, gix-status@0.24.0, gix-transport@0.52.1, gix-url@0.34.0, gix@0.77.0, glob@0.3.3, icu_locale_core@2.1.1, ignore@0.4.25, jiff@0.2.17, jobserver@0.1.34, opener@0.8.3, parking_lot@0.12.5, prodash@30.0.1, rand@0.9.2, regex-automata@0.4.13, regex-syntax@0.8.8, rusqlite@0.38.0, serde-value@0.7.0, serde_json@1.0.148, tar@0.4.44, tempfile@3.24.0, thread_local@1.1.9, tracing-chrome@0.7.2, tracing-core@0.1.36, tracing-log@0.2.0, tracing-subscriber@0.3.22, tracing@0.1.44, unicode-bom@2.0.3。代表：`SoN construction: unsupported in bb15: representation-encoded aggregate constant requires semantic constant decoding`。
- **unwind**：cargo@0.95.0, curl@0.4.49, git2@0.20.3, gix-filter@0.24.0, gix-index@0.45.0, gix-pack@0.64.0, gix-transport@0.52.1, gix@0.77.0, ignore@0.4.25, jobserver@0.1.34, rusqlite@0.38.0, tracing-chrome@0.7.2。代表：`MIR conversion: intrinsic `catch_unwind` is not implemented`。
- **uninhabited-value**：cargo@0.95.0, clap_builder@4.5.53, clap_complete@4.5.64, crates-io@0.40.17, dashmap@6.1.0, gix-command@0.6.4, gix-odb@0.74.0, gix-ref@0.57.0, gix@0.77.0, ignore@0.4.25, jiff@0.2.17, tempfile@3.24.0, time@0.3.44, toml@0.9.10+spec-1.1.0, tracing-core@0.1.36, tracing-log@0.2.0。代表：`layout lowering: unsupported in layout lowering: ignored enum is not uniquely inhabited`。
- **dyn**：cargo@0.95.0, gix-transport@0.52.1。代表：`MIR conversion: trait-object supertrait upcast from `&dyn std::error::Error` to `&dyn std::fmt::Display` is not supported yet`。
- **C-variadic**：cargo-credential-libsecret@0.5.5, cargo-util@0.2.27, cargo@0.95.0, curl@0.4.49, dashmap@6.1.0, filetime@0.2.26, getrandom@0.2.16, getrandom@0.3.4, git2@0.20.3, gix-transport@0.52.1, jobserver@0.1.34, memmap2@0.9.9, openssl-sys@0.9.111, parking_lot@0.12.5, parking_lot_core@0.9.12, socket2@0.6.1。代表：`MIR conversion: C-variadic functions are not implemented`。
- **atomic_or**：cargo@0.95.0, crossbeam-epoch@0.9.18, gix-features@0.45.1, gix-pack@0.64.0, gix-transport@0.52.1, gix@0.77.0, parking_lot@0.12.5, tracing-chrome@0.7.2。代表：`MIR conversion: intrinsic `atomic_or` is not implemented`。
- **atomic_singlethreadfence**：cargo@0.95.0, crossbeam-epoch@0.9.18, der@0.7.10, ed25519-compact@2.2.0, ignore@0.4.25, orion@0.17.11, pasetors@0.7.7, zeroize@1.8.2。代表：`MIR conversion: intrinsic `atomic_singlethreadfence` is not implemented`。
- **semantic-place-type**：cargo@0.95.0, tracing-core@0.1.36, tracing-subscriber@0.3.22。代表：`SoN construction: unsupported in bb0: assigned value disagrees with the semantic place type`。
- **assembly**：constant_time_eq@0.3.1, faster-hex@0.10.0, gix-index@0.45.0, gix-prompt@0.12.0, rustix@1.1.3, sha1@0.10.6, sha2@0.10.9, tempfile@3.24.0, terminal_size@0.4.3。代表：`MIR conversion: terminator "InlineAsm"`。
- **static-linkage**：curl@0.4.49。代表：`static `INIT_CTOR` has an unsupported link section or import linkage`。
- **enum-phi-shape**：orion@0.17.11, pasetors@0.7.7。代表：`layout lowering: malformed layout input: Phi field shape mismatch`。
- **atomic_and**：parking_lot@0.12.5, parking_lot_core@0.9.12。代表：`MIR conversion: intrinsic `atomic_and` is not implemented`。

### 精确错误签名

同类别的不同失败层/错误签名单列，尤其保留SoN与Cranelift panic差异。

| 错误签名 | 项数 | target units | 代表crate / 函数 |
|---|---:|---:|---|
| SoN construction: unsupported: unsupported semantic type f64 | 6357 | 23 | cargo@0.95.0 / serde_ignored::deserialize::<'_, toml::Deserializer<'_>, {closure@src/cargo/util/toml/mod.rs:193:89: 193:95}, cargo_util_schemas::manifest::TomlManife |
| SoN construction: unsupported: union value is outside the semantic path: erased_serde::any::Value#4593a8e425866e41-1d18f79556f99c2d | 3877 | 2 | cargo@0.95.0 / erased_serde::any::Any::new::<serde_ignored::Wrap<'_, '_, core::dependency::_::_serde::de::value::private::UnitOnly<toml::de::Error>, {closure@src/car |
| MIR conversion: unsupported scalar ABI ScalarAbi { bits: 128, extension: None } | 1374 | 61 | aho-corasick@1.1.4 / std_detect::detect::cache::test |
| SoN construction: unsupported: SIMD value is outside the semantic path: std::arch::x86_64::__m128i#67c1bd76a8cc8674-84f70887d6d2298a | 1152 | 39 | annotate-snippets@0.12.10 / hashbrown::raw::RawTableInner::resize_inner::<std::alloc::Global> |
| MIR conversion: floating-point scalar ABI is not implemented | 1036 | 37 | cargo@0.95.0 / core::bool::<impl bool>::then_some::<f64> |
| SoN construction: unsupported: unsupported semantic type f32 | 586 | 10 | cargo@0.95.0 / gix::open::<&std::path::PathBuf> |
| MIR conversion: intrinsic `atomic_xsub` is not implemented | 581 | 55 | aho-corasick@1.1.4 / <alloc::sync::Arc<dyn ahocorasick::AcAutomaton> as core::ops::Drop>::drop |
| SoN construction: unsupported: union value is outside the semantic path: std::sync::lazy_lock::Data<std::backtrace::Capture, {closure@std::backtrace::helper::lazy_resolve::{closure#0}}>#90d1a56733746958-823052a7bacf9 | 494 | 5 | anyhow@1.0.100 / error::<impl Error>::construct_from_adhoc::<alloc::string::String> |
| SoN construction: unsupported: unsupported semantic type u128 | 405 | 40 | aho-corasick@1.1.4 / util::prefilter::Builder::ascii_case_insensitive |
| SoN construction: unsupported: union value is outside the semantic path: kstring::string::inner::KStringInner<std::boxed::Box<str>>#564cdc8937913746-c4b647f87c215485 | 361 | 7 | cargo@0.95.0 / core::bool::<impl bool>::then::<(gix::filter::gix_filter::Pipeline, gix::worktree::Stack), {closure@gix::status::gix_status::index_as_worktree_with_re |
| MIR conversion: intrinsic `atomic_xadd` is not implemented | 282 | 42 | aho-corasick@1.1.4 / <alloc::sync::Arc<dyn util::prefilter::PrefilterI> as core::clone::Clone>::clone |
| machine lowering: malformed graph: control node n124 is unreachable | 239 | 35 | aho-corasick@1.1.4 / alloc::collections::btree::navigate::<impl alloc::collections::btree::node::Handle<alloc::collections::btree::node::NodeRef<alloc::collections::btree: |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::arch::x86_64::__m128i#67c1bd76a8cc8674-84f70887d6d2298a | 232 | 12 | aho-corasick@1.1.4 / packed::teddy::generic::Teddy::<8>::verify::<core::arch::x86_64::__m128i> |
| MIR conversion: intrinsic `atomic_load` is not implemented | 227 | 93 | aho-corasick@1.1.4 / core::sync::atomic::atomic_load::<*mut ()> |
| machine lowering: malformed graph: control node n108 is unreachable | 220 | 35 | aho-corasick@1.1.4 / alloc::collections::btree::navigate::LazyLeafRange::<alloc::collections::btree::node::marker::Dying, alloc::boxed::Box<[u8]>, usize>::init_front |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::arch::x86_64::__m256i#fba485d6d644d376-331e338a37203e0b | 211 | 5 | aho-corasick@1.1.4 / packed::teddy::generic::Teddy::<8>::verify::<core::arch::x86_64::__m256i> |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::i8x16#4de202288ac8fa56-9422adfea3fe85ee | 199 | 47 | aho-corasick@1.1.4 / core::arch::x86_64::_mm_alignr_epi8::<13> |
| SoN construction: unsupported: unsupported semantic type i128 | 150 | 13 | bitflags@2.10.0 / std::result::Result::<i128, std::num::ParseIntError>::map_err::<parser::ParseError, {closure@/opt/cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ |
| machine lowering: malformed graph: control node n169 is unreachable | 147 | 34 | aho-corasick@1.1.4 / alloc::collections::btree::navigate::LazyLeafRange::<alloc::collections::btree::node::marker::Dying, alloc::boxed::Box<[u8]>, usize>::take_front |
| MIR conversion: intrinsic `catch_unwind` is not implemented | 147 | 13 | cargo@0.95.0 / std::thread::scope::<'_, {closure@gix::parallel::in_parallel_with_slice<gix::gix_odb::gix_pack::cache::delta::tree::Item<gix::gix_odb::gix_pack::index |
| SoN construction: unsupported in bb9: representation-encoded aggregate constant requires semantic constant decoding | 128 | 38 | cargo@0.95.0 / std::io::default_read_buf_exact::<std::io::BufReader<gix::gix_features::interrupt::Read<'_, gix::progress::Read<&mut dyn std::io::BufRead, gix::progre |
| machine lowering: malformed graph: control node n102 is unreachable | 127 | 22 | aho-corasick@1.1.4 / alloc::collections::btree::search::<impl alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Mut<'_>, alloc::boxed::Box< |
| MIR conversion: intrinsic `atomic_xchg` is not implemented | 124 | 27 | anstream@0.6.21 / <std::sync::ReentrantLockGuard<'_, std::cell::RefCell<std::io::LineWriter<std::io::stdio::StdoutRaw>>> as std::ops::Drop>::drop |
| SoN construction: unsupported: union value is outside the semantic path: memchr::memmem::searcher::SearcherKind#a279c25f26c61910-e09fd91d8c712f7 | 120 | 13 | aho-corasick@1.1.4 / memchr::memmem::Finder::<'_>::new::<alloc::vec::Vec<u8>> |
| MIR conversion: intrinsic `atomic_store` is not implemented | 109 | 52 | anstream@0.6.21 / std::sync::atomic::atomic_store::<u64> |
| SoN construction: unsupported: SIMD value is outside the semantic path: hashbrown::control::group::sse2::Group#43962360bfb6ad4d-77d0cfbe08b0e5b9 | 105 | 10 | cargo@0.95.0 / hashbrown::raw::RawIterRange::<(gix::ObjectId, gix::gix_revwalk::graph::Commit<gix::gix_negotiate::Metadata>)>::next_impl::<false> |
| compiler panic in owned graph pipeline: inline nodes have at most two inputs | 97 | 12 | cargo@0.95.0 / gix::status::index_worktree::<impl gix::Repository>::index_worktree_status_pathspec::<gix::status::index_worktree::Error, gix::bstr::BString, std::vec |
| machine lowering: unsupported in machine lowering: values of type `f64` | 97 | 7 | cargo@0.95.0 / hashbrown::raw::RawTable::<(core::compiler::job_queue::JobId, core::compiler::timings::UnitTime)>::remove_entry::<{closure@hashbrown::map::equivalent_ |
| SoN construction: unsupported: union value is outside the semantic path: core::escape::MaybeEscapedCharacter<10>#9370681a58d76310-d6e95c04666c705d | 93 | 33 | anyhow@1.0.100 / <context::Quoted<&mut core::fmt::Formatter<'_>> as core::fmt::Write>::write_str |
| MIR conversion: intrinsic `atomic_or` is not implemented | 92 | 9 | cargo@0.95.0 / std::sync::mpmc::list::Channel::<gix::status::gix_status::index_as_worktree_with_renames::function::Event<'_, (), gix::submodule::Status>>::disconnect |
| MIR conversion: intrinsic `atomic_cxchg` is not implemented | 84 | 41 | arc-swap@1.7.1 / std::sync::atomic::atomic_compare_exchange::<usize> |
| SoN construction: unsupported: SIMD value is outside the semantic path: std::arch::x86_64::__m256i#fba485d6d644d376-331e338a37203e0b | 82 | 13 | bitmaps@2.1.0 / bitmap::x86_arch::<impl bitmap::Bitmap<typenum::UInt<typenum::UInt<typenum::UInt<typenum::UInt<typenum::UInt<typenum::UInt<typenum::UInt<typenum::UInt |
| SoN construction: unsupported: TLS is outside the semantic path | 81 | 42 | annotate-snippets@0.12.10 / std::hash::RandomState::new::KEYS::{constant#0}::{closure#1} |
| semantic verification: graph verification failed: TypeMismatch { node: NodeId(0), detail: "type is outside the specialized value graph subset" } | 80 | 3 | aho-corasick@1.1.4 / core::arch::x86_64::_mm256_permute2x128_si256::<33> |
| SoN construction: unsupported in bb0: unsized field requires a producer-proven static slice/string tail | 66 | 19 | aho-corasick@1.1.4 / alloc::sync::Arc::<dyn ahocorasick::AcAutomaton>::drop_slow |
| semantic verification: graph verification failed: TypeMismatch { node: NodeId(69), detail: "semantic aggregate constants require value constructors" } | 62 | 14 | cargo@0.95.0 / <std::option::Option<std::vec::Vec<std::string::String>> as std::cmp::PartialOrd>::partial_cmp |
| MIR conversion: C-variadic functions are not implemented | 61 | 16 | cargo@0.95.0 / curl::easy::Easy2::<curl::easy::handle::EasyData>::setopt_ptr |
| semantic verification: graph verification failed: TypeMismatch { node: NodeId(7), detail: "semantic aggregate constants require value constructors" } | 56 | 12 | cargo@0.95.0 / std::cell::RefCell::<std::option::Option<git2::Tree<'_>>>::new |
| SoN construction: unsupported: SIMD value is outside the semantic path: hashbrown::raw::inner::sse2::Group#799b310d19595db-b1f1603e4cee533b | 55 | 10 | cargo@0.95.0 / hashbrown::raw::inner::sse2::Group::match_empty |
| MIR conversion: intrinsic `atomic_fence` is not implemented | 53 | 53 | aho-corasick@1.1.4 / core::sync::atomic::fence |
| machine lowering: unsupported in machine lowering: values of type `f32` | 52 | 4 | cargo@0.95.0 / std::option::Option::<serde_value::Value>::Some |
| definition specialization: unsupported scalar ABI ScalarAbi { bits: 128, extension: None } | 47 | 5 | blake3@1.8.2 / std::cmp::min::<i128> |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::i8x32#db30f0c4957e1ff0-7946e52d8322e478 | 46 | 16 | aho-corasick@1.1.4 / core::arch::x86_64::_mm256_alignr_epi8::<13> |
| MIR conversion: intrinsic `atomic_cxchgweak` is not implemented | 46 | 29 | arc-swap@1.7.1 / std::sync::atomic::atomic_compare_exchange_weak::<*mut debt::list::Node> |
| SoN construction: unsupported in bb15: representation-encoded aggregate constant requires semantic constant decoding | 46 | 14 | cargo@0.95.0 / std::io::append_to_string::<{closure@std::io::default_read_to_string<flate2::read::GzDecoder<&std::fs::File>>::{closure#0}}> |
| SoN construction: unsupported in bb14: representation-encoded aggregate constant requires semantic constant decoding | 43 | 15 | cargo@0.95.0 / std::io::default_read_exact::<std::io::BufReader<gix::gix_features::interrupt::Read<'_, gix::progress::Read<&mut dyn std::io::BufRead, gix::progress:: |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::i32x4#b32f3b256dcc2d93-64989a8efc606e2e | 39 | 5 | crc32fast@1.5.0 / std::arch::x86_64::_mm_extract_epi32::<1> |
| layout lowering: unsupported in layout lowering: cannot reconstruct an inhabited value of Never | 36 | 12 | cargo@0.95.0 / <std::result::Result<std::vec::in_place_drop::InPlaceDrop<std::borrow::Cow<'_, str>>, !> as std::ops::FromResidual<std::result::Result<std::convert::I |
| layout lowering: unsupported in layout lowering: uninhabited variant construction | 32 | 10 | cargo@0.95.0 / std::sync::OnceLock::<std::vec::Vec<(std::string::String, util::context::target::TargetCfgConfig)>>::initialize::<{closure@std::sync::OnceLock<std::ve |
| SoN construction: unsupported: union value is outside the semantic path: core::escape::MaybeEscapedCharacter<4>#85c3c6670cf6ff0b-5bbda9fdbd4456f7 | 31 | 30 | aho-corasick@1.1.4 / <util::debug::DebugByte as core::fmt::Debug>::fmt |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::i64x4#b53314c51c8f39bb-8a3b8ce4aa781344 | 28 | 6 | aho-corasick@1.1.4 / core::arch::x86_64::_mm256_permute2f128_si256::<33> |
| machine lowering: unsupported in machine lowering: 128-bit integers | 28 | 6 | gix-date@0.12.0 / <std::option::Option<jiff::util::rangeint::ri128<-377705023201000000000, 253402207200999999999>> as std::ops::FromResidual<std::option::Option<std::co |
| MIR conversion: intrinsic `volatile_store` is not implemented | 27 | 7 | cargo@0.95.0 / std::ptr::write_volatile::<u8> |
| MIR conversion: LLVM intrinsic `llvm.x86.sse2.pause` | 27 | 18 | cargo@0.95.0 / std::sync::Arc::<gix::progress::tree::Root>::downgrade |
| SoN construction: unsupported: union value is outside the semantic path: memchr::memmem::searcher::PrefilterKind#aa9f9ffbcc8dacf4-801ccfc7ead00cee | 24 | 12 | aho-corasick@1.1.4 / memchr::memmem::searcher::Searcher::new::<memchr::arch::all::packedpair::DefaultFrequencyRank> |
| SoN construction: unsupported: union value is outside the semantic path: ppv_lite86::x86_64::vec128_storage#43b8875c4d0b1326-84070288d39b2a67 | 24 | 2 | cargo@0.95.0 / std::result::Result::<rand_chacha::chacha::ChaCha12Core, rand::rand_core::OsError>::map::<(), {closure@rand::rngs::reseeding::ReseedingCore<rand_chach |
| SoN construction: unsupported: union value is outside the semantic path: ppv_lite86::vec128_storage#43b8875c4d0b1326-84070288d39b2a67 | 24 | 1 | rand_chacha@0.9.0 / guts::init_chacha::fn_impl::<ppv_lite86::x86_64::SseMachine<ppv_lite86::x86_64::NoS3, ppv_lite86::x86_64::NoS4, ppv_lite86::x86_64::NoNI>> |
| definition specialization: floating-point scalar ABI is not implemented | 23 | 7 | cargo@0.95.0 / std::option::Option::<f64>::Some |
| MIR conversion: terminator "InlineAsm" | 23 | 9 | constant_time_eq@0.3.1 / optimizer_hide |
| machine lowering: malformed graph: control node n111 is unreachable | 21 | 7 | cargo@0.95.0 / alloc::collections::btree::search::<impl alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Mut<'_>, std::string::Strin |
| SoN construction: unsupported: SIMD value is outside the semantic path: ppv_lite86::x86_64::sse2::u32x4_sse2<ppv_lite86::x86_64::YesS3, ppv_lite86::x86_64::YesS4, ppv_lite86::x86_64::NoNI>#77aef4c0a5bc17e2-bd4494d921e68cc1 | 20 | 1 | rand_chacha@0.9.0 / guts::refill_wide_impl::<ppv_lite86::x86_64::SseMachine<ppv_lite86::x86_64::YesS3, ppv_lite86::x86_64::YesS4, ppv_lite86::x86_64::NoNI>> |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::i64x2#5c6c7e16162ee2bd-14b51f0e3e172e | 19 | 6 | aho-corasick@1.1.4 / core::arch::x86_64::_mm256_broadcastsi128_si256 |
| SoN construction: unsupported: SIMD value is outside the semantic path: ppv_lite86::x86_64::sse2::u32x4_sse2<ppv_lite86::x86_64::NoS3, ppv_lite86::x86_64::NoS4, ppv_lite86::x86_64::NoNI>#591d9e09f5e633a7-463eaf626ab3c900 | 19 | 1 | rand_chacha@0.9.0 / guts::refill_wide_impl::<ppv_lite86::x86_64::SseMachine<ppv_lite86::x86_64::NoS3, ppv_lite86::x86_64::NoS4, ppv_lite86::x86_64::NoNI>> |
| SoN construction: unsupported: SIMD value is outside the semantic path: hashbrown::control::group::sse2::Group#96988ed59efa53b5-373d61a9558b51f9 | 17 | 3 | cargo@0.95.0 / hashbrown::control::group::sse2::Group::match_empty |
| SoN construction: unsupported: SIMD value is outside the semantic path: std::simd::Simd<i8, 1>#babfc436dcd0b2f3-c5ad70aee1feef5b | 15 | 15 | cargo@0.95.0 / std::simd::Simd::<T, N>::splat::splat_rt::<i8, 16> |
| SoN construction: unsupported: SIMD value is outside the semantic path: std::simd::Simd<u8, 1>#a121cd8074903a21-87cddfd4304b71c6 | 15 | 15 | cargo@0.95.0 / std::simd::Simd::<T, N>::splat::splat_rt::<u8, 16> |
| SoN construction: unsupported: SIMD value is outside the semantic path: std::simd::Simd<i8, 16>#9bf4305c7d6ea96a-95134b9d5703dc97 | 15 | 15 | cargo@0.95.0 / std::str::pattern::simd_contains::{closure#3} |
| SoN construction: unsupported: SIMD value is outside the semantic path: std::simd::Simd<i8, 64>#a299f5919a1a8597-e8a8de458c297305 | 15 | 15 | cargo@0.95.0 / core::core_simd::masks::mask_impl::Mask::<i8, 16>::to_bitmask_integer |
| SoN construction: unsupported: SIMD value is outside the semantic path: std::simd::Simd<u8, 16>#e1f034a248ac678d-7599364c536942c0 | 15 | 15 | cargo@0.95.0 / std::str::pattern::simd_contains |
| layout lowering: unsupported in layout lowering: ignored enum is not uniquely inhabited | 14 | 9 | cargo@0.95.0 / util::context::GlobalContext::get_config_env::<std::string::String>::{closure#0} |
| semantic verification: graph verification failed: TypeMismatch { node: NodeId(4), detail: "type is outside the specialized value graph subset" } | 14 | 2 | memchr@2.7.6 / arch::generic::memchr::One::<core::arch::x86_64::__m128i>::new |
| SoN construction: unsupported: SIMD value is outside the semantic path: ppv_lite86::x86_64::sse2::u32x4_sse2<ppv_lite86::x86_64::YesS3, ppv_lite86::x86_64::NoS4, ppv_lite86::x86_64::NoNI>#d69d41d34bcfbd89-7d99ec786648d998 | 14 | 1 | rand_chacha@0.9.0 / guts::refill_wide_impl::<ppv_lite86::x86_64::SseMachine<ppv_lite86::x86_64::YesS3, ppv_lite86::x86_64::NoS4, ppv_lite86::x86_64::NoNI>> |
| SoN construction: unsupported: SIMD value is outside the semantic path: ppv_lite86::x86_64::sse2::avx2::u32x4x2_avx2<ppv_lite86::x86_64::NoNI>#72d9436466b8103a-39d5a7ef183ab42b | 14 | 1 | rand_chacha@0.9.0 / guts::refill_wide_impl::<ppv_lite86::x86_64::Avx2Machine<ppv_lite86::x86_64::NoNI>> |
| semantic verification: graph verification failed: TypeMismatch { node: NodeId(6), detail: "semantic aggregate constants require value constructors" } | 13 | 10 | cargo@0.95.0 / std::io::impls::<impl std::io::Write for std::vec::Vec<u8>>::flush |
| semantic verification: graph verification failed: TypeMismatch { node: NodeId(12), detail: "semantic aggregate constants require value constructors" } | 12 | 6 | cargo@0.95.0 / <std::cell::RefCell<std::option::Option<u64>> as std::default::Default>::default |
| SoN construction: unsupported: SIMD value is outside the semantic path: std::mem::MaybeDangling<std::arch::x86_64::__m128i>#cd4fdcfebd430fb7-29e0b5a9328b6ad9 | 12 | 1 | zlib-rs@0.5.5 / std::array::try_from_fn_erased::<std::ops::try_trait::NeverShortCircuit<std::arch::x86_64::__m128i>, std::ops::try_trait::Wrapped<std::arch::x86_64::_ |
| MIR conversion: intrinsic `size_of_val` on unsized `std::path::Path` | 11 | 11 | cargo@0.95.0 / <std::boxed::Box<std::path::Path> as std::ops::Drop>::drop |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::u64x4#e36d1c13dcef8303-a1dec524aa76c240 | 10 | 2 | faster-hex@0.10.0 / std::arch::x86_64::_mm256_srli_epi64::<4> |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::u64x2#4b0ac128a898a4da-c71f3d313f1efdd2 | 10 | 2 | faster-hex@0.10.0 / std::arch::x86_64::_mm_srli_epi64::<4> |
| SoN construction: unsupported: SIMD value is outside the semantic path: bitflags::__private::core::core_arch::simd::i8x16#4de202288ac8fa56-9422adfea3fe85ee | 10 | 2 | gix-negotiate@0.25.0 / std::arch::x86_64::_mm_set1_epi8 |
| semantic verification: graph verification failed: TypeMismatch { node: NodeId(35), detail: "semantic aggregate constants require value constructors" } | 8 | 4 | cargo@0.95.0 / jiff::shared::posix::<impl jiff::shared::PosixTimeZone<jiff::shared::util::array_str::ArrayStr<30>>>::dst_info_wall |
| MIR conversion: intrinsic `atomic_singlethreadfence` is not implemented | 8 | 8 | cargo@0.95.0 / std::sync::atomic::compiler_fence |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::i32x8#c87367e184f5fe05-4dfcb0351916e3d6 | 8 | 4 | faster-hex@0.10.0 / std::arch::x86_64::_mm256_or_si256 |
| SoN construction: unsupported: SIMD value is outside the semantic path: ppv_lite86::x86_64::sse2::u64x2_sse2<ppv_lite86::x86_64::YesS3, ppv_lite86::x86_64::YesS4, ppv_lite86::x86_64::NoNI>#51ab4675fa2582eb-bdd437fbd0939d3 | 8 | 1 | rand_chacha@0.9.0 / guts::d0123::<ppv_lite86::x86_64::SseMachine<ppv_lite86::x86_64::YesS3, ppv_lite86::x86_64::YesS4, ppv_lite86::x86_64::NoNI>> |
| SoN construction: unsupported in bb11: representation-encoded aggregate constant requires semantic constant decoding | 7 | 7 | cargo@0.95.0 / std::io::impls::<impl std::io::Read for &[u8]>::read_exact |
| layout lowering: unsupported in layout lowering: load of an uninhabited enum | 7 | 1 | time@0.3.44 / parsing::parsable::<impl parsing::parsable::sealed::Sealed for format_description::well_known::rfc2822::Rfc2822>::parse_into |
| semantic verification: graph verification failed: TypeMismatch { node: NodeId(28), detail: "type is outside the specialized value graph subset" } | 6 | 2 | cargo@0.95.0 / <std::result::Result<f64, anyhow::Error> as std::ops::Try>::branch |
| MIR conversion: LLVM intrinsic `llvm.x86.pclmulqdq` | 6 | 2 | crc32fast@1.5.0 / std::arch::x86_64::_mm_clmulepi64_si128::<0> |
| SoN construction: unsupported: SIMD value is outside the semantic path: ppv_lite86::x86_64::sse2::u64x2_sse2<ppv_lite86::x86_64::NoS3, ppv_lite86::x86_64::NoS4, ppv_lite86::x86_64::NoNI>#cc36bde4d59019af-856e9038de549cc8 | 6 | 1 | rand_chacha@0.9.0 / guts::d0123::<ppv_lite86::x86_64::SseMachine<ppv_lite86::x86_64::NoS3, ppv_lite86::x86_64::NoS4, ppv_lite86::x86_64::NoNI>> |
| SoN construction: unsupported: SIMD value is outside the semantic path: ppv_lite86::x86_64::sse2::u64x2_sse2<ppv_lite86::x86_64::YesS3, ppv_lite86::x86_64::NoS4, ppv_lite86::x86_64::NoNI>#4142ea2f32b3ccdb-e03b50370ac58ff5 | 6 | 1 | rand_chacha@0.9.0 / guts::d0123::<ppv_lite86::x86_64::SseMachine<ppv_lite86::x86_64::YesS3, ppv_lite86::x86_64::NoS4, ppv_lite86::x86_64::NoNI>> |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::u32x4#bfffc437a4290f0c-a85f5553e0513eb | 6 | 1 | rand_chacha@0.9.0 / std::arch::x86_64::_mm_slli_epi32::<7> |
| MIR conversion: LLVM intrinsic `llvm.x86.ssse3.pshuf.b.128` | 5 | 5 | aho-corasick@1.1.4 / core::arch::x86_64::_mm_shuffle_epi8 |
| MIR conversion: intrinsic `volatile_load` is not implemented | 5 | 5 | crypto-bigint@0.5.5 / core::ptr::read_volatile::<u8> |
| SoN construction: unsupported: union value is outside the semantic path: zerovec::ule::NichedOptionULE<icu_locale_core::subtags::Script, 4>#a874729afa248840-cc3e7dc7269fdd40 | 5 | 1 | icu_properties@2.1.2 / core::option::Option::<zerovec::ule::NichedOptionULE<icu_locale_core::subtags::Script, 4>>::map::<zerovec::ule::NichedOption<icu_locale_core::subtags: |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::u32x8#25580cf97e53993d-6d118ada1c8f604e | 5 | 2 | rand_chacha@0.9.0 / std::arch::x86_64::_mm256_slli_epi32::<7> |
| MIR conversion: LLVM intrinsic `llvm.x86.avx2.pshuf.b` | 4 | 4 | aho-corasick@1.1.4 / core::arch::x86_64::_mm256_shuffle_epi8 |
| SoN construction: unsupported in bb13: representation-encoded aggregate constant requires semantic constant decoding | 4 | 1 | cargo@0.95.0 / std::sys::net::connection::each_addr::<&std::net::SocketAddr, for<'a> fn(&'a std::net::SocketAddr) -> std::result::Result<std::sys::net::connection::s |
| machine lowering: malformed graph: control node n98 is unreachable | 4 | 1 | cargo@0.95.0 / std::collections::BTreeMap::<core::package_id::PackageId, alloc::collections::btree::set_val::SetValZST>::first_key_value |
| SoN construction: unsupported in bb16: representation-encoded aggregate constant requires semantic constant decoding | 4 | 2 | cargo@0.95.0 / <std::io::BufReader<gix::gix_features::interrupt::Read<'_, gix::progress::Read<&mut dyn std::io::BufRead, gix::progress::ThroughputOnDrop<gix::progres |
| SoN construction: unsupported in bb8: representation-encoded aggregate constant requires semantic constant decoding | 4 | 4 | cargo@0.95.0 / std::io::impls::<impl std::io::Read for &[u8]>::read_buf_exact |
| SoN construction: unsupported in bb5: representation-encoded aggregate constant requires semantic constant decoding | 4 | 4 | cargo@0.95.0 / std::io::impls::<impl std::io::Read for &[u8]>::read_to_string |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::i16x16#3f98e27a0b3e2709-4f05efbb9d687a9a | 4 | 1 | faster-hex@0.10.0 / std::arch::x86_64::_mm256_srai_epi16::<6> |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<str>` | 4 | 4 | jiff@0.2.17 / <std::sync::Weak<str, &std::alloc::Global> as core::ops::Drop>::drop |
| layout lowering: malformed layout input: Phi field shape mismatch | 4 | 2 | orion@0.17.11 / hazardous::kem::ml_kem::internal::KeyPairInternal::<hazardous::kem::ml_kem::internal::MlKem512Internal>::keygen::<2, 800, 1632> |
| MIR conversion: LLVM intrinsic `llvm.x86.sha1rnds4` | 4 | 1 | sha1@0.10.6 / core::arch::x86_64::_mm_sha1rnds4_epu32::<0> |
| MIR conversion: LLVM intrinsic `llvm.x86.xgetbv` | 3 | 3 | faster-hex@0.10.0 / std::arch::x86_64::_xgetbv |
| SoN construction: unsupported: SIMD value is outside the semantic path: raw::inner::sse2::Group#799b310d19595db-b1f1603e4cee533b | 3 | 1 | hashbrown@0.14.5 / raw::inner::sse2::Group::match_empty |
| MIR conversion: intrinsic `size_of_val` on unsized `std::ffi::OsStr` | 3 | 1 | jiff@0.2.17 / std::boxed::Box::<std::ffi::OsStr>::clone_from_ref_in |
| MIR conversion: intrinsic `atomic_and` is not implemented | 3 | 2 | parking_lot@0.12.5 / std::sync::atomic::AtomicUsize::fetch_and |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::i16x8#557e21d58c42a0f6-71bbf043478df631 | 3 | 2 | rand_chacha@0.9.0 / std::arch::x86_64::_mm_shufflehi_epi16::<177> |
| SoN construction: unsupported: union value is outside the semantic path: hashlink::linked_hash_map::Links<std::sync::Arc<str>, raw_statement::RawStatement>#29f16e734087136a-eaa27a5550c85768 | 3 | 1 | rusqlite@0.38.0 / hashlink::linked_hash_map::allocate_node::<std::sync::Arc<str>, raw_statement::RawStatement> |
| SoN construction: unsupported in bb2: unsized field requires a producer-proven static slice/string tail | 3 | 1 | tar@0.4.44 / archive::EntriesFields::<'_>::next_entry_raw |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::u16x16#6bdca214a316638c-202837c97954ac3 | 2 | 2 | aho-corasick@1.1.4 / core::arch::x86_64::_mm256_srli_epi16::<4> |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<dyn util::prefilter::PrefilterI>` | 2 | 2 | aho-corasick@1.1.4 / <alloc::sync::Weak<dyn util::prefilter::PrefilterI, &alloc::alloc::Global> as core::ops::Drop>::drop |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::u16x16::splat::JustOne#91bbd583e2014d34-cdd7cb045a9982b2 | 2 | 2 | aho-corasick@1.1.4 / core::core_arch::simd::u16x16::splat |
| MIR conversion: intrinsic `size_of_val` on unsized `serde_json::value::RawValue` | 2 | 2 | cargo@0.95.0 / <std::boxed::Box<serde_json::value::RawValue> as std::ops::Drop>::drop |
| SoN construction: unsupported in bb4: representation-encoded aggregate constant requires semantic constant decoding | 2 | 2 | cargo@0.95.0 / std::io::impls::<impl std::io::Write for &mut [u8]>::write_all_vectored |
| SoN construction: unsupported in bb3: representation-encoded aggregate constant requires semantic constant decoding | 2 | 2 | cargo@0.95.0 / std::io::impls::<impl std::io::Write for &mut [u8]>::write_all |
| SoN construction: unsupported in bb0: assigned value disagrees with the semantic place type | 2 | 2 | cargo@0.95.0 / core::hash::impls::<impl std::hash::Hash for *const dyn tracing::Callsite>::hash::<std::hash::DefaultHasher> |
| MIR conversion: LLVM intrinsic `llvm.x86.avx2.pmadd.ub.sw` | 2 | 2 | faster-hex@0.10.0 / std::arch::x86_64::_mm256_maddubs_epi16 |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::u64x2::splat::JustOne#2fd7fb24172d9baa-4e46ec8cd08a6964 | 2 | 2 | faster-hex@0.10.0 / core::core_arch::simd::u64x2::splat |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::u64x4::splat::JustOne#138fa20e6921caab-964ec538308410fe | 2 | 2 | faster-hex@0.10.0 / core::core_arch::simd::u64x4::splat |
| MIR conversion: dynamic drop requires a dereferenced trait-object pointer | 2 | 1 | gix-transport@0.52.1 / std::ptr::drop_in_place::<std::cell::UnsafeCell<dyn std::ops::FnMut(gix_credentials::helper::Action) -> std::result::Result<std::option::Option<gix_cr |
| SoN construction: unsupported: SIMD value is outside the semantic path: control::group::sse2::Group#43962360bfb6ad4d-77d0cfbe08b0e5b9 | 2 | 1 | hashbrown@0.16.1 / control::group::sse2::Group::match_empty |
| SoN construction: unsupported: SIMD value is outside the semantic path: control::group::sse2::Group#96988ed59efa53b5-373d61a9558b51f9 | 2 | 1 | hashbrown@0.15.5 / control::group::sse2::Group::match_empty |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::u32x8::splat::JustOne#6db11940d6a979ef-a10621f7a86cc278 | 2 | 2 | rand_chacha@0.9.0 / core::core_arch::simd::u32x8::splat |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<[alloc::string::String]>` | 2 | 1 | regex@1.12.2 / alloc::sync::Arc::<[alloc::string::String]>::from_iter_exact::<core::iter::Cloned<core::slice::Iter<'_, alloc::string::String>>> |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<dyn subscriber::Subscriber + core::marker::Send + core::marker::Sync>` | 2 | 1 | tracing-core@0.1.36 / <alloc::sync::Weak<dyn subscriber::Subscriber + core::marker::Send + core::marker::Sync> as core::ops::Drop>::drop |
| SoN construction: unsupported in bb6: assigned value disagrees with the semantic place type | 2 | 1 | tracing-core@0.1.36 / <*const dyn subscriber::Subscriber + core::marker::Send + core::marker::Sync as core::fmt::Pointer>::fmt |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::u16x8#fc223c7741df9da1-60c99891d594a8ad | 1 | 1 | aho-corasick@1.1.4 / core::arch::x86_64::_mm_srli_epi16::<4> |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<dyn ahocorasick::AcAutomaton>` | 1 | 1 | aho-corasick@1.1.4 / <alloc::sync::Weak<dyn ahocorasick::AcAutomaton, &alloc::alloc::Global> as core::ops::Drop>::drop |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<dyn packed::teddy::builder::SearcherT>` | 1 | 1 | aho-corasick@1.1.4 / <alloc::sync::Weak<dyn packed::teddy::builder::SearcherT, &alloc::alloc::Global> as core::ops::Drop>::drop |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::u16x8::splat::JustOne#f0b8cb745c36772d-a22730049bf12b18 | 1 | 1 | aho-corasick@1.1.4 / core::core_arch::simd::u16x8::splat |
| machine lowering: malformed graph: control node n211 is unreachable | 1 | 1 | cargo@0.95.0 / alloc::collections::btree::navigate::<impl alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Immut<'_>, core::package_ |
| machine lowering: malformed graph: control node n201 is unreachable | 1 | 1 | cargo@0.95.0 / alloc::collections::btree::navigate::<impl alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Immut<'_>, core::package_ |
| MIR conversion: trait-object supertrait upcast from `&dyn std::error::Error` to `&dyn std::fmt::Display` is not supported yet | 1 | 1 | cargo@0.95.0 / error_chain::{closure#2} |
| machine lowering: malformed graph: control node n107 is unreachable | 1 | 1 | cargo@0.95.0 / std::collections::BTreeMap::<core::package_id::PackageId, alloc::collections::btree::set_val::SetValZST>::first_entry |
| MIR conversion: intrinsic `size_of_val` on unsized `std::rc::RcInner<[std::string::String]>` | 1 | 1 | cargo@0.95.0 / <std::rc::Weak<[std::string::String], &std::alloc::Global> as std::ops::Drop>::drop |
| MIR conversion: intrinsic `size_of_val` on unsized `std::rc::RcInner<str>` | 1 | 1 | cargo@0.95.0 / <std::rc::Weak<str, &std::alloc::Global> as std::ops::Drop>::drop |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<dyn core::compiler::Executor>` | 1 | 1 | cargo@0.95.0 / <std::sync::Weak<dyn core::compiler::Executor, &std::alloc::Global> as std::ops::Drop>::drop |
| semantic verification: graph verification failed: TypeMismatch { node: NodeId(47), detail: "semantic aggregate constants require value constructors" } | 1 | 1 | cargo@0.95.0 / jiff::civil::Time::with_subsec_parts_ranged::<jiff::util::rangeint::ri16<0, 999>, jiff::util::rangeint::ri16<0, 999>, jiff::util::rangeint::ri16<0, 99 |
| semantic verification: graph verification failed: TypeMismatch { node: NodeId(36), detail: "semantic aggregate constants require value constructors" } | 1 | 1 | cargo@0.95.0 / <std::option::Option<tracing_chrome::ChromeLayer<tracing_subscriber::layer::Layered<tracing_subscriber::filter::Filtered<tracing_subscriber::fmt::Laye |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<dyn clap_complete::engine::ValueCompleter>` | 1 | 1 | cargo@0.95.0 / <std::sync::Weak<dyn clap_complete::engine::ValueCompleter, &std::alloc::Global> as std::ops::Drop>::drop |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<dyn clap_complete::engine::ValueCandidates>` | 1 | 1 | cargo@0.95.0 / <std::sync::Weak<dyn clap_complete::engine::ValueCandidates, &std::alloc::Global> as std::ops::Drop>::drop |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<dyn std::any::Any + std::marker::Send + std::marker::Sync>` | 1 | 1 | clap_builder@4.5.53 / <std::sync::Weak<dyn std::any::Any + std::marker::Send + std::marker::Sync, &std::alloc::Global> as std::ops::Drop>::drop |
| static `INIT_CTOR` has an unsupported link section or import linkage | 1 | 1 | curl@0.4.49 / INIT_CTOR |
| SoN construction: unsupported: union value is outside the semantic path: any::Value#4593a8e425866e41-1d18f79556f99c2d | 1 | 1 | erased-serde@0.4.9 / <de::Variant<'_> as serde::de::VariantAccess<'_>>::unit_variant |
| MIR conversion: LLVM intrinsic `llvm.x86.avx2.packuswb` | 1 | 1 | faster-hex@0.10.0 / std::arch::x86_64::_mm256_packus_epi16 |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::i16x16::splat::JustOne#33fffc6bf83870f0-1bad64872a6eaad0 | 1 | 1 | faster-hex@0.10.0 / core::core_arch::simd::i16x16::splat |
| compiler panic in owned graph pipeline: assertion failed: payload < (1 << 30) | 1 | 1 | gix-odb@0.74.0 / store_impls::loose::write::<impl store_impls::loose::Store>::dest |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<dyn std::ops::Fn() -> std::boxed::Box<dyn gix_pack::cache::DecodeEntry + std::marker::Send> + std::marker::Send + std::marker::Sync>` | 1 | 1 | gix-odb@0.74.0 / <std::sync::Weak<dyn std::ops::Fn() -> std::boxed::Box<dyn gix_pack::cache::DecodeEntry + std::marker::Send> + std::marker::Send + std::marker::Sync,  |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<dyn std::ops::Fn() -> std::boxed::Box<dyn gix_pack::cache::Object + std::marker::Send> + std::marker::Send + std::marker::Sync>` | 1 | 1 | gix-odb@0.74.0 / <std::sync::Weak<dyn std::ops::Fn() -> std::boxed::Box<dyn gix_pack::cache::Object + std::marker::Send> + std::marker::Send + std::marker::Sync, &std: |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<std::sync::Mutex<dyn std::ops::FnMut(gix_credentials::helper::Action) -> std::result::Result<std::option::Option<gix_credentials::protocol::Outcome>, gix_credentials::protocol::Error> + std::marker::Send + std::marker::Sync>>` | 1 | 1 | gix-transport@0.52.1 / <std::sync::Weak<std::sync::Mutex<dyn std::ops::FnMut(gix_credentials::helper::Action) -> std::result::Result<std::option::Option<gix_credentials::pro |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<std::sync::Mutex<dyn std::any::Any + std::marker::Send + std::marker::Sync>>` | 1 | 1 | gix-transport@0.52.1 / <std::sync::Weak<std::sync::Mutex<dyn std::any::Any + std::marker::Send + std::marker::Sync>, &std::alloc::Global> as std::ops::Drop>::drop |
| MIR conversion: intrinsic `size_of_val` on unsized `zerovec::ZeroSlice<potential_utf::PotentialCodePoint>` | 1 | 1 | icu_collections@2.1.1 / core::mem::size_of_val::<zerovec::ZeroSlice<potential_utf::PotentialCodePoint>> |
| MIR conversion: intrinsic `size_of_val` on unsized `codepointinvliststringlist::CodePointInversionListAndStringListULE` | 1 | 1 | icu_collections@2.1.1 / core::mem::size_of_val::<codepointinvliststringlist::CodePointInversionListAndStringListULE> |
| MIR conversion: intrinsic `size_of_val` on unsized `codepointinvlist::cpinvlist::CodePointInversionListULE` | 1 | 1 | icu_collections@2.1.1 / core::mem::size_of_val::<codepointinvlist::cpinvlist::CodePointInversionListULE> |
| semantic verification: graph verification failed: TypeMismatch { node: NodeId(19), detail: "type is outside the specialized value graph subset" } | 1 | 1 | icu_properties@2.1.2 / core::option::Option::<&zerovec::ule::NichedOptionULE<icu_locale_core::subtags::Script, 4>>::copied |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<dyn for<'a, 'b> std::ops::Fn(&'a std::path::Path, &'b std::path::Path) -> std::cmp::Ordering + std::marker::Send + std::marker::Sync>` | 1 | 1 | ignore@0.4.25 / <std::sync::Weak<dyn for<'a, 'b> std::ops::Fn(&'a std::path::Path, &'b std::path::Path) -> std::cmp::Ordering + std::marker::Send + std::marker::Sync, |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<dyn for<'a, 'b> std::ops::Fn(&'a std::ffi::OsStr, &'b std::ffi::OsStr) -> std::cmp::Ordering + std::marker::Send + std::marker::Sync>` | 1 | 1 | ignore@0.4.25 / <std::sync::Weak<dyn for<'a, 'b> std::ops::Fn(&'a std::ffi::OsStr, &'b std::ffi::OsStr) -> std::cmp::Ordering + std::marker::Send + std::marker::Sync, |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<dyn for<'a> std::ops::Fn(&'a walk::DirEntry) -> bool + std::marker::Send + std::marker::Sync>` | 1 | 1 | ignore@0.4.25 / <std::sync::Weak<dyn for<'a> std::ops::Fn(&'a walk::DirEntry) -> bool + std::marker::Send + std::marker::Sync, &std::alloc::Global> as std::ops::Drop> |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<[crossbeam_deque::Stealer<walk::Message>]>` | 1 | 1 | ignore@0.4.25 / <std::sync::Weak<[crossbeam_deque::Stealer<walk::Message>], &std::alloc::Global> as std::ops::Drop>::drop |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<dyn unit::traits::DisplayValue + std::marker::Send + std::marker::Sync>` | 1 | 1 | prodash@30.0.1 / <std::sync::Weak<dyn unit::traits::DisplayValue + std::marker::Send + std::marker::Sync, &std::alloc::Global> as std::ops::Drop>::drop |
| SoN construction: unsupported: union value is outside the semantic path: ppv_lite86::vec512_storage#307b6302d07d58c1-f330ac49b4d6d621 | 1 | 1 | rand_chacha@0.9.0 / ppv_lite86::vec512_storage::split128 |
| SoN construction: unsupported: union value is outside the semantic path: ppv_lite86::vec256_storage#a6e1f7fd5af61a94-c2239fbee51ce1eb | 1 | 1 | rand_chacha@0.9.0 / ppv_lite86::x86_64::sse2::avx2::<impl ppv_lite86::Store<ppv_lite86::vec512_storage> for ppv_lite86::soft::x2<ppv_lite86::x86_64::sse2::avx2::u32x4x2_a |
| MIR conversion: LLVM intrinsic `llvm.x86.avx.vzeroupper` | 1 | 1 | rand_chacha@0.9.0 / std::arch::x86_64::_mm256_zeroupper |
| MIR conversion: SIMD intrinsic requires nonempty integer lanes, got `std::arch::x86_64::__m128` | 1 | 1 | rand_chacha@0.9.0 / std::arch::x86_64::_mm256_setr_m128i |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::u32x4::splat::JustOne#4227657c72009138-c62afb4ed03638dc | 1 | 1 | rand_chacha@0.9.0 / core::core_arch::simd::u32x4::splat |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<dyn meta::strategy::Strategy>` | 1 | 1 | regex-automata@0.4.13 / <std::sync::Weak<dyn meta::strategy::Strategy, &std::alloc::Global> as core::ops::Drop>::drop |
| MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<[u8]>` | 1 | 1 | regex-automata@0.4.13 / <std::sync::Weak<[u8], &std::alloc::Global> as core::ops::Drop>::drop |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::simd::Simd<i8, 1>#babfc436dcd0b2f3-c5ad70aee1feef5b | 1 | 1 | regex-syntax@0.8.8 / core::simd::Simd::<T, N>::splat::splat_rt::<i8, 16> |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::simd::Simd<u8, 1>#a121cd8074903a21-87cddfd4304b71c6 | 1 | 1 | regex-syntax@0.8.8 / core::simd::Simd::<T, N>::splat::splat_rt::<u8, 16> |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::simd::Simd<i8, 16>#9bf4305c7d6ea96a-95134b9d5703dc97 | 1 | 1 | regex-syntax@0.8.8 / core::str::pattern::simd_contains::{closure#3} |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::simd::Simd<i8, 64>#a299f5919a1a8597-e8a8de458c297305 | 1 | 1 | regex-syntax@0.8.8 / core::core_simd::masks::mask_impl::Mask::<i8, 16>::to_bitmask_integer |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::simd::Simd<u8, 16>#e1f034a248ac678d-7599364c536942c0 | 1 | 1 | regex-syntax@0.8.8 / core::str::pattern::simd_contains |
| MIR conversion: intrinsic `size_of_val` on unsized `smallvec::alloc::sync::ArcInner<str>` | 1 | 1 | rusqlite@0.38.0 / <std::sync::Weak<str, &std::alloc::Global> as std::ops::Drop>::drop |
| MIR conversion: intrinsic `size_of_val` on unsized `std::rc::RcInner<[u8]>` | 1 | 1 | rustfix@0.9.4 / <std::rc::Weak<[u8], &std::alloc::Global> as std::ops::Drop>::drop |
| MIR conversion: intrinsic `size_of_val` on unsized `raw::RawValue` | 1 | 1 | serde_json@1.0.148 / <alloc::boxed::Box<raw::RawValue> as core::ops::Drop>::drop |
| MIR conversion: LLVM intrinsic `llvm.x86.sha1msg1` | 1 | 1 | sha1@0.10.6 / core::arch::x86_64::_mm_sha1msg1_epu32 |
| MIR conversion: LLVM intrinsic `llvm.x86.sha1msg2` | 1 | 1 | sha1@0.10.6 / core::arch::x86_64::_mm_sha1msg2_epu32 |
| MIR conversion: LLVM intrinsic `llvm.x86.sha1nexte` | 1 | 1 | sha1@0.10.6 / core::arch::x86_64::_mm_sha1nexte_epu32 |
| MIR conversion: SIMD intrinsic requires nonempty integer lanes, got `core::arch::x86_64::__m128` | 1 | 1 | sha2@0.10.9 / core::arch::x86_64::_mm256_set_m128i |
| MIR conversion: LLVM intrinsic `llvm.x86.sha256msg1` | 1 | 1 | sha2@0.10.9 / core::arch::x86_64::_mm_sha256msg1_epu32 |
| MIR conversion: LLVM intrinsic `llvm.x86.sha256msg2` | 1 | 1 | sha2@0.10.9 / core::arch::x86_64::_mm_sha256msg2_epu32 |
| MIR conversion: LLVM intrinsic `llvm.x86.sha256rnds2` | 1 | 1 | sha2@0.10.9 / core::arch::x86_64::_mm_sha256rnds2_epu32 |
| SoN construction: unsupported in bb10: unsized field requires a producer-proven static slice/string tail | 1 | 1 | tar@0.4.44 / archive::EntriesFields::<'_>::parse_sparse_header |
| SoN construction: unsupported in bb18: unsized field requires a producer-proven static slice/string tail | 1 | 1 | tar@0.4.44 / archive::EntriesFields::<'_>::skip |
| SoN construction: unsupported in bb6: representation-encoded aggregate constant requires semantic constant decoding | 1 | 1 | tempfile@3.24.0 / <std::io::Cursor<std::vec::Vec<u8>> as std::io::Read>::read_to_string |
| semantic verification: graph verification failed: TypeMismatch { node: NodeId(27), detail: "type is outside the specialized value graph subset" } | 1 | 1 | zlib-rs@0.5.5 / <std::iter::Copied<std::slice::Iter<'_, std::arch::x86_64::__m256i>> as std::iter::Iterator>::next |
| MIR conversion: LLVM intrinsic `llvm.x86.avx2.psad.bw` | 1 | 1 | zlib-rs@0.5.5 / std::arch::x86_64::_mm256_sad_epu8 |
| MIR conversion: LLVM intrinsic `llvm.x86.avx2.pmadd.wd` | 1 | 1 | zlib-rs@0.5.5 / std::arch::x86_64::_mm256_madd_epi16 |
| MIR conversion: LLVM intrinsic `llvm.x86.avx2.permd` | 1 | 1 | zlib-rs@0.5.5 / std::arch::x86_64::_mm256_permutevar8x32_epi32 |

### MIR intrinsic调用观察（包含受更早错误遮挡的调用）

| intrinsic | 调用点数 | target units |
|---|---:|---:|
| assert_inhabited | 3048 | 98 |
| size_of_val | 2769 | 151 |
| cold_path | 2658 | 156 |
| align_of_val | 2345 | 115 |
| ptr_offset_from_unsigned | 2328 | 136 |
| arith_offset | 1589 | 104 |
| abort | 1271 | 66 |
| atomic_cxchg | 1260 | 41 |
| saturating_add | 1021 | 57 |
| copy | 855 | 66 |
| rotate_left | 731 | 37 |
| atomic_cxchgweak | 690 | 29 |
| atomic_load | 681 | 93 |
| atomic_xsub | 664 | 55 |
| select_unpredictable | 513 | 44 |
| atomic_xadd | 420 | 42 |
| cttz_nonzero | 400 | 39 |
| saturating_sub | 376 | 76 |
| atomic_store | 327 | 52 |
| atomic_xchg | 298 | 27 |
| write_bytes | 249 | 129 |
| ctlz | 229 | 67 |
| ctlz_nonzero | 227 | 51 |
| typed_swap_nonoverlapping | 222 | 48 |
| atomic_fence | 212 | 53 |
| simd_shuffle | 171 | 22 |
| compare_bytes | 170 | 113 |
| ptr_offset_from | 159 | 52 |
| is_val_statically_known | 158 | 148 |
| catch_unwind | 147 | 13 |
| simd_eq | 136 | 50 |
| simd_bitmask | 109 | 54 |
| atomic_or | 108 | 9 |
| bswap | 104 | 30 |
| raw_eq | 85 | 51 |
| black_box | 85 | 9 |
| ctpop | 79 | 38 |
| simd_or | 66 | 43 |
| cttz | 62 | 46 |
| disjoint_bitor | 56 | 44 |
| simd_lt | 48 | 43 |
| unchecked_funnel_shl | 40 | 37 |
| assert_zero_valid | 40 | 10 |
| simd_reduce_all | 32 | 16 |
| atomic_singlethreadfence | 32 | 8 |
| volatile_store | 29 | 7 |
| simd_gt | 28 | 27 |
| simd_and | 25 | 22 |
| simd_shr | 20 | 4 |
| rotate_right | 20 | 13 |
| unchecked_funnel_shr | 16 | 13 |
| simd_shl | 15 | 4 |
| atomic_and | 15 | 2 |
| simd_extract | 13 | 5 |
| simd_add | 12 | 5 |
| simd_xor | 8 | 5 |
| exact_div | 6 | 6 |
| fabsf64 | 6 | 6 |
| volatile_load | 5 | 5 |
| simd_insert | 5 | 1 |
| copysignf64 | 4 | 4 |
| truncf64 | 3 | 2 |
| powif32 | 2 | 2 |
| ceilf64 | 2 | 2 |
| roundf64 | 2 | 2 |
| simd_select | 2 | 1 |
| floorf64 | 2 | 2 |
| bitreverse | 2 | 1 |
| log2f32 | 1 | 1 |
| maxnumf64 | 1 | 1 |
| minnumf64 | 1 | 1 |
| ceilf32 | 1 | 1 |
| caller_location | 1 | 1 |
| powif64 | 1 | 1 |
| fabsf32 | 1 | 1 |
| copysignf32 | 1 | 1 |

### 只移除一种首错后的候选范围

下表只按已观察首错计算；同函数的后续缺口和crate级变换仍可能阻止编译。

| 唯一首错类别 | target units |
|---|---|
| union | gix_packetline-3f5449732ea07dc5, gix_revision-1cb14912bc31ce88 |
| SIMD | hashbrown-6cccc646fe8fb8b3, hashbrown-9fe2e99e957d6d1b, hashbrown-a88f7f3da2b31b75 |
| atomic_load | gix_actor-e0a79e379551a3b7, gix_glob-46bceff1eb1bdc14, gix_ignore-1226ba733392aae7, gix_traverse-59606b29eec0bf16, http_auth-cf045a82e9035816, libgit2_sys-ac68749fbb8eb7e0, libssh2_sys-9c1613906af3578d |
| i128-u128 | bitflags-7c70e50bdd3bf392, gix_hashtable-d800d5db67fdaae5, idna-773098ca291c2c8b, matchers-a081a6cc853ccecd, p384-a58e58727dc7a7f7, powerfmt-02f7f8f946f45634, rand_core-13b6e53c3fdc9b4f, shlex-f86302b4d1ccb548, typeid-dd9e399c4118e8c9, unicode_ident-a704f4bd3d1bfb7c, writeable-f162ef4ec7af25fe, zerovec-6099836856e61aef |
| DST-size_of_val | icu_collections-41c2d309c63376ef, rustfix-84bd69d3157d8728 |
| float-other | serde-be0a2d01995cc587, serde_untagged-e0303b738f4ef367 |
| float-ABI | ordered_float-1e32ed6b3b96226e, zerocopy-74c7503e50acf314 |
| aggregate-constant | icu_locale_core-fdd771c37eafcd74, unicode_bom-8fb63688632526df |
| assembly | constant_time_eq-c749b29ceaaa025a, terminal_size-e395799fdce6c7e4 |

## ripgrep

33/33 个target units报告完整；35422个项，1694个首错。

| 缺口 | 首错项数 | 涉及target units | 涉及package versions | 代表函数 |
|---|---:|---:|---:|---|
完整报告的Cranelift配置：speed=33 units

| SIMD | 548 | 13 | 13 | packed::teddy::generic::Teddy::<8>::verify::<core::arch::x86_64::__m128i> |
| i128-u128 | 241 | 10 | 10 | util::prefilter::Builder::ascii_case_insensitive |
| float-other | 171 | 4 | 4 | alloc::collections::btree::node::Handle::<alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Mut<'_>, std::string::String, serde_json::Value, alloc:: |
| atomic_xsub | 145 | 12 | 12 | <alloc::sync::Arc<dyn ahocorasick::AcAutomaton> as core::ops::Drop>::drop |
| union | 134 | 8 | 8 | memchr::memmem::searcher::Searcher::new::<memchr::arch::all::packedpair::DefaultFrequencyRank> |
| float-ABI | 83 | 6 | 6 | util::NiceDuration::fractional_seconds |
| unreachable-control | 71 | 4 | 4 | alloc::collections::btree::search::<impl alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Mut<'_>, alloc::boxed::Box<[u8]>, usize, alloc::collectio |
| atomic_xadd | 58 | 9 | 9 | <alloc::sync::Arc<dyn util::prefilter::PrefilterI> as core::clone::Clone>::clone |
| atomic_load | 39 | 17 | 17 | core::sync::atomic::atomic_load::<*mut ()> |
| aggregate-constant | 31 | 9 | 9 | <std::sys::stdio::unix::Stderr as std::io::Write>::write_all |
| DST-tail | 30 | 7 | 7 | alloc::sync::Arc::<dyn ahocorasick::AcAutomaton>::drop_slow |
| atomic_xchg | 25 | 6 | 6 | <std::sync::MutexGuard<'_, sync::sharded_lock::ThreadIndices> as core::ops::Drop>::drop |
| atomic_store | 19 | 10 | 10 | std::sync::atomic::atomic_store::<usize> |
| atomic_cxchg | 15 | 8 | 8 | core::sync::atomic::atomic_compare_exchange::<*mut regex_automata::dfa::dense::DFA<&[u32]>> |
| atomic_fence | 12 | 12 | 12 | core::sync::atomic::fence |
| DST-size_of_val | 11 | 3 | 3 | <alloc::sync::Weak<dyn ahocorasick::AcAutomaton, &alloc::alloc::Global> as core::ops::Drop>::drop |
| TLS | 10 | 5 | 5 | std::hash::RandomState::new::KEYS::{constant#0}::{closure#1} |
| verification-other | 9 | 2 | 2 | core::arch::x86_64::_mm256_permute2x128_si256::<33> |
| atomic_or | 8 | 2 | 2 | std::sync::atomic::AtomicUsize::fetch_or |
| LLVM-intrinsic | 7 | 4 | 4 | core::arch::x86_64::_mm256_shuffle_epi8 |
| unwind | 7 | 3 | 3 | std::thread::lifecycle::spawn_unchecked::<'_, {closure@crates/cli/src/process.rs:282:32: 282:39}, process::CommandError>::{closure#1} |
| atomic_cxchgweak | 5 | 3 | 3 | std::sync::atomic::atomic_compare_exchange_weak::<usize> |
| uninhabited-value | 5 | 3 | 3 | <std::result::Result<std::vec::in_place_drop::InPlaceDrop<regex_syntax::hir::Hir>, !> as std::ops::FromResidual<std::result::Result<std::convert::Infallible, !>>>::from_residual |
| compiler-panic | 4 | 1 | 1 | meta::strategy::ReverseSuffix::try_search_half_start |
| atomic_singlethreadfence | 2 | 2 | 2 | std::sync::atomic::compiler_fence |
| volatile-memory | 2 | 1 | 1 | std::ptr::read_volatile::<std::mem::MaybeUninit<walk::Message>> |
| C-variadic | 1 | 1 | 1 | os::MmapInner::remap |
| runtime-entry | 1 | 1 | 1 | executable runtime adapter |

每类涉及的crate与代表错误：

- **union**：aho-corasick@1.1.3, anyhow@1.0.87, bstr@1.10.0, globset@0.4.15, grep-regex@0.1.13, regex-automata@0.4.7, regex-syntax@0.8.4, ripgrep@14.1.1。代表：`SoN construction: unsupported: union value is outside the semantic path: memchr::memmem::searcher::PrefilterKind#9f269fbae2ea58ae-31918044440eb324`。
- **unreachable-control**：aho-corasick@1.1.3, regex-automata@0.4.7, ripgrep@14.1.1, serde_json@1.0.128。代表：`machine lowering: malformed graph: control node n102 is unreachable`。
- **SIMD**：aho-corasick@1.1.3, bstr@1.10.0, crossbeam-utils@0.8.20, globset@0.4.15, grep-cli@0.1.11, grep-regex@0.1.13, ignore@0.4.23, memchr@2.7.4, regex-automata@0.4.7, regex-syntax@0.8.4, ripgrep@14.1.1, serde@1.0.210, termcolor@1.4.1。代表：`SoN construction: unsupported: SIMD value is outside the semantic path: core::arch::x86_64::__m128i#67c1bd76a8cc8674-84f70887d6d2298a`。
- **atomic_load**：aho-corasick@1.1.3, bstr@1.10.0, crossbeam-epoch@0.9.18, crossbeam-utils@0.8.20, globset@0.4.15, grep-cli@0.1.11, grep-printer@0.2.2, grep-regex@0.1.13, grep-searcher@0.1.14, ignore@0.4.23, log@0.4.22, memchr@2.7.4, memmap2@0.9.4, regex-automata@0.4.7, ripgrep@14.1.1, serde_json@1.0.128, termcolor@1.4.1。代表：`MIR conversion: intrinsic `atomic_load` is not implemented`。
- **verification-other**：aho-corasick@1.1.3, memchr@2.7.4。代表：`semantic verification: graph verification failed: TypeMismatch { node: NodeId(0), detail: "type is outside the specialized value graph subset" }`。
- **i128-u128**：aho-corasick@1.1.3, anyhow@1.0.87, bstr@1.10.0, globset@0.4.15, memchr@2.7.4, regex-automata@0.4.7, ripgrep@14.1.1, ryu@1.0.18, serde@1.0.210, serde_json@1.0.128。代表：`SoN construction: unsupported: unsupported semantic type u128`。
- **DST-tail**：aho-corasick@1.1.3, bstr@1.10.0, globset@0.4.15, grep-regex@0.1.13, ignore@0.4.23, regex-automata@0.4.7, ripgrep@14.1.1。代表：`SoN construction: unsupported in bb0: unsized field requires a producer-proven static slice/string tail`。
- **atomic_xsub**：aho-corasick@1.1.3, bstr@1.10.0, crossbeam-epoch@0.9.18, crossbeam-utils@0.8.20, globset@0.4.15, grep-cli@0.1.11, grep-printer@0.2.2, grep-regex@0.1.13, ignore@0.4.23, regex-automata@0.4.7, ripgrep@14.1.1, walkdir@2.5.0。代表：`MIR conversion: intrinsic `atomic_xsub` is not implemented`。
- **DST-size_of_val**：aho-corasick@1.1.3, ignore@0.4.23, regex-automata@0.4.7。代表：`MIR conversion: intrinsic `size_of_val` on unsized `alloc::sync::ArcInner<dyn ahocorasick::AcAutomaton>``。
- **atomic_xadd**：aho-corasick@1.1.3, crossbeam-epoch@0.9.18, crossbeam-utils@0.8.20, globset@0.4.15, grep-cli@0.1.11, grep-printer@0.2.2, ignore@0.4.23, regex-automata@0.4.7, ripgrep@14.1.1。代表：`MIR conversion: intrinsic `atomic_xadd` is not implemented`。
- **atomic_fence**：aho-corasick@1.1.3, bstr@1.10.0, crossbeam-epoch@0.9.18, crossbeam-utils@0.8.20, globset@0.4.15, grep-cli@0.1.11, grep-printer@0.2.2, grep-regex@0.1.13, ignore@0.4.23, regex-automata@0.4.7, ripgrep@14.1.1, walkdir@2.5.0。代表：`MIR conversion: intrinsic `atomic_fence` is not implemented`。
- **LLVM-intrinsic**：aho-corasick@1.1.3, ignore@0.4.23, log@0.4.22, ripgrep@14.1.1。代表：`MIR conversion: LLVM intrinsic `llvm.x86.avx2.pshuf.b``。
- **atomic_cxchg**：bstr@1.10.0, crossbeam-epoch@0.9.18, crossbeam-utils@0.8.20, globset@0.4.15, grep-cli@0.1.11, ignore@0.4.23, log@0.4.22, ripgrep@14.1.1。代表：`MIR conversion: intrinsic `atomic_cxchg` is not implemented`。
- **atomic_store**：crossbeam-epoch@0.9.18, crossbeam-utils@0.8.20, globset@0.4.15, grep-cli@0.1.11, ignore@0.4.23, log@0.4.22, memchr@2.7.4, memmap2@0.9.4, ripgrep@14.1.1, termcolor@1.4.1。代表：`MIR conversion: intrinsic `atomic_store` is not implemented`。
- **atomic_cxchgweak**：crossbeam-epoch@0.9.18, ignore@0.4.23, ripgrep@14.1.1。代表：`MIR conversion: intrinsic `atomic_cxchgweak` is not implemented`。
- **atomic_or**：crossbeam-epoch@0.9.18, ripgrep@14.1.1。代表：`MIR conversion: intrinsic `atomic_or` is not implemented`。
- **atomic_singlethreadfence**：crossbeam-epoch@0.9.18, ignore@0.4.23。代表：`MIR conversion: intrinsic `atomic_singlethreadfence` is not implemented`。
- **TLS**：crossbeam-utils@0.8.20, globset@0.4.15, ignore@0.4.23, regex-automata@0.4.7, ripgrep@14.1.1。代表：`SoN construction: unsupported: TLS is outside the semantic path`。
- **atomic_xchg**：crossbeam-utils@0.8.20, globset@0.4.15, grep-cli@0.1.11, ignore@0.4.23, ripgrep@14.1.1, termcolor@1.4.1。代表：`MIR conversion: intrinsic `atomic_xchg` is not implemented`。
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
| MIR conversion: intrinsic `atomic_xsub` is not implemented | 145 | 12 | aho-corasick@1.1.3 / <alloc::sync::Arc<dyn ahocorasick::AcAutomaton> as core::ops::Drop>::drop |
| SoN construction: unsupported: SIMD value is outside the semantic path: std::arch::x86_64::__m128i#67c1bd76a8cc8674-84f70887d6d2298a | 88 | 5 | globset@0.4.15 / hashbrown::raw::RawTableInner::resize_inner::<std::alloc::Global> |
| MIR conversion: floating-point scalar ABI is not implemented | 82 | 6 | grep-printer@0.2.2 / util::NiceDuration::fractional_seconds |
| SoN construction: unsupported: union value is outside the semantic path: std::sync::lazy_lock::Data<std::backtrace::Capture, {closure@std::backtrace::helper::lazy_resolve::{closure#0}}>#90d1a56733746958-823052a7bacf9 | 64 | 2 | anyhow@1.0.87 / error::<impl Error>::from_adhoc::<alloc::string::String> |
| MIR conversion: intrinsic `atomic_xadd` is not implemented | 58 | 9 | aho-corasick@1.1.3 / <alloc::sync::Arc<dyn util::prefilter::PrefilterI> as core::clone::Clone>::clone |
| SoN construction: unsupported: union value is outside the semantic path: memchr::memmem::searcher::SearcherKind#31d3ada2ca368391-4f0195beac66192b | 45 | 3 | aho-corasick@1.1.3 / memchr::memmem::Finder::<'_>::new::<alloc::vec::Vec<u8>> |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::i8x16#4de202288ac8fa56-9422adfea3fe85ee | 40 | 10 | aho-corasick@1.1.3 / core::arch::x86_64::_mm_alignr_epi8::<13> |
| MIR conversion: intrinsic `atomic_load` is not implemented | 39 | 17 | aho-corasick@1.1.3 / core::sync::atomic::atomic_load::<*mut ()> |
| SoN construction: unsupported in bb0: unsized field requires a producer-proven static slice/string tail | 30 | 7 | aho-corasick@1.1.3 / alloc::sync::Arc::<dyn ahocorasick::AcAutomaton>::drop_slow |
| MIR conversion: intrinsic `atomic_xchg` is not implemented | 25 | 6 | crossbeam-utils@0.8.20 / <std::sync::MutexGuard<'_, sync::sharded_lock::ThreadIndices> as core::ops::Drop>::drop |
| MIR conversion: unsupported scalar ABI ScalarAbi { bits: 128, extension: None } | 23 | 7 | aho-corasick@1.1.3 / std_detect::detect::cache::test |
| machine lowering: malformed graph: control node n124 is unreachable | 21 | 4 | aho-corasick@1.1.3 / alloc::collections::btree::navigate::<impl alloc::collections::btree::node::Handle<alloc::collections::btree::node::NodeRef<alloc::collections::btree: |
| MIR conversion: intrinsic `atomic_store` is not implemented | 19 | 10 | crossbeam-epoch@0.9.18 / std::sync::atomic::atomic_store::<usize> |
| machine lowering: malformed graph: control node n108 is unreachable | 18 | 4 | aho-corasick@1.1.3 / alloc::collections::btree::navigate::LazyLeafRange::<alloc::collections::btree::node::marker::Dying, alloc::boxed::Box<[u8]>, usize>::init_front |
| semantic verification: graph verification failed: TypeMismatch { node: NodeId(7), detail: "semantic aggregate constants require value constructors" } | 18 | 5 | grep-printer@0.2.2 / std::cell::RefCell::<std::vec::Vec<u8>>::new |
| machine lowering: malformed graph: control node n102 is unreachable | 17 | 4 | aho-corasick@1.1.3 / alloc::collections::btree::search::<impl alloc::collections::btree::node::NodeRef<alloc::collections::btree::node::marker::Mut<'_>, alloc::boxed::Box< |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::i8x32#db30f0c4957e1ff0-7946e52d8322e478 | 17 | 4 | aho-corasick@1.1.3 / core::arch::x86_64::_mm256_alignr_epi8::<13> |
| SoN construction: unsupported: union value is outside the semantic path: core::escape::MaybeEscapedCharacter<10>#9370681a58d76310-d6e95c04666c705d | 16 | 6 | anyhow@1.0.87 / <context::Quoted<&mut core::fmt::Formatter<'_>> as core::fmt::Write>::write_str |
| machine lowering: unsupported in machine lowering: 128-bit integers | 16 | 1 | regex-automata@0.4.7 / core::ptr::drop_in_place::<core::option::Option<meta::wrappers::HybridEngine>> - shim(Some(core::option::Option<meta::wrappers::HybridEngine>)) |
| MIR conversion: intrinsic `atomic_cxchg` is not implemented | 15 | 8 | bstr@1.10.0 / core::sync::atomic::atomic_compare_exchange::<*mut regex_automata::dfa::dense::DFA<&[u32]>> |
| machine lowering: malformed graph: control node n169 is unreachable | 12 | 4 | aho-corasick@1.1.3 / alloc::collections::btree::navigate::LazyLeafRange::<alloc::collections::btree::node::marker::Dying, alloc::boxed::Box<[u8]>, usize>::take_front |
| MIR conversion: intrinsic `atomic_fence` is not implemented | 12 | 12 | aho-corasick@1.1.3 / core::sync::atomic::fence |
| SoN construction: unsupported: TLS is outside the semantic path | 10 | 5 | crossbeam-utils@0.8.20 / std::hash::RandomState::new::KEYS::{constant#0}::{closure#1} |
| SoN construction: unsupported in bb9: representation-encoded aggregate constant requires semantic constant decoding | 10 | 5 | grep-cli@0.1.11 / <std::sys::stdio::unix::Stderr as std::io::Write>::write_all |
| SoN construction: unsupported: SIMD value is outside the semantic path: core::core_arch::simd::i64x4#b53314c51c8f39bb-8a3b8ce4aa781344 | 9 | 2 | aho-corasick@1.1.3 / core::arch::x86_64::_mm256_permute2f128_si256::<33> |
| MIR conversion: intrinsic `atomic_or` is not implemented | 8 | 2 | crossbeam-epoch@0.9.18 / std::sync::atomic::AtomicUsize::fetch_or |
| MIR conversion: intrinsic `catch_unwind` is not implemented | 7 | 3 | grep-cli@0.1.11 / std::thread::lifecycle::spawn_unchecked::<'_, {closure@crates/cli/src/process.rs:282:32: 282:39}, process::CommandError>::{closure#1} |
| SoN construction: unsupported: union value is outside the semantic path: memchr::memmem::searcher::PrefilterKind#9f269fbae2ea58ae-31918044440eb324 | 6 | 3 | aho-corasick@1.1.3 / memchr::memmem::searcher::Searcher::new::<memchr::arch::all::packedpair::DefaultFrequencyRank> |
| semantic verification: graph verification failed: TypeMismatch { node: NodeId(4), detail: "type is outside the specialized value graph subset" } | 6 | 1 | memchr@2.7.4 / arch::generic::memchr::One::<core::arch::x86_64::__m128i>::new |
| MIR conversion: intrinsic `atomic_cxchgweak` is not implemented | 5 | 3 | crossbeam-epoch@0.9.18 / std::sync::atomic::atomic_compare_exchange_weak::<usize> |
| MIR conversion: LLVM intrinsic `llvm.x86.sse2.pause` | 5 | 3 | ignore@0.4.23 / std::sync::Arc::<dir::IgnoreInner>::downgrade |
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
| MIR conversion: intrinsic `atomic_singlethreadfence` is not implemented | 2 | 2 | crossbeam-epoch@0.9.18 / std::sync::atomic::compiler_fence |
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
