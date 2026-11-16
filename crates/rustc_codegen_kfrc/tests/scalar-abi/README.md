This differential fixture targets Rust 1.94.1 and x86-64 Linux System V.
Run `bash run.sh BACKEND NEW_OUTPUT_DIRECTORY [CONTROL_BACKEND]`.

The dependency and driver use LLVM. The library uses LLVM, candidate with
1/4 threads, the candidate's original x86 machine backend, its legacy scalar
ABI control, and optionally the
previous backend. The driver asserts every result before printing a common
checksum. It covers both directions of direct and indirect Rust/C calls,
bool and signed/unsigned 8/16/32-bit values, a transparent signed wrapper,
ScalarPair arguments/results, stack arguments and exact byte writes.
All 8/16-bit patterns and both Boolean values are exercised; 32-bit inputs
include sign/width boundaries and a deterministic sequence.

Assembly checks the required low 32 argument bits for C narrow integers and
Rust/C bool, and fills only unspecified bits of arguments/results. Linux
8/16-bit results need not define an entire return register. Bool inputs and
the low result byte remain valid 0/1; no invalid Rust value is used to test
an optimization. The shims preserve callee-saved registers and call-site
stack alignment. The ScalarPair assembly uses the pinned producer's two
lanes of `Pair { number: i16, predicate: bool }`, in that order; inspect its
LLVM IR as part of validation rather than assuming all Rust structs use it.

This is a correctness fixture, not a runtime benchmark. Preserve commands,
exit codes, stdout, source/toolchain/backend hashes and actual LLVM/CLIF/
machine assembly when evaluating changes to the call ABI.

The driver also accepts `--wrapper-extension`: it passes all signed bytes
through `repr(transparent)` to a C assembly callee that checks required sign
extension through bit 31. Run this separately on LLVM and candidate backends.
The historical aggregate lane inferred unsigned extension after erasing
producer attributes; legacy/control are not passing oracles for this strict
case. Preserve a single separately invoked legacy/control result and its real
exit code when confirming that defect; the main matrix remains comparable.
