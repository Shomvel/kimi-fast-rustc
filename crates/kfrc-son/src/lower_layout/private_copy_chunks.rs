//! Recover contiguous bytes of one scalar snapshot without new source loads.
//!
//! Little-endian x86 supports unaligned scalar stores. Each chunk stays within
//! the original copy range; unrelated or noncontiguous bytes stay byte stores.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Byte {
    source: NodeId,
    bit: u8,
}

pub(super) struct Chunk {
    pub offset: usize,
    pub value: Forward,
}

fn source(graph: &FunctionGraph, forward: Forward) -> Option<Byte> {
    let mut source = graph.resolve(forward.value);
    let mut bit = forward.shift;
    for _ in 0..16 {
        let width = u8::try_from(integer(graph, graph.rust_type(source)?)? * 8).ok()?;
        if u16::from(bit) + 8 > u16::from(width) {
            return None;
        }
        if !graph.control(source).is_none() || graph.payload(source) != 0 {
            return Some(Byte { source, bit });
        }
        match graph.op(source) {
            Opcode::IntCast => {
                let input = graph.resolve(graph.input(source, 0));
                let bits = integer(graph, graph.rust_type(input)?)? * 8;
                if u64::from(bit) + 8 > bits {
                    break;
                }
                source = input;
            }
            Opcode::ShiftRightLogical => {
                let input = graph.resolve(graph.input(source, 0));
                let amount = graph.resolve(graph.input(source, 1));
                if graph.op(amount) != Opcode::Const {
                    break;
                }
                let shift = (graph.constant_value(amount) % u128::from(width)) as u8;
                if u16::from(bit) + 8 + u16::from(shift) > u16::from(width) {
                    break;
                }
                bit += shift;
                source = input;
            }
            _ => break,
        }
    }
    Some(Byte { source, bit })
}

pub(super) fn collect(graph: &mut FunctionGraph, values: &[Forward]) -> Vec<Chunk> {
    let origins: Vec<_> = values.iter().map(|&value| source(graph, value)).collect();
    let mut chunks = Vec::new();
    let mut offset = 0;
    while offset < values.len() {
        let bytes = [8, 4, 2]
            .into_iter()
            .find(|&bytes| {
                offset + bytes <= values.len()
                    && origins[offset].is_some_and(|first| {
                        origins[offset..offset + bytes]
                            .iter()
                            .enumerate()
                            .all(|(index, origin)| {
                                *origin
                                    == Some(Byte {
                                        source: first.source,
                                        bit: first.bit + index as u8 * 8,
                                    })
                            })
                    })
            })
            .unwrap_or(1);
        let value = if bytes == 1 {
            values[offset]
        } else {
            let origin = origins[offset].expect("matched contiguous bytes");
            let width = match bytes {
                2 => IntegerWidth::W16,
                4 => IntegerWidth::W32,
                8 => IntegerWidth::W64,
                _ => unreachable!("scalar chunk width"),
            };
            let ty = graph.oracle.intern(
                TypeShape::Int {
                    width,
                    signed: false,
                },
                Some(SizeAlign {
                    size: bytes as u64,
                    align: bytes as u64,
                }),
            );
            // Recorded nominal integer validity may be narrower than its bits.
            // Reuse the original byte path if this type is not an ordinary int.
            if integer(graph, ty) != Some(bytes as u64) {
                chunks.push(Chunk {
                    offset,
                    value: values[offset],
                });
                offset += 1;
                continue;
            }
            Forward {
                value: origin.source,
                shift: origin.bit,
                ty,
            }
        };
        chunks.push(Chunk { offset, value });
        offset += bytes;
    }
    chunks
}
