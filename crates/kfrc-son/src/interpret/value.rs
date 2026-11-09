//! Owned semantic snapshots. Repeated arrays share their default element and
//! sparse overrides, so cloning or projecting one never expands its length.
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Debug, Eq)]
pub enum Value {
    Scalar(u64),
    /// MaybeUninit representation snapshot; None bytes are legal contents,
    /// not poison observations, until a typed T decode is requested.
    Uninit {
        bytes: Arc<Vec<Option<u8>>>,
    },
    Aggregate(Vec<Value>),
    Array {
        element: Arc<Value>,
        length: u64,
        overrides: Arc<HashMap<u64, Value>>,
    },
    Variant {
        variant: u32,
        fields: Vec<Value>,
    },
    WidePointer {
        data: Box<Value>,
        metadata: Box<Value>,
    },
    Poison,
}

impl Value {
    pub fn array(element: Value, length: u64) -> Self {
        Self::Array {
            element: Arc::new(element),
            length,
            overrides: Arc::new(HashMap::new()),
        }
    }

    pub fn array_element(&self, index: u64) -> Option<&Value> {
        let Self::Array {
            element,
            length,
            overrides,
        } = self
        else {
            return None;
        };
        (index < *length).then(|| overrides.get(&index).unwrap_or(element))
    }

    pub fn contains_poison(&self) -> bool {
        match self {
            Self::Poison => true,
            Self::Aggregate(fields) | Self::Variant { fields, .. } => {
                fields.iter().any(Self::contains_poison)
            }
            Self::Array {
                element,
                length,
                overrides,
            } => {
                ((overrides.len() as u64) < *length && element.contains_poison())
                    || overrides.values().any(Self::contains_poison)
            }
            Self::WidePointer { data, metadata } => {
                data.contains_poison() || metadata.contains_poison()
            }
            Self::Scalar(_) | Self::Uninit { .. } => false,
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Scalar(left), Self::Scalar(right)) => left == right,
            (Self::Uninit { bytes: left }, Self::Uninit { bytes: right }) => left == right,
            (Self::Poison, Self::Poison) => true,
            (Self::Aggregate(left), Self::Aggregate(right)) => left == right,
            (
                Self::Variant {
                    variant: left_variant,
                    fields: left,
                },
                Self::Variant {
                    variant: right_variant,
                    fields: right,
                },
            ) => left_variant == right_variant && left == right,
            (
                Self::WidePointer {
                    data: left_data,
                    metadata: left_metadata,
                },
                Self::WidePointer {
                    data: right_data,
                    metadata: right_metadata,
                },
            ) => left_data == right_data && left_metadata == right_metadata,
            (Self::Array { length, .. }, Self::Aggregate(fields)) => {
                *length == fields.len() as u64
                    && fields
                        .iter()
                        .enumerate()
                        .all(|(index, field)| self.array_element(index as u64) == Some(field))
            }
            (Self::Aggregate(_), Self::Array { .. }) => other == self,
            (
                Self::Array {
                    element: left,
                    length: left_length,
                    overrides: left_overrides,
                },
                Self::Array {
                    element: right,
                    length: right_length,
                    overrides: right_overrides,
                },
            ) => {
                if left_length != right_length {
                    return false;
                }
                if left_length == &0 {
                    return true;
                }
                // If defaults differ, every index must have an override on at
                // least one side. Count the union without scanning the length.
                if left != right {
                    let union_len = left_overrides.len()
                        + right_overrides
                            .keys()
                            .filter(|index| !left_overrides.contains_key(index))
                            .count();
                    if union_len as u64 != *left_length {
                        return false;
                    }
                }
                left_overrides
                    .keys()
                    .chain(right_overrides.keys())
                    .all(|&index| self.array_element(index) == other.array_element(index))
            }
            _ => false,
        }
    }
}

/// Type-directed construction/checking shared by S execution and the independent
/// R interpreter's lazily instantiated type environment. No ABI is consulted.
pub(super) fn poison(
    types: &kfrc_body::RecordedTypes,
    ty: kfrc_body::TyId,
) -> Result<Value, super::InterpretError> {
    use kfrc_body::TypeShape;
    let semantic = types.semantic();
    if semantic.maybe_uninit_inner(ty).is_some() {
        use kfrc_body::TypeOracle;
        let size = types
            .size_align(ty)
            .ok_or_else(|| {
                super::InterpretError::Malformed("MaybeUninit lacks sized representation".into())
            })?
            .size;
        if size > 16 * 1024 * 1024 {
            return Err(super::InterpretError::OutOfFuel);
        }
        return Ok(Value::Uninit {
            bytes: Arc::new(vec![None; size as usize]),
        });
    }

    if semantic.is_enum(ty) {
        return Ok(Value::Poison);
    }
    if let TypeShape::Array { element, length } = semantic.shape(ty) {
        return Ok(Value::array(
            if *length == 0 {
                Value::Poison
            } else {
                poison(types, *element)?
            },
            *length,
        ));
    }
    if let Some(fields) = semantic.field_types(ty, None) {
        let fields = fields
            .into_iter()
            .map(|ty| poison(types, ty))
            .collect::<Result<Vec<_>, _>>()?;
        if matches!(
            semantic.shape(ty),
            TypeShape::Reference { .. } | TypeShape::RawPointer { .. }
        ) {
            let [data, metadata]: [Value; 2] = fields.try_into().map_err(|_| {
                super::InterpretError::Malformed("wide pointer requires two semantic fields".into())
            })?;
            return Ok(Value::WidePointer {
                data: Box::new(data),
                metadata: Box::new(metadata),
            });
        }
        return Ok(Value::Aggregate(fields));
    }
    Ok(Value::Poison)
}

pub(super) fn check(
    types: &kfrc_body::RecordedTypes,
    ty: kfrc_body::TyId,
    value: &Value,
) -> Result<(), super::InterpretError> {
    use kfrc_body::TypeShape;
    let bad = || super::InterpretError::Malformed("value does not match its semantic type".into());
    if *value == Value::Poison {
        return Ok(());
    }
    let semantic = types.semantic();
    if semantic.maybe_uninit_inner(ty).is_some() {
        use kfrc_body::TypeOracle;
        return match value {
            Value::Uninit { bytes }
                if types
                    .size_align(ty)
                    .is_some_and(|layout| layout.size == bytes.len() as u64) =>
            {
                Ok(())
            }
            _ => Err(bad()),
        };
    }
    if matches!(value, Value::Uninit { .. }) {
        return Err(bad());
    }

    if let Value::Array {
        element: default,
        length: count,
        overrides,
    } = value
    {
        let TypeShape::Array { element, length } = semantic.shape(ty) else {
            return Err(bad());
        };
        if count != length || overrides.keys().any(|index| index >= length) {
            return Err(bad());
        }
        if (overrides.len() as u64) < *length {
            check(types, *element, default)?;
        }
        for value in overrides.values() {
            check(types, *element, value)?;
        }
        return Ok(());
    }
    if let Value::Aggregate(values) = value
        && let TypeShape::Array { element, length } = semantic.shape(ty)
    {
        if values.len() as u64 != *length {
            return Err(bad());
        }
        for value in values {
            check(types, *element, value)?;
        }
        return Ok(());
    }
    let (field_types, fields): (Vec<_>, Vec<&Value>) = match value {
        Value::Variant { variant, fields } if semantic.is_enum(ty) => (
            semantic.field_types(ty, Some(*variant)).ok_or_else(bad)?,
            fields.iter().collect(),
        ),
        Value::Aggregate(fields) if !semantic.is_enum(ty) => (
            semantic.field_types(ty, None).ok_or_else(bad)?,
            fields.iter().collect(),
        ),
        Value::WidePointer { data, metadata }
            if matches!(
                semantic.shape(ty),
                TypeShape::Reference { .. } | TypeShape::RawPointer { .. }
            ) =>
        {
            (
                semantic.field_types(ty, None).ok_or_else(bad)?,
                vec![data, metadata],
            )
        }
        Value::Scalar(bits) if !semantic.is_enum(ty) && !semantic.is_product(ty) => {
            let width = match semantic.shape(ty) {
                TypeShape::Bool => 1,
                TypeShape::Char => {
                    if char::from_u32(*bits as u32).is_none() {
                        return Err(bad());
                    }
                    32
                }
                TypeShape::Int { width, .. } => u32::from(width.bits(64)),
                TypeShape::Reference { .. }
                | TypeShape::RawPointer { .. }
                | TypeShape::PlaceAddress { .. }
                | TypeShape::FunctionItem { .. } => 64,
                _ => return Err(bad()),
            };
            if width < 64 && bits >> width != 0 {
                return Err(bad());
            }
            return Ok(());
        }
        _ => return Err(bad()),
    };
    if field_types.len() != fields.len() {
        return Err(bad());
    }
    for (ty, field) in field_types.into_iter().zip(fields) {
        check(types, ty, field)?;
    }
    Ok(())
}
