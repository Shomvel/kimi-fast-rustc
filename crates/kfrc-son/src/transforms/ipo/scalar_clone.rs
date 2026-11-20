//! A same-ABI specialization whose caller has already proved a literal argument.
use crate::{FunctionGraph, GraphLevel, Opcode, PipelineError};
use kfrc_body::{PassMode, TypeOracle, TypeShape};

/// Clone a specialized body with one valid scalar argument fixed. The result
/// still has the original entry types and ABI. A producer may only call it
/// with the exact literal, and must still observe all original arguments.
pub fn specialize_literal_argument(
    source: &FunctionGraph,
    parameter: usize,
    shape: &TypeShape,
    bits: u128,
) -> Result<Option<FunctionGraph>, PipelineError> {
    let valid = match shape {
        TypeShape::Char => u32::try_from(bits).ok().and_then(char::from_u32).is_some(),
        TypeShape::Bool => bits <= 1,
        TypeShape::Int { width, .. } => {
            let width = width.bits(64);
            width == 128 || bits >> width == 0
        }
        _ => false,
    };
    if source.level != GraphLevel::Specialized
        || !matches!(
            source.abi.args.get(parameter),
            Some(PassMode::Direct | PassMode::DirectScalar(_))
        )
        || !valid
    {
        return Ok(None);
    }

    let replacements: Vec<_> = source
        .ids()
        .filter_map(|node| {
            if source.op(node) != Opcode::Projection
                || source.resolve(source.input(node, 0)) != source.start
                || source.payload(node).checked_sub(2).map(|arg| arg as usize) != Some(parameter)
            {
                return None;
            }
            let ty = source.rust_type(node)?;
            (source.oracle.shape(ty) == shape).then_some((node, ty))
        })
        .collect();
    if replacements.is_empty() {
        return Ok(None);
    }

    let mut graph = source.clone();
    for (node, ty) in replacements {
        let constant = graph.add_constant(bits, ty);
        graph.replace(node, constant);
    }
    graph.resolve_edges();
    crate::optimize_specialized_graph(&mut graph)?;
    graph.compact();
    crate::verify::verify_pipeline_step(&mut graph).map_err(PipelineError::Verification)?;
    Ok(Some(graph))
}
