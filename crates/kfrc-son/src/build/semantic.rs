//! Rust values retained before layout lowering.

use super::*;

mod compare;
mod maybe_uninit;
mod places;
mod values;

impl Builder<'_> {
    pub(super) fn semantic_scalar(&self, ty: TyId) -> bool {
        let types = self.graph.oracle.semantic();
        match types.shape(ty) {
            TypeShape::Bool
            | TypeShape::Char
            | TypeShape::Int { .. }
            | TypeShape::Unit
            | TypeShape::Never
            | TypeShape::FunctionItem { .. } => true,
            TypeShape::Reference { .. } | TypeShape::RawPointer { .. } => !types.is_product(ty),
            _ => false,
        }
    }

    pub(super) fn check_semantic_types(&self) -> Result<(), BuildError> {
        let types = self.body.types.semantic();
        let mut pending: Vec<_> = self.body.locals.iter().map(|local| local.ty).collect();
        let mut seen = std::collections::HashSet::new();
        while let Some(ty) = pending.pop() {
            if !seen.insert(ty) {
                continue;
            }
            if let Some(inner) = types.maybe_uninit_inner(ty) {
                pending.push(inner);
                continue;
            }
            if types.is_union(ty) || types.is_vector(ty) {
                return self.unsupported("union and SIMD values are outside the semantic path");
            }
            match types.shape(ty) {
                TypeShape::Bool
                | TypeShape::Char
                | TypeShape::Unit
                | TypeShape::Never
                | TypeShape::FunctionItem { .. } => {}
                TypeShape::Int { width, .. } if *width != IntegerWidth::W128 => {}
                TypeShape::Reference { .. } | TypeShape::RawPointer { .. } => {
                    if let Some(fields) = types.field_types(ty, None) {
                        pending.extend(fields);
                    }
                }
                TypeShape::Tuple(fields) => pending.extend(fields.iter().copied()),
                TypeShape::Array { element, .. } => {
                    pending.push(*element);
                }
                TypeShape::Adt { .. } if types.is_enum(ty) => {
                    let count = types
                        .variant_count(ty)
                        .ok_or_else(|| BuildError::Unsupported {
                            block: None,
                            what: "enum has no semantic variants".into(),
                        })?;
                    for variant in 0..count {
                        pending.extend(types.field_types(ty, Some(variant as u32)).ok_or_else(
                            || BuildError::Unsupported {
                                block: None,
                                what: "enum variant has no semantic fields".into(),
                            },
                        )?);
                    }
                }
                TypeShape::Adt { .. } => {
                    pending.extend(types.field_types(ty, None).ok_or_else(|| {
                        BuildError::Unsupported {
                            block: None,
                            what: format!(
                                "no semantic fields for {}",
                                self.body.types.describe(ty)
                            ),
                        }
                    })?);
                }
                _ => {
                    return self.unsupported(format!(
                        "unsupported semantic type {}",
                        self.body.types.describe(ty)
                    ));
                }
            }
        }
        Ok(())
    }

    fn semantic_fields(&self, ty: TyId, variant: Option<u32>) -> Result<Vec<TyId>, BuildError> {
        self.graph
            .oracle
            .semantic()
            .field_types(ty, variant)
            .ok_or_else(|| BuildError::Unsupported {
                block: self.current_block,
                what: format!(
                    "missing semantic fields for {}",
                    self.graph.oracle.describe(ty)
                ),
            })
    }

    pub(super) fn semantic_field(
        &mut self,
        source: NodeId,
        ty: TyId,
        variant: Option<u32>,
        index: u32,
    ) -> Result<NodeId, BuildError> {
        let field_ty = self
            .graph
            .oracle
            .semantic()
            .field_type(ty, variant, index)
            .ok_or_else(|| BuildError::Unsupported {
                block: self.current_block,
                what: format!(
                    "invalid logical field {variant:?}/{index} of {}",
                    self.graph.oracle.describe(ty)
                ),
            })?;
        let (opcode, payload) = match variant {
            Some(variant) => (
                Opcode::VariantField,
                self.graph.intern_variant_field(variant, index),
            ),
            None => (Opcode::Field, index),
        };
        Ok(self.graph.add_node(
            opcode,
            NodeId::NONE,
            &[source],
            payload,
            ValueType::Rust(field_ty),
        ))
    }

    pub(super) fn semantic_empty_value(&mut self, ty: TyId) -> Result<NodeId, BuildError> {
        crate::semantic_value::empty_value(&mut self.graph, ty).map_err(|what| {
            BuildError::Unsupported {
                block: self.current_block,
                what,
            }
        })
    }

    pub(super) fn semantic_operand(
        &mut self,
        operand: &Operand,
        block: usize,
        control: &mut NodeId,
    ) -> Result<NodeId, BuildError> {
        match operand {
            Operand::Copy(place) => self.semantic_read_place(*place, block, control),
            Operand::Move(place) => {
                let value = self.semantic_read_place(*place, block, control)?;
                let ty = self.place_type(*place)?;
                let uninitialized = self.graph.add_node(
                    Opcode::Undefined,
                    NodeId::NONE,
                    &[],
                    0,
                    ValueType::Rust(ty),
                );
                self.semantic_write_place(*place, uninitialized, block, control)?;
                Ok(value)
            }
            Operand::Constant(id) => {
                let constant = self.body.constant(*id).clone();
                match constant.kind {
                    ConstantKind::ZeroSized | ConstantKind::Function(_) => self.semantic_empty_value(constant.ty),
                    ConstantKind::Scalar(bits) if self.semantic_scalar(constant.ty) => Ok(self.graph.add_constant(bits, constant.ty)),
                    ConstantKind::Scalar(bits) => Ok(self.graph.add_repr_constant(bits, constant.ty)),
                    ConstantKind::Address { symbol, addend } if matches!(self.graph.oracle.shape(constant.ty), TypeShape::Reference { .. } | TypeShape::RawPointer { .. }) && self.semantic_scalar(constant.ty) => {
                        let key = (symbol, addend);
                        let index = self.graph.symbols.iter().position(|existing| *existing == key).unwrap_or_else(|| {
                            self.graph.symbols.push(key);
                            self.graph.symbols.len() - 1
                        });
                        Ok(self.graph.add_node(Opcode::SymbolAddress, NodeId::NONE, &[], index as u32, ValueType::Rust(constant.ty)))
                    }
                    _ => self.unsupported("representation-encoded aggregate constant requires semantic constant decoding"),
                }
            }
        }
    }
}
