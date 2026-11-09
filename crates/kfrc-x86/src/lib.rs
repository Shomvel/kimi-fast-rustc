//! x86-64 back end for verified Layout graphs.
//!
//! The graph is scheduled here (blocks in reverse postorder, pure nodes at
//! their earliest dominating block), lowered to the existing three-address
//! `VirtualInstr` machine IR, register-allocated and encoded by the existing
//! linear-scan allocator and encoder (`crate::machine::allocate`,
//! `machine::x86_encode`), and written as an ELF relocatable with the `object` crate.
//!
//! Scalars narrower than 64 bits live in 64-bit registers in canonical form:
//! sign-extended for signed types, zero-extended otherwise. Arguments and call
//! results are canonicalized on arrival because the SysV/Rust ABIs leave the
//! upper bits unspecified.
//!
//! Thread-local addresses use the Linux x86-64 local-exec ELF model: read
//! `FS:0`, then add the linker's `R_X86_64_TPOFF32` offset with LEA. TLS templates
//! reside in `.tdata`/`.tbss` with `STT_TLS` symbols. Objects and rlibs may be
//! linked into executables (including PIE); linking their TLS access code into
//! a shared library is unsupported and rejected by the linker. Initialization,
//! destructor registration and teardown remain in the real Rust/std runtime;
//! the machine backend only obtains the current thread's storage address.
//!
//! An explicitly classified 16-byte vector ABI value uses XMM0–XMM7 for
//! parameters and XMM0 for returns. GPR allocation advances independently;
//! exhausted vector arguments occupy sixteen-byte-aligned stack slots. The
//! graph keeps vector values in addressable storage and the backend uses
//! MOVDQU transfers at ABI boundaries. Rustc's indirect SIMD ABI remains
//! indirect; vector layout alone never changes its recorded passing mode.

pub mod machine;

mod analysis;
mod lower;
mod regalloc;
mod object_file;

pub use lower::{LowerError, lower_function, lower_function_with_options};
pub use object_file::{
    FunctionCode, ObjectError, SymbolBinding, write_object, write_object_with_data,
};

pub use crate::machine::x86::{MachineFunction, MachineInstr, Reg, StackSlot};
