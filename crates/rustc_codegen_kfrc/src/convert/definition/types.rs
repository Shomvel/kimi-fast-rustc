use super::*;
use kfrc_body::{IntegerWidth, SemanticAggregate};

impl<'tcx> DefinitionCx<'tcx> {
    pub(super) fn ty(&mut self, native: Ty<'tcx>) -> Result<TyId, String> {
        if let Some(&id) = self.types.get(&native) {
            return Ok(id);
        }
        let shape = match native.kind() {
            ty::Param(_) if !native.is_sized(self.tcx, self.typing_env) => {
                return Err(
                    "possibly unsized parameter requires dynamic dispatch/metadata adaptation"
                        .into(),
                );
            }
            ty::Param(param) => TypeShape::Param {
                index: param.index,
                name: param.name.to_string(),
            },
            ty::Bool => TypeShape::Bool,
            ty::Char => TypeShape::Char,
            ty::Int(width) => TypeShape::Int {
                width: integer(width.bit_width())?,
                signed: true,
            },
            ty::Uint(width) => TypeShape::Int {
                width: integer(width.bit_width())?,
                signed: false,
            },
            ty::Tuple(fields) if fields.is_empty() => TypeShape::Unit,
            ty::Tuple(fields) => TypeShape::Tuple(
                fields
                    .iter()
                    .map(|field| self.ty(field))
                    .collect::<Result<_, _>>()?,
            ),
            ty::Array(element, length) => TypeShape::Array {
                element: self.ty(*element)?,
                length: length
                    .try_to_target_usize(self.tcx)
                    .ok_or("generic array length needs a ConstValue hole")?,
            },
            ty::Never => TypeShape::Never,
            ty::Ref(_, pointee, mutability) => TypeShape::Reference {
                mutability: super::super::convert_mutability(*mutability),
                pointee: self.ty(*pointee)?,
            },
            ty::RawPtr(pointee, mutability) => TypeShape::RawPointer {
                mutability: super::super::convert_mutability(*mutability),
                pointee: self.ty(*pointee)?,
            },
            ty::Adt(def, _) if !def.is_union() => TypeShape::Adt {
                name: crate::type_identity::name(self.tcx, native),
            },
            _ => {
                return Err(format!(
                    "definition type `{native}` is not in the R producer subset"
                ));
            }
        };
        let id = self.graph.oracle.intern(shape, None);
        self.types.insert(native, id);
        self.native_types.insert(id, native);
        if let ty::Adt(def, args) = native.kind() {
            if self.tcx.lang_items().ordering_enum() == Some(def.did()) {
                self.graph.oracle.record_ordering(id);
            }
            let variants = def
                .variants()
                .iter()
                .map(|variant| {
                    variant
                        .fields
                        .iter()
                        .map(|field| self.ty(field.ty(self.tcx, args)))
                        .collect::<Result<Vec<_>, _>>()
                })
                .collect::<Result<Vec<_>, _>>()?;
            let fields = if def.is_struct() {
                variants[0].clone()
            } else {
                Vec::new()
            };
            self.graph
                .oracle
                .record_semantic_aggregate(id, SemanticAggregate { fields, variants });
            if def.is_enum() {
                self.graph.oracle.record_semantic_enum(
                    id,
                    def.discriminants(self.tcx)
                        .map(|(_, discriminant)| discriminant.val)
                        .collect(),
                );
            }
        }
        Ok(id)
    }
}
fn integer(bits: Option<u64>) -> Result<IntegerWidth, String> {
    Ok(match bits {
        None => IntegerWidth::Pointer,
        Some(8) => IntegerWidth::W8,
        Some(16) => IntegerWidth::W16,
        Some(32) => IntegerWidth::W32,
        Some(64) => IntegerWidth::W64,
        _ => return Err("R producer integer width above 64".into()),
    })
}
