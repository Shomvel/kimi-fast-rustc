//! Machine-layer selection and the hand-over of encoded functions.
//!
//! Every function leaves this module as a `kfrc_body::EncodedFunction`, the
//! record the object writer accepts. `KFRC_MACHINE` selects the layer that
//! produces it: `cranelift` (the Cranelift exporter) or `x86` (the original
//! selector, kept as the differential baseline).

use std::sync::OnceLock;

use kfrc_body::{DataObject, EncodedFunction};

use crate::profile;

/// One emitted function and the data objects it owns.
pub(crate) type CompiledFunction = (EncodedFunction, Vec<DataObject>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MachineLayer {
    Cranelift,
    X86,
}

/// The layer selected for this process, or the reason the request is invalid.
pub(crate) fn layer() -> Result<MachineLayer, String> {
    static LAYER: OnceLock<Result<MachineLayer, String>> = OnceLock::new();
    LAYER
        .get_or_init(|| match std::env::var("KFRC_MACHINE").as_deref() {
            Err(std::env::VarError::NotPresent) | Ok("cranelift") => Ok(MachineLayer::Cranelift),
            Ok("x86") => Ok(MachineLayer::X86),
            Ok(other) => Err(format!(
                "KFRC_MACHINE=`{other}` (expected `cranelift` or `x86`)"
            )),
            Err(error) => Err(format!("KFRC_MACHINE: {error}")),
        })
        .clone()
}

fn cranelift() -> Result<&'static kfrc_cranelift::Machine, String> {
    static MACHINE: OnceLock<Result<kfrc_cranelift::Machine, String>> = OnceLock::new();
    MACHINE
        .get_or_init(|| {
            let optimization = match std::env::var("KFRC_CRANELIFT_OPT").as_deref() {
                Err(std::env::VarError::NotPresent) | Ok("speed") => {
                    kfrc_cranelift::Optimization::Speed
                }
                Ok("none") => kfrc_cranelift::Optimization::None,
                Ok("speed_and_size") => kfrc_cranelift::Optimization::SpeedAndSize,
                Ok(other) => {
                    return Err(format!(
                        "KFRC_CRANELIFT_OPT=`{other}` (expected `none`, `speed` or `speed_and_size`)"
                    ));
                }
                Err(error) => return Err(format!("KFRC_CRANELIFT_OPT: {error}")),
            };
            let verify = std::env::var("KFRC_CRANELIFT_VERIFY").as_deref() == Ok("1");
            let enabled = |name: &str| std::env::var(name).as_deref() != Ok("0");
            kfrc_cranelift::Machine::new(kfrc_cranelift::MachineOptions {
                optimization,
                verify,
                promote_storage: enabled("KFRC_CRANELIFT_PROMOTE"),
                split_after_calls: enabled("KFRC_CRANELIFT_SPLIT_CALLS"),
                report_storage: std::env::var("KFRC_CRANELIFT_STORAGE_REPORT").as_deref()
                    == Ok("1"),
            })
            .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// Lowers one verified Layout graph with the selected machine layer.
pub(crate) fn lower(graph: &kfrc_son::FunctionGraph) -> Result<EncodedFunction, String> {
    match layer()? {
        MachineLayer::Cranelift => {
            let machine = cranelift()?;
            profile::measure("cranelift", &graph.symbol, || {
                kfrc_cranelift::lower_function(graph, machine)
            })
            .map_err(|error| error.to_string())
        }
        MachineLayer::X86 => profile::measure("x86", &graph.symbol, || {
            kfrc_x86::lower_function(graph)
                .map_err(|error| error.to_string())
                .and_then(|function| encode(&function))
        }),
    }
}

/// Encodes a hand-assembled runtime shim.
pub(crate) fn encode(function: &kfrc_x86::MachineFunction) -> Result<EncodedFunction, String> {
    kfrc_x86::encode_function(function)
        .map_err(|error| format!("encoding `{}` failed: {error:?}", function.name))
}
