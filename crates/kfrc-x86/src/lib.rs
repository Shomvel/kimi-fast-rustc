//! x86-64 back end for verified R/S-level scalar graphs (PoC P0).
//!
//! The graph is scheduled here (blocks in reverse postorder, pure nodes at
//! their earliest dominating block), lowered to the existing three-address
//! `VirtualInstr` machine IR, register-allocated and encoded by the existing
//! linear-scan allocator and encoder (`kimi_fast_rustc::native`,
//! `x86_encode`), and written as an ELF relocatable with the `object` crate.
//!
//! Scalars narrower than 64 bits live in 64-bit registers in canonical form:
//! sign-extended for signed types, zero-extended otherwise. Arguments and call
//! results are canonicalized on arrival because the SysV/Rust ABIs leave the
//! upper bits unspecified.

mod lower;
mod object_file;

pub use lower::{LowerError, lower_function};
pub use object_file::{FunctionCode, ObjectError, SymbolBinding, write_object};

pub use kimi_fast_rustc::x86::MachineFunction;
