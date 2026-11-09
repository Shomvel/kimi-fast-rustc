//! Linear-scan homes and short constrained fragments at instruction boundaries.
use super::*;

pub(super) struct Allocation {
    segments: Vec<Vec<super::scan::Segment>>,
    memory_slots: Vec<Option<StackSlot>>,
    pub liveness: liveness::Liveness,
    pub spill_slots: u32,
    // Indexed by target. None means this is not an actual adjacent CFG edge,
    // or that a malformed home requires the original diagnostic path.
    adjacent_edges: Vec<Option<Vec<Copy>>>,
}
impl Allocation {
    pub fn new(instructions: &[Instruction], count: u32) -> Result<Self, LowerError> {
        let facts: Vec<_> = instructions.iter().map(Instruction::facts).collect();
        let liveness = liveness::analyze(&facts);
        let scan = super::scan::scan(instructions, &liveness, count)?;
        let mut allocation = Self {
            segments: scan.segments,
            memory_slots: scan.memory_slots,
            liveness,
            spill_slots: scan.spill_slots,
            adjacent_edges: Vec::new(),
        };
        allocation.adjacent_edges = allocation.adjacent_edge_copies();
        Ok(allocation)
    }
    /// The slot holding `v` wherever it is live, when stored at definitions.
    pub fn memory_slot(&self, v: VirtualReg) -> Option<StackSlot> {
        self.memory_slots.get(v.0 as usize).copied().flatten()
    }
    /// A boundary copy into the value's own memory slot rewrites what the slot
    /// already holds.
    fn needs_copy(&self, v: VirtualReg, copy: &Copy) -> bool {
        copy.src != copy.dst
            && self
                .memory_slot(v)
                .is_none_or(|slot| copy.dst != Location::Stack(slot))
    }
    pub fn home(&self, v: VirtualReg, position: usize) -> Result<Location, LowerError> {
        self.segments
            .get(v.0 as usize)
            .and_then(|parts| {
                let index = parts.partition_point(|s| s.end <= position);
                parts.get(index).filter(|s| s.start <= position)
            })
            .map(|s| s.location)
            .ok_or_else(|| {
                LowerError::Malformed(format!("missing allocation for v{} at {position}", v.0))
            })
    }
    pub fn value(&self, v: VirtualValue, position: usize) -> Result<Location, LowerError> {
        match v {
            VirtualValue::Reg(v) => self.home(v, position),
            VirtualValue::Imm(n) => Ok(Location::Imm(n)),
        }
    }
    /// Values carried along a CFG edge. A value live at the target but
    /// undefined at the source holds no program value on this edge.
    pub fn edge_values(&self, source: usize, target: usize) -> Vec<VirtualReg> {
        let after = &self.liveness.after[source];
        self.liveness.before[target]
            .iter()
            .copied()
            .filter(|v| after.binary_search_by_key(&v.0, |v| v.0).is_ok())
            .collect()
    }
    pub fn live_through(&self, position: usize, definitions: &[VirtualReg]) -> Vec<VirtualReg> {
        let after = &self.liveness.after[position];
        self.liveness.before[position]
            .iter()
            .copied()
            .filter(|v| {
                after.binary_search_by_key(&v.0, |v| v.0).is_ok() && !definitions.contains(v)
            })
            .collect()
    }
    // Along an adjacent edge a home can change only at a segment boundary.
    // Visit those events once instead of scanning every live value at every
    // instruction. Iterating vregs in order preserves parallel-copy ordering.
    fn adjacent_edge_copies(&self) -> Vec<Option<Vec<Copy>>> {
        let mut edges = vec![None; self.liveness.before.len()];
        for target in 1..edges.len() {
            if self.liveness.successors[target - 1].contains(&target) {
                edges[target] = Some(Vec::new());
            }
        }
        for (value, parts) in self.segments.iter().enumerate() {
            let value = VirtualReg(value as u32);
            let mut previous = None;
            for target in parts.iter().flat_map(|part| [part.start, part.end]) {
                if previous == Some(target) {
                    continue;
                }
                previous = Some(target);
                if target == 0 || target >= edges.len() || edges[target].is_none() {
                    continue;
                }
                if self.liveness.before[target]
                    .binary_search_by_key(&value.0, |v| v.0)
                    .is_err()
                    || self.liveness.after[target - 1]
                        .binary_search_by_key(&value.0, |v| v.0)
                        .is_err()
                {
                    continue;
                }
                // Use the same half-open lookup as emission, including holes.
                // On failure defer to edge()'s full scan, preserving its first
                // error and when that error is returned.
                match (self.home(value, target - 1), self.home(value, target)) {
                    (Ok(src), Ok(dst)) => {
                        let copy = Copy { src, dst };
                        if self.needs_copy(value, &copy) {
                            edges[target].as_mut().unwrap().push(copy);
                        }
                    }
                    _ => edges[target] = None,
                }
            }
        }
        edges
    }

    pub fn edge(&self, source: usize, target: usize) -> Result<Vec<Copy>, LowerError> {
        if source.checked_add(1) == Some(target)
            && let Some(Some(copies)) = self.adjacent_edges.get(target)
        {
            return Ok(copies.clone());
        }
        self.edge_values(source, target)
            .into_iter()
            .map(|v| {
                Ok((
                    v,
                    Copy {
                        src: self.home(v, source)?,
                        dst: self.home(v, target)?,
                    },
                ))
            })
            .filter(|c| !matches!(c, Ok((v, c)) if !self.needs_copy(*v, c)))
            .map(|c| c.map(|(_, copy)| copy))
            .collect()
    }
}

pub(super) struct Registers {
    pub inputs: Vec<Reg>,
    pub outputs: Vec<Reg>,
}

impl Allocation {
    pub fn registers(&self, inst: &Instruction, position: usize) -> Result<Registers, LowerError> {
        let definitions: Vec<_> = inst.outputs.iter().map(|output| output.value).collect();
        let live_registers = self
            .live_through(position, &definitions)
            .into_iter()
            .filter_map(|v| self.home(v, position).ok().and_then(Location::register))
            .collect::<Vec<_>>();
        // Registers holding this instruction's input values. Borrowing one
        // for another operand would force that input out of its home first.
        let input_registers = inst
            .inputs
            .iter()
            .filter_map(|input| self.value(input.value, position).ok())
            .filter_map(Location::register)
            .collect::<Vec<_>>();
        let mut inputs = vec![None; inst.inputs.len()];
        for (index, input) in inst.inputs.iter().enumerate() {
            if let Constraint::Fixed(r) = input.constraint {
                inputs[index] = Some(r);
            }
        }
        // Fixed inputs are assigned together, before any unconstrained operand.
        for index in 0..inputs.len() {
            if inputs[index].is_some() {
                continue;
            }
            let input = inst.inputs[index];
            let reuse = inst
                .outputs
                .iter()
                .find(|o| matches!(o.constraint,Constraint::Reuse(i) if i==index));
            let preferred = reuse
                .and_then(|o| self.home(o.value, position).ok())
                .and_then(Location::register);
            let source = self.value(input.value, position)?.register();
            let allowed = |r: &Reg| {
                !inst.clobbers.contains(r)
                    && inputs.iter().enumerate().all(|(j, reg)| {
                        reg != &Some(*r) || (inst.inputs[j].value == input.value && reuse.is_none())
                    })
            };
            // Materializing an immediate or a spilled operand must not always
            // borrow rax: that turns a long-lived rax value into a spill pair
            // at every instruction even with many free registers available.
            let unchanged_source =
                source.filter(|r| reuse.is_none() || !live_registers.contains(r));
            inputs[index] = preferred
                .into_iter()
                .chain(unchanged_source)
                .chain(
                    Reg::ALLOCATABLE
                        .into_iter()
                        .filter(|r| !live_registers.contains(r) && !input_registers.contains(r)),
                )
                .chain(
                    Reg::ALLOCATABLE
                        .into_iter()
                        .filter(|r| !live_registers.contains(r)),
                )
                .chain(source)
                .chain(Reg::ALLOCATABLE)
                .find(allowed);
            if inputs[index].is_none() {
                return Err(LowerError::Malformed(
                    "instruction requires too many input registers".into(),
                ));
            }
        }
        let inputs: Vec<_> = inputs.into_iter().map(Option::unwrap).collect();
        let mut outputs = Vec::new();
        for output in &inst.outputs {
            let r = match output.constraint {
                Constraint::Fixed(r) => r,
                Constraint::Reuse(i) => inputs[i],
                Constraint::Any => {
                    let preferred = self.home(output.value, position)?.register();
                    preferred
                        .into_iter()
                        .chain(
                            Reg::ALLOCATABLE
                                .into_iter()
                                .filter(|r| !live_registers.contains(r)),
                        )
                        .chain(Reg::ALLOCATABLE)
                        .find(|r| {
                            !inst.clobbers.contains(r)
                                && !outputs.contains(r)
                                && inst.inputs.iter().zip(&inputs).all(|(input, reg)| {
                                    !matches!(input.phase, Phase::Late) || r != reg
                                })
                        })
                        .ok_or_else(|| {
                            LowerError::Malformed(
                                "instruction requires too many output registers".into(),
                            )
                        })?
                }
            };
            outputs.push(r);
        }
        Ok(Registers { inputs, outputs })
    }
}

#[cfg(test)]
mod tests {
    use super::super::scan::Segment;
    use super::*;

    fn segment(start: usize, end: usize, location: Location) -> Segment {
        Segment {
            start,
            end,
            location,
        }
    }

    fn full_edge(
        allocation: &Allocation,
        source: usize,
        target: usize,
    ) -> Result<Vec<Copy>, LowerError> {
        allocation
            .edge_values(source, target)
            .into_iter()
            .map(|value| {
                Ok(Copy {
                    src: allocation.home(value, source)?,
                    dst: allocation.home(value, target)?,
                })
            })
            .filter(|copy| !matches!(copy, Ok(copy) if copy.src == copy.dst))
            .collect()
    }

    fn locations(copies: Vec<Copy>) -> Vec<(Location, Location)> {
        copies
            .into_iter()
            .map(|copy| (copy.src, copy.dst))
            .collect()
    }

    fn fixture() -> Allocation {
        let mut allocation = Allocation {
            segments: vec![
                vec![
                    segment(0, 2, Location::Reg(Reg::Rax)),
                    segment(2, 5, Location::Reg(Reg::Rcx)),
                ],
                // This lifetime hole is crossed by a real non-adjacent edge.
                vec![
                    segment(0, 1, Location::Reg(Reg::Rdi)),
                    segment(3, 6, Location::Reg(Reg::Rdx)),
                ],
                vec![
                    segment(0, 2, Location::Reg(Reg::Rdx)),
                    segment(2, 6, Location::Stack(StackSlot(0))),
                ],
                // A dying input and its coalesced output overlap at instruction
                // 1. Their different end points must not create an edge copy.
                vec![segment(0, 2, Location::Reg(Reg::R8))],
                vec![segment(1, 3, Location::Reg(Reg::R8))],
            ],
            liveness: liveness::Liveness {
                intervals: Vec::new(),
                before: vec![
                    vec![],
                    vec![VirtualReg(0), VirtualReg(2), VirtualReg(3)],
                    vec![VirtualReg(0), VirtualReg(2), VirtualReg(4)],
                    vec![VirtualReg(0), VirtualReg(1), VirtualReg(2)],
                    vec![VirtualReg(0), VirtualReg(1), VirtualReg(2)],
                    vec![VirtualReg(1), VirtualReg(2)],
                ],
                after: vec![
                    vec![VirtualReg(0), VirtualReg(1), VirtualReg(2), VirtualReg(3)],
                    vec![VirtualReg(0), VirtualReg(2), VirtualReg(4)],
                    vec![],
                    vec![VirtualReg(0), VirtualReg(1), VirtualReg(2)],
                    vec![VirtualReg(1), VirtualReg(2)],
                    vec![],
                ],
                successors: vec![vec![1, 3], vec![2], vec![], vec![4], vec![5], vec![]],
            },
            memory_slots: Vec::new(),
            spill_slots: 1,
            adjacent_edges: Vec::new(),
        };
        allocation.adjacent_edges = allocation.adjacent_edge_copies();
        allocation
    }

    #[test]
    fn adjacent_event_cache_preserves_copy_order_holes_and_coalesced_ends() {
        let allocation = fixture();
        for (source, targets) in allocation.liveness.successors.iter().enumerate() {
            for &target in targets {
                assert_eq!(
                    locations(allocation.edge(source, target).unwrap()),
                    locations(full_edge(&allocation, source, target).unwrap()),
                    "edge {source} -> {target}"
                );
            }
        }
        assert_eq!(
            locations(allocation.edge(1, 2).unwrap()),
            vec![
                (Location::Reg(Reg::Rax), Location::Reg(Reg::Rcx)),
                (Location::Reg(Reg::Rdx), Location::Stack(StackSlot(0))),
            ]
        );
        assert!(allocation.adjacent_edges[4].as_ref().unwrap().is_empty());
        assert_eq!(
            locations(allocation.edge(0, 3).unwrap()),
            vec![
                (Location::Reg(Reg::Rax), Location::Reg(Reg::Rcx)),
                (Location::Reg(Reg::Rdi), Location::Reg(Reg::Rdx)),
                (Location::Reg(Reg::Rdx), Location::Stack(StackSlot(0))),
            ]
        );
    }

    #[test]
    fn malformed_boundary_and_non_cfg_queries_keep_full_scan_errors() {
        let mut allocation = fixture();
        allocation.liveness.successors[2].push(3);
        allocation.liveness.after[2] = (0..4).map(VirtualReg).collect();
        allocation.adjacent_edges = allocation.adjacent_edge_copies();
        // v1 is live at target 3 but has no source home at 2. Cache construction
        // must not turn this into an empty copy list or an eager allocation error.
        assert!(allocation.adjacent_edges[3].is_none());
        for (source, target) in [(2, 3), (2, 1)] {
            assert_eq!(
                allocation.edge(source, target).unwrap_err().to_string(),
                full_edge(&allocation, source, target)
                    .unwrap_err()
                    .to_string()
            );
        }
    }
}
