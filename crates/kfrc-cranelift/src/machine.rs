//! The target description shared by every function of one compilation.

use std::sync::Arc;

use cranelift_codegen::isa::{self, TargetIsa};
use cranelift_codegen::settings::{self, Configurable};

#[cfg(test)]
mod narrow_load_tests;

#[cfg(test)]
mod cold_call_tests;

/// How much of Cranelift's mid-end runs before instruction selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Optimization {
    /// Instruction selection and register allocation only.
    None,
    /// Value numbering, loop-invariant code motion, rewrite rules and
    /// redundant-load elimination.
    #[default]
    Speed,
    /// `Speed`, preferring the shorter encoding where rules offer a choice.
    SpeedAndSize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MachineOptions {
    pub optimization: Optimization,
    /// Run Cranelift's IR verifier on every exported function.
    pub verify: bool,
    /// Represent private frame storage as SSA variables.
    pub promote_storage: bool,
    /// End the Cranelift block after every call, so that a constant used on
    /// both sides of the call is produced again behind it.
    pub split_after_calls: bool,
    /// Print one line per function describing which frame storage became
    /// variables and why the rest kept its memory.
    pub report_storage: bool,
}

impl Default for MachineOptions {
    fn default() -> Self {
        Self {
            optimization: Optimization::default(),
            verify: false,
            promote_storage: true,
            split_after_calls: true,
            report_storage: false,
        }
    }
}

#[derive(Debug)]
pub struct MachineError(pub String);

impl std::fmt::Display for MachineError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Cranelift target setup failed: {}", self.0)
    }
}

impl std::error::Error for MachineError {}

/// x86-64 System V ELF at the baseline feature level (SSE2), the same level
/// rustc assumes for `x86_64-unknown-linux-gnu`. The description is immutable
/// and shared across compilation threads.
#[derive(Clone)]
pub struct Machine {
    pub(crate) isa: Arc<dyn TargetIsa>,
    pub(crate) verify: bool,
    pub(crate) options: MachineOptions,
}

impl Machine {
    pub fn new(options: MachineOptions) -> Result<Self, MachineError> {
        let mut flags = settings::builder();
        let mut set = |name: &str, value: &str| {
            flags
                .set(name, value)
                .map_err(|error| MachineError(format!("{name}={value}: {error}")))
        };
        set(
            "opt_level",
            match options.optimization {
                Optimization::None => "none",
                Optimization::Speed => "speed",
                Optimization::SpeedAndSize => "speed_and_size",
            },
        )?;
        // Every symbol is reached by a RIP-relative `lea` or a rel32 call.
        // The linker binds preemptible functions through the PLT, so the
        // code is position-independent without GOT loads.
        set("is_pic", "false")?;
        set("use_colocated_libcalls", "true")?;
        // The canonical general-dynamic sequence; the linker relaxes it to
        // local-exec when it produces an executable.
        set("tls_model", "elf_gd")?;
        set("enable_llvm_abi_extensions", "true")?;
        set("unwind_info", "false")?;
        set("preserve_frame_pointers", "false")?;
        set("enable_probestack", "true")?;
        set("probestack_strategy", "inline")?;
        set("enable_verifier", "false")?;
        set(
            "regalloc_checker",
            if options.verify { "true" } else { "false" },
        )?;
        set("enable_heap_access_spectre_mitigation", "false")?;
        set("enable_table_access_spectre_mitigation", "false")?;
        set("log2_min_function_alignment", "4")?;
        let triple = "x86_64-unknown-linux-gnu"
            .parse::<target_lexicon::Triple>()
            .map_err(|error| MachineError(error.to_string()))?;
        let isa = isa::lookup(triple)
            .map_err(|error| MachineError(error.to_string()))?
            .finish(settings::Flags::new(flags))
            .map_err(|error| MachineError(error.to_string()))?;
        Ok(Self {
            isa,
            verify: options.verify,
            options,
        })
    }
}
