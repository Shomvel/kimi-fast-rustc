# Native code-generation research

The x86_64 System V backend uses order-preserving instruction selection and linear-scan register allocation. It does not run a static ILP list scheduler. Out-of-order execution can hide many independent latencies dynamically, but it is not a proof that arbitrary static order is optimal; dependency chains, spills, register pressure, calls, and memory access still require measurement and optimization.

Live intervals use half-open instruction ranges. The allocator expires old intervals, reuses registers and spill slots deterministically, honors fixed-register constraints, and spills the latest-ending eligible interval under pressure. The initial whole-interval representation is intentionally simple. General CFG support will need accurate liveness, interval holes, fixed intervals for calls, edge resolution, and parallel-copy handling before it can be considered complete.

`rax` is reserved for return values and `r10`/`r11` for scratch. The remaining allocatable set includes caller-saved and callee-saved registers. Every used callee-saved register is preserved in the frame. Incoming argument registers are homed when later clobbers require it. Stack slots and saved-register areas are distinct, and the frame maintains System V alignment.

The backend currently emits GNU Intel-syntax assembly and uses system `as`/`gcc`, not LLVM. A direct ELF writer is implemented for relocatable objects; follow-up work covers additional relocations and broader ABI support. Unsupported ABI features such as aggregates, variadics, and unwinding must be rejected explicitly.

References:

- Poletto and Sarkar, [Linear Scan Register Allocation](https://web.cs.ucla.edu/~palsberg/course/cs132/linearscan.pdf).
- Wimmer and Mössenböck, [Optimized Interval Splitting in a Linear Scan Register Allocator](https://www.usenix.org/events/vee05/full_papers/p132-wimmer.pdf).
- [System V AMD64 psABI](https://gitlab.com/x86-psABIs/x86-64-ABI).
- Intel, [64 and IA-32 Optimization Reference Manual](https://cdrdv2-public.intel.com/814198/248966-Optimization-Reference-Manual-V1-049.pdf).
