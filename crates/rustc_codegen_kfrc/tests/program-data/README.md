# Crate-owned definitions and real linking

On the supported Linux toolchain, run `bash run.sh /absolute/backend.so /fresh/results`.
`KFRC_TOOLCHAIN` and optional `KFRC_SYSROOT` select the same pinned compiler/std as other shell E2Es.

The runner builds one shared dependency and two independent rlibs instantiating
its generic definition, then links an LLVM driver. LLVM, KFRC with one CGU and
KFRC with sixteen CGUs must produce the same checksum for 4096 inputs. It also
checks named-static identity, cyclic named relocations, process-wide mutable
state, function pointers, nested anonymous data and vtables.

The saved ELF symbol tables must show one definition per function in each rlib,
and at most one canonical constant per using object. Hidden globals and every
compiler/link/run command are recorded in `summary.json` and per-command files.
An optional third argument `record` records duplicate functions for historical
baselines while retaining the runtime differential. No LTO or weak-definition
workaround is used to make the three rlibs link.

The semantic frontend currently rejects `ThreadLocalRef` explicitly. TLS code
is retained under `--cfg=program_tls` for an independent rejection/frontier
probe; the normal passing runner does not claim runtime TLS coverage. Synthetic
pool tests separately check that TLS definitions retain identity and ownership.
