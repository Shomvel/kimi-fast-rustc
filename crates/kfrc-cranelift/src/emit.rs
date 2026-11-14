//! Cranelift compilation and the hand-over to the object writer's contract.

use cranelift_codegen::binemit::Reloc;
use cranelift_codegen::control::ControlPlane;
use cranelift_codegen::ir::{ExternalName, LibCall};
use cranelift_codegen::{Context, FinalizedRelocTarget};
use kfrc_body::{CodeRelocation, CodeRelocationKind, EncodedFunction};

use crate::export::LowerError;
use crate::machine::Machine;

pub(crate) fn encode(
    symbol: &str,
    context: &mut Context,
    symbols: &[String],
    machine: &Machine,
) -> Result<EncodedFunction, LowerError> {
    context
        .compile(&*machine.isa, &mut ControlPlane::default())
        .map_err(|error| LowerError::Codegen(format!("{:?}", error.inner)))?;
    let compiled = context
        .compiled_code()
        .expect("a successful compilation retains its code");
    let bytes = compiled.code_buffer().to_vec();
    let names = context.func.params.user_named_funcs();
    let mut relocations = Vec::with_capacity(compiled.buffer.relocs().len());
    for relocation in compiled.buffer.relocs() {
        let target = match &relocation.target {
            FinalizedRelocTarget::ExternalName(ExternalName::User(reference)) => {
                symbols[names[*reference].index as usize].clone()
            }
            FinalizedRelocTarget::ExternalName(ExternalName::LibCall(libcall)) => {
                libcall_symbol(*libcall)?.to_owned()
            }
            other => {
                return Err(LowerError::Unsupported(format!(
                    "relocation target {other:?}"
                )));
            }
        };
        let offset = relocation.offset as usize;
        let kind = match relocation.kind {
            // Cranelift names both a rel32 branch and a RIP-relative `lea` of
            // a nearby symbol with this relocation. A branch field directly
            // follows its one-byte opcode; a `lea` field follows a ModRM byte
            // of the form 00.reg.101, which is never a branch opcode.
            Reloc::X86CallPCRel4 => {
                if offset > 0 && matches!(bytes[offset - 1], 0xe8 | 0xe9) {
                    CodeRelocationKind::Call
                } else {
                    CodeRelocationKind::PcRelative
                }
            }
            Reloc::X86CallPLTRel4 => CodeRelocationKind::Call,
            Reloc::X86PCRel4 => CodeRelocationKind::PcRelative,
            Reloc::X86GOTPCRel4 => CodeRelocationKind::GotPcRelative,
            Reloc::Abs8 => CodeRelocationKind::Absolute64,
            Reloc::ElfX86_64TlsGd => CodeRelocationKind::TlsGeneralDynamic,
            other => {
                return Err(LowerError::Unsupported(format!("relocation kind {other:?}")));
            }
        };
        relocations.push(CodeRelocation {
            offset: offset as u64,
            target,
            addend: relocation.addend,
            kind,
        });
    }
    Ok(EncodedFunction {
        symbol: symbol.to_owned(),
        bytes,
        align: u64::from(compiled.buffer.alignment.max(1)),
        relocations,
    })
}

/// Runtime routines Cranelift itself may call. Each is the C symbol the
/// platform linker resolves.
fn libcall_symbol(libcall: LibCall) -> Result<&'static str, LowerError> {
    Ok(match libcall {
        LibCall::ElfTlsGetAddr => "__tls_get_addr",
        LibCall::Memcpy => "memcpy",
        LibCall::Memset => "memset",
        LibCall::Memmove => "memmove",
        LibCall::Memcmp => "memcmp",
        other => {
            return Err(LowerError::Unsupported(format!(
                "runtime routine {other:?}"
            )));
        }
    })
}
