//! Encoded functions from this selector's `MachineFunction`s, and the object
//! entry points that accept them directly.

use kfrc_body::{CodeRelocation, CodeRelocationKind, DataObject, EncodedFunction};
use kfrc_object::ObjectFunction;
pub use kfrc_object::SymbolBinding;

use crate::machine::x86::MachineFunction;
use crate::machine::x86_encode::{
    AddressRelocationKind, EncodeError, encode_with_all_relocations,
};

// Function placement is independent of the encoder's relative loop alignment.
// A multiple of the loop alignment keeps those relative headers aligned after
// linking without inserting more padding into every loop body.
const FUNCTION_ALIGNMENT: u64 = 32;

pub struct FunctionCode {
    pub function: MachineFunction,
    pub binding: SymbolBinding,
}

#[derive(Debug)]
pub enum ObjectError {
    Encode { symbol: String, error: EncodeError },
    Object(kfrc_object::ObjectError),
}

impl std::fmt::Display for ObjectError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Encode { symbol, error } => {
                write!(formatter, "encoding `{symbol}` failed: {error:?}")
            }
            Self::Object(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ObjectError {}

/// Final bytes and symbolic relocations of one machine function: calls first,
/// then address references, each in instruction order.
pub fn encode_function(function: &MachineFunction) -> Result<EncodedFunction, EncodeError> {
    let (bytes, calls, addresses) = encode_with_all_relocations(function)?;
    let mut relocations = Vec::with_capacity(calls.len() + addresses.len());
    for call in calls {
        relocations.push(CodeRelocation {
            offset: call.offset,
            target: call.target,
            addend: -4,
            kind: CodeRelocationKind::Call,
        });
    }
    for address in addresses {
        let (kind, addend) = match address.kind {
            AddressRelocationKind::PcRelative => {
                (CodeRelocationKind::PcRelative, address.addend - 4)
            }
            AddressRelocationKind::TlsLocalExec => {
                (CodeRelocationKind::TlsLocalExec, address.addend)
            }
        };
        relocations.push(CodeRelocation {
            offset: address.offset,
            target: address.target,
            addend,
            kind,
        });
    }
    Ok(EncodedFunction {
        symbol: function.name.clone(),
        bytes,
        align: FUNCTION_ALIGNMENT,
        relocations,
    })
}

/// Writes independently collectable function sections with `R_X86_64_PLT32`
/// relocations for calls. Symbols retain identity; sections are not mergeable.
pub fn write_object(functions: &[FunctionCode]) -> Result<Vec<u8>, ObjectError> {
    write_object_with_data(functions, &[])
}

pub fn write_object_with_data(
    functions: &[FunctionCode],
    data: &[DataObject],
) -> Result<Vec<u8>, ObjectError> {
    let encoded = functions
        .iter()
        .map(|code| {
            Ok(ObjectFunction {
                function: encode_function(&code.function).map_err(|error| {
                    ObjectError::Encode {
                        symbol: code.function.name.clone(),
                        error,
                    }
                })?,
                binding: code.binding,
            })
        })
        .collect::<Result<Vec<_>, ObjectError>>()?;
    kfrc_object::write_object_with_data(&encoded, data).map_err(ObjectError::Object)
}
