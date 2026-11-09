# Validation evidence

Small acceptance records, source-file fingerprints and measured summaries are
versioned under `poc-semantic-only-20261109/`. Each candidate refers to a frozen
source archive by SHA-256; later status-document edits do not rewrite that input.

Raw source/result archives, binaries, disassemblies, test logs and profiles stay
in the local evidence directory and are ignored by Git. `artifact-hashes.json`
records collected archive identities. They are not bundled with a fresh clone;
use the checked-in validation runners and recorded toolchain/input identities to
reproduce a run. Historical failures and unsupported scope remain in the receipts.

Candidate13 is the ObserveValue / Refine migration acceptance input. Its archive
SHA-256 is `5316798289d820c4aa9443043197095f592bcc4e4791544b5e894511071067ca`.
Production sources in the implementation commit match its per-file manifest;
subsequent changes are packaging and documentation only. Passing correctness
and structural gates does not mean LLVM performance parity.
