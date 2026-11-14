# base64 unit-test TLS support boundary

The final integrated backend compiles the original base64 library and passes its corpus/checksums plus all 13 upstream integration tests. Its unit-test build advances beyond c12-phi's caller-location adapter rejection and reaches:

```
std::hash::RandomState::new::KEYS::{constant#0}::{closure#1}
SoN construction: unsupported: TLS is outside the semantic path
```

Static evidence in frozen final1 source:

- Original base64 0.22.1 `src/engine/mod.rs:19` gates `mod tests` under `#[cfg(test)]`.
- Its `src/engine/tests.rs:420` and `:476` instantiate `collections::HashMap::new()`, selecting RandomState. These test functions exercise decode-invalid-last-symbol and lookup checks.
- `rustc_codegen_kfrc/src/convert/rvalues.rs:117` maps rustc MIR `ThreadLocalRef` explicitly to body `Rvalue::ThreadLocalRef`, validating that the referenced def is actually TLS.
- `kfrc-son/src/build.rs:176` explicitly rejects any body containing that Rvalue before semantic type/ABI construction. `build/semantic/values.rs:228` retains the same unsupported boundary.

This is a direct, deliberate TLS support rejection in the test-only HashMap path; the failing symbol is RandomState's actual TLS initializer. No caller-location adapter routing appears in the failing operation. The adapter fix allowed compilation to reach the next unsupported operation. This is recorded as a newly exposed test-support gap and `incomplete`, not a passed unit suite, u128 failure, or measured library miscompilation. TLS implementation is outside this task's scope. The failed unit build was not repeated.
