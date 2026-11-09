//! ErrorKind-like direct-tag enum fixture shared by layout and machine tests.
//! `lanes(selector, a, bits) -> u64` builds a variant, carries it through
//! constructor and loop Phis, round-trips it through memory and decodes it.
use kfrc_body::*;

struct Types {
    types: RecordedTypes,
    word: TyId,
    byte: TyId,
    half: TyId,
    wide: TyId,
    boolean: TyId,
    position: TyId,
    kind: TyId,
}

fn int(types: &mut RecordedTypes, width: IntegerWidth, size: u64) -> TyId {
    types.intern(
        TypeShape::Int {
            width,
            signed: false,
        },
        Some(SizeAlign { size, align: size }),
    )
}

fn direct_enum(
    types: &mut RecordedTypes,
    name: &str,
    size: u64,
    align: u64,
    tag: TyId,
    variants: Vec<Vec<FieldLayout>>,
) -> TyId {
    let ty = types.intern(
        TypeShape::Adt { name: name.into() },
        Some(SizeAlign { size, align }),
    );
    let discriminants: Vec<u128> = (0..variants.len() as u128).collect();
    types.record_aggregate(
        ty,
        AggregateLayout {
            fields: vec![],
            variants,
        },
    );
    types.record_semantic_enum(ty, discriminants.clone());
    types.enums.insert(
        ty,
        EnumLayout {
            tag: Some(FieldLayout { ty: tag, offset: 0 }),
            encoding: TagEncoding::Direct { discriminants },
        },
    );
    ty
}

/// An ErrorKind-like enum: byte payloads at 1 (a fieldless enum or u8) and
/// u32 payloads at 4. `overlap` adds a u16 at 4, which overlaps the u32 lane.
fn types(overlap: bool) -> Types {
    let mut types = RecordedTypes::new();
    let word = int(&mut types, IntegerWidth::W64, 8);
    let byte = int(&mut types, IntegerWidth::W8, 1);
    let half = int(&mut types, IntegerWidth::W16, 2);
    let wide = int(&mut types, IntegerWidth::W32, 4);
    let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let position = direct_enum(&mut types, "Position", 1, 1, byte, vec![vec![]; 5]);
    let at = |ty, offset| FieldLayout { ty, offset };
    let mut variants = vec![
        vec![],
        vec![at(position, 1)],
        vec![at(byte, 1)],
        vec![at(wide, 4)],
        vec![at(position, 1), at(wide, 4)],
        vec![at(byte, 1), at(wide, 4)],
    ];
    if overlap {
        variants.push(vec![at(half, 4)]);
    }
    let kind = direct_enum(&mut types, "Kind", 8, 4, byte, variants);
    Types {
        types,
        word,
        byte,
        half,
        wide,
        boolean,
        position,
        kind,
    }
}

fn local(b: &mut BodyBuilder, ty: TyId) -> LocalId {
    b.add_local(LocalDecl {
        ty,
        mutability: Mutability::Mut,
        name: None,
    })
}
fn copy(local: LocalId) -> Operand {
    Operand::Copy(Place::local(local))
}
fn constant(b: &mut BodyBuilder, ty: TyId, bits: u128) -> Operand {
    Operand::Constant(b.add_constant(Constant {
        ty,
        kind: ConstantKind::Scalar(bits),
    }))
}
fn assign(b: &mut BodyBuilder, place: Place, value: Rvalue) {
    b.push_statement(StatementKind::Assign(place, value));
}
fn payload(b: &mut BodyBuilder, local: LocalId, variant: u32, field: u32, ty: TyId) -> Place {
    Place {
        local,
        projection: b.add_projection(vec![
            ProjectionElem::Downcast(VariantIndex(variant)),
            ProjectionElem::Field(FieldIndex(field), ty),
        ]),
    }
}
fn cast(value: Operand, ty: TyId) -> Rvalue {
    Rvalue::Cast(CastKind::IntToInt, value, ty)
}
fn switch(b: &mut BodyBuilder, value: Operand, cases: Vec<(u128, usize)>, otherwise: usize) {
    b.finish_block(
        Terminator::SwitchInt {
            discriminant: value,
            targets: SwitchTargets {
                cases: cases
                    .into_iter()
                    .map(|(value, block)| (value, BlockId(block as u32)))
                    .collect(),
                otherwise: BlockId(otherwise as u32),
            },
        },
        false,
    );
}
fn goto(b: &mut BodyBuilder, block: usize) {
    b.finish_block(Terminator::Goto(BlockId(block as u32)), false);
}

/// `lanes(selector, a, b)`: build a variant chosen by `selector` (constructor
/// Phi), carry it through a counted loop (loop Phi), store it through a raw
/// pointer, load it back, then decode every payload into a u64 checksum.
pub fn lanes_body(overlap: bool) -> Body {
    let t = types(overlap);
    let Types {
        word,
        byte,
        half,
        wide,
        boolean,
        position,
        kind,
        ..
    } = t;
    let pointer = {
        let mut types = t.types;
        let pointer = types.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Mut,
                pointee: kind,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        (types, pointer)
    };
    let (types, pointer) = pointer;
    let variant_count = if overlap { 7 } else { 6 };
    let mut b = BodyBuilder::new(
        format!("variant_lanes_{}", u8::from(overlap)),
        FunctionAbi {
            args: vec![PassMode::Direct; 3],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    let ret = local(&mut b, word);
    let selector = local(&mut b, word);
    let a = local(&mut b, word);
    let bits = local(&mut b, word);
    b.set_arg_count(3);
    let value = local(&mut b, kind);
    let narrow = local(&mut b, byte);
    let narrow_half = local(&mut b, half);
    let narrow_wide = local(&mut b, wide);
    let counter = local(&mut b, word);
    let running = local(&mut b, boolean);
    let storage = local(&mut b, kind);
    let address = local(&mut b, pointer);
    let reloaded = local(&mut b, kind);
    let tag = local(&mut b, word);
    let inner = local(&mut b, word);
    let widened = local(&mut b, word);
    let partial = local(&mut b, word);

    // bb0: narrow the dynamic inputs, then choose a constructor.
    let a_byte = cast(copy(a), byte);
    assign(&mut b, Place::local(narrow), a_byte);
    let b_wide = cast(copy(bits), wide);
    assign(&mut b, Place::local(narrow_wide), b_wide);
    let b_half = cast(copy(bits), half);
    assign(&mut b, Place::local(narrow_half), b_half);
    let cases = (1..variant_count as u128)
        .map(|v| (v, v as usize))
        .collect();
    switch(&mut b, copy(selector), cases, variant_count);
    // bb1..: constructors; the last block is the fieldless variant.
    let position_value = local(&mut b, position);
    let mut payloads: Vec<Vec<Operand>> = vec![vec![]; variant_count + 1];
    for (variant, fields) in payloads.iter_mut().enumerate().skip(1) {
        *fields = match variant {
            1 | 4 => vec![copy(position_value)],
            2 | 5 => vec![copy(narrow)],
            6 => vec![copy(narrow_half)],
            _ => vec![],
        };
        if matches!(variant, 3..=5) {
            fields.push(copy(narrow_wide));
        }
    }
    let join = variant_count + 1;
    for variant in 1..=variant_count {
        let constructed = if variant == variant_count { 0 } else { variant };
        if matches!(constructed, 1 | 4) {
            assign(
                &mut b,
                Place::local(position_value),
                Rvalue::Aggregate(
                    AggregateKind::Adt {
                        ty: position,
                        variant: VariantIndex(constructed as u32 % 5),
                    },
                    vec![],
                ),
            );
        }
        assign(
            &mut b,
            Place::local(value),
            Rvalue::Aggregate(
                AggregateKind::Adt {
                    ty: kind,
                    variant: VariantIndex(constructed as u32),
                },
                payloads[constructed].clone(),
            ),
        );
        goto(&mut b, join);
    }
    // join: counted loop that replaces V0 (selector 0) by V4 on its first iteration.
    let zero = constant(&mut b, word, 0);
    assign(&mut b, Place::local(counter), Rvalue::Use(zero));
    goto(&mut b, join + 1);
    // loop head
    let two = constant(&mut b, word, 2);
    assign(
        &mut b,
        Place::local(running),
        Rvalue::BinaryOp(BinaryOp::Lt, copy(counter), two),
    );
    switch(&mut b, copy(running), vec![(0, join + 4)], join + 2);
    // loop body: if counter == 1 && selector == 0 rebuild as V4.
    let one = constant(&mut b, word, 1);
    assign(
        &mut b,
        Place::local(counter),
        Rvalue::BinaryOp(BinaryOp::Add, copy(counter), one),
    );
    let sum = local(&mut b, word);
    assign(
        &mut b,
        Place::local(sum),
        Rvalue::BinaryOp(BinaryOp::Add, copy(counter), copy(selector)),
    );
    let one = constant(&mut b, word, 1);
    let rebuild = local(&mut b, boolean);
    assign(
        &mut b,
        Place::local(rebuild),
        Rvalue::BinaryOp(BinaryOp::Eq, copy(sum), one),
    );
    switch(&mut b, copy(rebuild), vec![(0, join + 1)], join + 3);
    assign(
        &mut b,
        Place::local(position_value),
        Rvalue::Aggregate(
            AggregateKind::Adt {
                ty: position,
                variant: VariantIndex(3),
            },
            vec![],
        ),
    );
    assign(
        &mut b,
        Place::local(value),
        Rvalue::Aggregate(
            AggregateKind::Adt {
                ty: kind,
                variant: VariantIndex(4),
            },
            vec![copy(position_value), copy(narrow_wide)],
        ),
    );
    goto(&mut b, join + 1);
    // after loop: memory round trip.
    assign(
        &mut b,
        Place::local(address),
        Rvalue::RawPointer(Mutability::Mut, Place::local(storage)),
    );
    let deref = b.add_projection(vec![ProjectionElem::Deref]);
    let through = Place {
        local: address,
        projection: deref,
    };
    assign(&mut b, through, Rvalue::Use(copy(value)));
    assign(
        &mut b,
        Place::local(reloaded),
        Rvalue::Use(Operand::Copy(through)),
    );
    assign(
        &mut b,
        Place::local(tag),
        Rvalue::Discriminant(Place::local(reloaded)),
    );
    let decode = join + 5;
    let cases = (0..variant_count as u128)
        .map(|v| (v, decode + v as usize))
        .collect();
    switch(&mut b, copy(tag), cases, decode + variant_count);
    let done = decode + variant_count + 1;
    for variant in 0..variant_count as u32 {
        let tag_bits = constant(&mut b, word, 1000 * u128::from(variant));
        assign(&mut b, Place::local(partial), Rvalue::Use(tag_bits));
        let fields: Vec<TyId> = match variant {
            1 => vec![position],
            2 => vec![byte],
            3 => vec![wide],
            4 => vec![position, wide],
            5 => vec![byte, wide],
            6 => vec![half],
            _ => vec![],
        };
        for (index, ty) in fields.into_iter().enumerate() {
            let field = payload(&mut b, reloaded, variant, index as u32, ty);
            if ty == position {
                assign(&mut b, Place::local(inner), Rvalue::Discriminant(field));
            } else {
                let widen = cast(Operand::Copy(field), word);
                assign(&mut b, Place::local(inner), widen);
            }
            let scale = constant(&mut b, word, if index == 0 { 1 } else { 1 << 16 });
            assign(
                &mut b,
                Place::local(widened),
                Rvalue::BinaryOp(BinaryOp::Mul, copy(inner), scale),
            );
            assign(
                &mut b,
                Place::local(partial),
                Rvalue::BinaryOp(BinaryOp::Add, copy(partial), copy(widened)),
            );
        }
        assign(&mut b, Place::local(ret), Rvalue::Use(copy(partial)));
        goto(&mut b, done);
    }
    b.finish_block(Terminator::Unreachable, false);
    b.finish_block(Terminator::Return, false);
    b.finish()
}

pub fn expected(selector: u64, a: u64, bits: u64, overlap: bool) -> u64 {
    let variant_count = if overlap { 7 } else { 6 };
    let variant = if selector == 0 || selector >= variant_count {
        // V0 is replaced by V4(Position 3, b) inside the loop.
        if selector == 0 { 4 } else { 0 }
    } else {
        selector
    };
    let byte = a & 0xff;
    let wide = bits & 0xffff_ffff;
    1000 * variant
        + match variant {
            1 => 1,
            2 => byte,
            3 => wide,
            4 if selector == 0 => 3 + (wide << 16),
            4 => 4 % 5 + (wide << 16),
            5 => byte + (wide << 16),
            6 => bits & 0xffff,
            _ => 0,
        }
}
