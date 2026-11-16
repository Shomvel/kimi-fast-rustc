//! Storage lanes: frame storage that only ever holds scalars at fixed offsets
//! becomes SSA variables.
//!
//! Layout lowering gives every composite value that crosses a call boundary
//! a frame slot: fields are stored into it and the machine layer reads them
//! back into argument registers, or writes result registers into it for the
//! graph to load. Such storage is private when every use of its address is a
//! load or store at a constant offset, a lifetime marker, a by-value transfer
//! whose register lanes are known, or a merge with other private storage.
//! Accesses describe byte ranges, independently of Rust field types. Their
//! overlapping ranges share integer lanes; a narrower access extracts or
//! updates its bytes. Each lane is one Cranelift variable, and Cranelift's
//! SSA construction places the block parameters.
//!
//! A `Phi` of aggregate addresses is storage of its own: on every incoming
//! edge its lanes take the values of the corresponding lanes of that edge's
//! input.
//!
//! A `memcpy`/`memmove` of a constant size moves lanes. Between two lane
//! storages the copy reads all source bytes before updating the destination.
//! Every byte of the copied range has storage, including padding that no
//! typed load or store names.
//!
//! Storage with any other use of its address keeps its frame slot.

use cranelift_codegen::ir::Type;
use cranelift_frontend::Variable;
use kfrc_body::fx_hash::{HashMap, HashSet};
use kfrc_body::{CastAbi, CastExtension, PassMode, TypeOracle, TypeShape};
use kfrc_son::{NodeId, Opcode, ValueType};

use super::{Exporter, LowerError, integer_type};

/// A byte range of one storage accessed as a single integer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Lane {
    pub offset: u64,
    pub bytes: u8,
    pub variable: Variable,
}

impl Lane {
    pub(super) fn ty(self) -> Type {
        integer_type(self.bytes * 8).expect("lane widths are 1, 2, 4 or 8 bytes")
    }
}

#[derive(Default)]
pub(super) struct Promotion {
    /// Address nodes of promoted storage: storage index and byte offset.
    places: HashMap<NodeId, (usize, u64)>,
    /// Lanes of each promoted storage, by ascending offset.
    storages: Vec<Vec<Lane>>,
    /// Composite parameters and call results whose lanes arrive in registers.
    received: HashMap<NodeId, usize>,
    /// Address phis whose lanes are assigned on every incoming edge.
    merged: HashMap<NodeId, usize>,
    /// Address phis that are only returned through the caller's storage, and
    /// the address nodes derived from them. Each incoming edge writes its
    /// input to the caller's storage; the merge itself has no value.
    returned: HashSet<NodeId>,
}

impl Promotion {
    pub(super) fn place(&self, node: NodeId) -> Option<(usize, u64)> {
        self.places.get(&node).copied()
    }

    pub(super) fn received(&self, node: NodeId) -> Option<usize> {
        self.received.get(&node).copied()
    }

    pub(super) fn merged(&self, phi: NodeId) -> Option<usize> {
        self.merged.get(&phi).copied()
    }

    pub(super) fn returned(&self, node: NodeId) -> bool {
        self.returned.contains(&node)
    }

    pub(super) fn lanes(&self, storage: usize) -> &[Lane] {
        &self.storages[storage]
    }
}

/// Where a storage's address first appears.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Root {
    /// A graph stack slot.
    Slot,
    /// A composite parameter or call result that arrives in register lanes.
    Received(NodeId),
    /// A phi of aggregate addresses.
    Merged(NodeId),
}

/// The use that makes a storage keep its frame memory.
type Kept = &'static str;

struct Candidate {
    root: Root,
    /// Every node that is an address inside this storage, with its offset.
    derived: Vec<(NodeId, u64)>,
    lanes: Vec<(u64, u8)>,
    /// Address phis that take an address of this storage as an input.
    merges: Vec<NodeId>,
    /// Scheduled blocks in which this storage is read, written, transferred
    /// or handed to a merge.
    sites: Vec<usize>,
    /// Constant-size copies that read or write this storage.
    copies: Vec<CopyUse>,
    kept: Option<Kept>,
}

/// One side of a constant-size copy, seen from the storage it touches.
#[derive(Clone, Copy)]
struct CopyUse {
    /// Offset of the copied range inside the storage.
    offset: u64,
    bytes: u64,
}

/// A `memcpy`/`memmove` call that copies a constant number of bytes:
/// destination, source and size.
pub(super) fn constant_copy(
    graph: &kfrc_son::FunctionGraph,
    call: NodeId,
) -> Option<(NodeId, NodeId, u64)> {
    if graph.op(call) != Opcode::Call || graph.inputs(call).len() != 4 {
        return None;
    }
    let callee = graph.callees.get(graph.payload(call) as usize)?;
    let count = graph.resolve(graph.input(call, 3));
    if !matches!(callee.symbol.as_str(), "memcpy" | "memmove")
        || callee.abi.args != [PassMode::Direct; 3]
        || callee.abi.ret != PassMode::Direct
        || callee.abi.c_variadic
        || graph.op(count) != Opcode::Const
    {
        return None;
    }
    let bytes = u64::try_from(graph.constant_value(count)).ok()?;
    Some((
        graph.resolve(graph.input(call, 1)),
        graph.resolve(graph.input(call, 2)),
        bytes,
    ))
}

/// Raw lanes covering `[start, end)`: the widest of 8, 4, 2, 1 bytes that
/// fits. SSA lanes have no alignment requirement.
fn raw_lanes(mut start: u64, end: u64, lanes: &mut Vec<(u64, u8)>) {
    while start < end {
        let width = [8u64, 4, 2, 1]
            .into_iter()
            .find(|width| *width <= end - start)
            .expect("one byte always fits");
        lanes.push((start, width as u8));
        start += width;
    }
}

/// The register lanes `(offset, bytes)` of a by-value transfer under `mode`,
/// or `None` when the transfer needs the value in memory.
fn transfer_lanes(
    exporter: &Exporter<'_, '_>,
    mode: &PassMode,
    node: NodeId,
) -> Option<Vec<(u64, u8)>> {
    let graph = exporter.graph;
    let ty = graph.rust_type(node)?;
    // A scalar-typed address is a pointer value, which exposes the storage.
    if exporter.scalar_of_type(ty).is_ok() {
        return None;
    }
    match mode {
        PassMode::Pair | PassMode::ScalarPair(_) => {
            let fields = graph.oracle.abi_pairs.get(&ty)?;
            fields
                .iter()
                .map(|field| {
                    let bits = exporter.scalar_of_type(field.ty).ok()?.bits;
                    (bits != 0).then_some((field.offset, bits / 8))
                })
                .collect()
        }
        PassMode::Direct | PassMode::DirectScalar(_) => {
            let size = graph.oracle.size_align(ty)?.size;
            matches!(size, 1 | 2 | 4 | 8).then(|| vec![(0, size as u8)])
        }
        PassMode::Cast(cast) => cast_transfer_lanes(*cast),
        _ => None,
    }
}

fn cast_transfer_lanes(cast: CastAbi) -> Option<Vec<(u64, u8)>> {
    if cast.extension != CastExtension::None || cast.consecutive {
        return None;
    }
    cast.lanes
        .into_iter()
        .flatten()
        .map(|lane| {
            let bytes = u64::from(lane.bytes).min(cast.size.saturating_sub(lane.offset));
            matches!(bytes, 1 | 2 | 4 | 8).then_some((lane.offset, bytes as u8))
        })
        .collect()
}

struct Analysis<'a, 'graph, 'function> {
    exporter: &'a Exporter<'graph, 'function>,
    users: Vec<Vec<(NodeId, usize)>>,
    /// The scheduled block of every fixed node.
    block_of: HashMap<NodeId, usize>,
    /// Phis whose inputs are written to the caller's storage edge by edge.
    returned: HashSet<NodeId>,
}

impl Analysis<'_, '_, '_> {
    /// The nodes of a merge that is only returned: `phi` is an address phi
    /// in a block that does nothing but return its value through the
    /// caller's storage. Moving that transfer onto the incoming edges leaves
    /// the order of every effect unchanged.
    fn returned_merge(&self, phi: NodeId) -> Option<Vec<NodeId>> {
        let graph = self.exporter.graph;
        let schedule = self.exporter.schedule;
        if graph.abi.ret != PassMode::Indirect || !self.is_address_phi(phi) {
            return None;
        }
        let block = &schedule.blocks[schedule.block_of_head[&graph.input(phi, 0)]];
        let [last] = block.chain.as_slice() else {
            return None;
        };
        if graph.op(*last) != Opcode::Return {
            return None;
        }
        let mut nodes = vec![phi];
        let mut next = 0;
        let mut reaches_return = false;
        while next < nodes.len() {
            let node = nodes[next];
            next += 1;
            for &(user, slot) in &self.users[node.index()] {
                match (graph.op(user), slot) {
                    (Opcode::PtrOffset, 0) => {
                        let step = graph.resolve(graph.input(user, 1));
                        if graph.op(step) != Opcode::Const || graph.constant_value(step) != 0 {
                            return None;
                        }
                        nodes.push(user);
                    }
                    (Opcode::Return, 1) if user == *last => reaches_return = true,
                    _ => return None,
                }
            }
        }
        reaches_return.then_some(nodes)
    }

    /// A constant-size copy whose bytes can move as lanes: small enough to
    /// be expanded, and nobody reads the destination pointer it returns.
    fn admits_copy(&self, call: NodeId) -> bool {
        let Some((_, _, bytes)) = constant_copy(self.exporter.graph, call) else {
            return false;
        };
        bytes <= self.exporter.options.max_unrolled_memory_bytes
            && self
                .exporter
                .projection(call, 2)
                .is_none_or(|result| self.users[result.index()].is_empty())
    }

    /// A scheduled phi of storage addresses: an aggregate value at Layout
    /// level, or the IR's own place address (never a Rust pointer value).
    fn is_address_phi(&self, node: NodeId) -> bool {
        let graph = self.exporter.graph;
        let storage_address = match graph.value_type(node) {
            ValueType::LayoutValue(_) => true,
            ValueType::Rust(ty) => {
                matches!(graph.oracle.shape(*ty), TypeShape::PlaceAddress { .. })
            }
            _ => false,
        };
        graph.op(node) == Opcode::Phi
            && storage_address
            && self
                .exporter
                .schedule
                .block_of_head
                .contains_key(&graph.input(node, 0))
    }

    /// Every access to the storage reached from `addresses`.
    fn candidate(&self, root: Root, addresses: &[NodeId], arriving: Vec<(u64, u8)>) -> Candidate {
        let mut candidate = Candidate {
            root,
            derived: addresses.iter().map(|&node| (node, 0)).collect(),
            lanes: arriving,
            merges: Vec::new(),
            sites: Vec::new(),
            copies: Vec::new(),
            kept: None,
        };
        candidate.kept = self.accesses(&mut candidate).err();
        candidate
    }

    fn accesses(&self, candidate: &mut Candidate) -> Result<(), Kept> {
        let graph = self.exporter.graph;
        let mut next = 0;
        while next < candidate.derived.len() {
            let (address, offset) = candidate.derived[next];
            next += 1;
            for &(user, slot) in &self.users[address.index()] {
                let site = match graph.op(user) {
                    Opcode::PtrOffset | Opcode::SlotLive | Opcode::SlotDead => None,
                    // A merge reads its input at the end of the predecessor.
                    Opcode::Phi if slot >= 1 => self
                        .block_of
                        .get(&graph.resolve(graph.input(graph.input(user, 0), slot - 1)))
                        .copied(),
                    _ => self.block_of.get(&user).copied(),
                };
                if let Some(site) = site
                    && !candidate.sites.contains(&site)
                {
                    candidate.sites.push(site);
                }
                let lanes = &mut candidate.lanes;
                match (graph.op(user), slot) {
                    (Opcode::PtrOffset, 0) => {
                        let step = graph.resolve(graph.input(user, 1));
                        if graph.op(step) != Opcode::Const {
                            return Err("variable offset");
                        }
                        let step = graph.constant_value(step) as u64 as i64;
                        let moved = offset
                            .checked_add_signed(step)
                            .ok_or("offset before the storage")?;
                        candidate.derived.push((user, moved));
                    }
                    (Opcode::Load, 1) => {
                        if let Some(result) = self.exporter.projection(user, 2) {
                            let loaded = self.exporter.scalar_of_node(result);
                            match loaded.map_err(|_| "untyped load")?.bits {
                                0 => {}
                                bits => lanes.push((offset, bits / 8)),
                            }
                        }
                    }
                    (Opcode::Store, 1) => {
                        let stored = self.exporter.scalar_of_node(graph.input(user, 2));
                        match stored.map_err(|_| "untyped store")?.bits {
                            0 => {}
                            bits => lanes.push((offset, bits / 8)),
                        }
                    }
                    (Opcode::SlotLive | Opcode::SlotDead, 1) => {}
                    (Opcode::Call, 1 | 2) if self.admits_copy(user) => {
                        let (_, _, bytes) = constant_copy(graph, user).expect("admitted copy");
                        candidate.copies.push(CopyUse { offset, bytes });
                    }
                    (Opcode::Call, slot) if slot >= 1 => {
                        // Copy and comparison routines take the address as
                        // a pointer value, which `transfer_lanes` rejects.
                        let callee = &graph.callees[graph.payload(user) as usize];
                        let mode = callee
                            .abi
                            .args
                            .get(slot - 1)
                            .ok_or("argument beyond the ABI")?;
                        let transfer =
                            transfer_lanes(self.exporter, mode, address).ok_or_else(|| {
                                match (callee.symbol.as_str(), mode) {
                                    ("memcpy" | "memmove", _) => "copy routine",
                                    ("memcmp" | "bcmp", _) => "comparison routine",
                                    ("memset", _) => "fill routine",
                                    (_, PassMode::Indirect) => "argument passed by address",
                                    (_, PassMode::Direct | PassMode::DirectScalar(_)) => {
                                        "pointer argument"
                                    }
                                    _ => "argument lanes",
                                }
                            })?;
                        lanes.extend(transfer.into_iter().map(|(at, bytes)| (offset + at, bytes)));
                    }
                    (Opcode::CallIndirect, slot) if slot >= 2 => {
                        let abi = &graph.call_abis[graph.payload(user) as usize];
                        let mode = abi.args.get(slot - 2).ok_or("argument beyond the ABI")?;
                        let transfer =
                            transfer_lanes(self.exporter, mode, address).ok_or(match mode {
                                PassMode::Indirect => "argument passed by address",
                                PassMode::Direct | PassMode::DirectScalar(_) => "pointer argument",
                                _ => "argument lanes",
                            })?;
                        lanes.extend(transfer.into_iter().map(|(at, bytes)| (offset + at, bytes)));
                    }
                    (Opcode::Return, 1) => match &graph.abi.ret {
                        // The lanes are written to the caller's storage one
                        // by one; no further range is read.
                        PassMode::Indirect => {
                            let ty = graph.rust_type(address).ok_or("untyped return")?;
                            if self.exporter.scalar_of_type(ty).is_ok() {
                                return Err("pointer returned");
                            }
                        }
                        mode => {
                            let transfer = transfer_lanes(self.exporter, mode, address)
                                .ok_or("return lanes")?;
                            lanes.extend(
                                transfer.into_iter().map(|(at, bytes)| (offset + at, bytes)),
                            );
                        }
                    },
                    // The edge writes this storage to the caller's
                    // storage; the site above places that transfer.
                    (Opcode::Phi, slot) if slot >= 1 && self.returned.contains(&user) => {}
                    (Opcode::Phi, slot)
                        if slot >= 1 && offset == 0 && self.is_address_phi(user) =>
                    {
                        if !candidate.merges.contains(&user) {
                            candidate.merges.push(user);
                        }
                    }
                    (Opcode::Phi, _) => return Err("address merged as a pointer"),
                    (Opcode::Store, _) => return Err("address stored"),
                    (Opcode::Compare, _) => return Err("address compared"),
                    (Opcode::Return, _) => return Err("pointer returned"),
                    (other, _) => return Err(other.name()),
                }
            }
        }
        Ok(())
    }
}

/// Cover the union of accessed bytes with disjoint integer lanes. Overlap is
/// a property of accesses, not a reason to allocate memory. Adjacent ranges
/// stay separate so independent fields keep their existing cheap accesses.
fn partition(lanes: &[(u64, u8)]) -> Option<Vec<(u64, u8)>> {
    let mut lanes = lanes.to_vec();
    lanes.sort_unstable();
    lanes.dedup();
    let mut ranges: Vec<(u64, u64)> = Vec::new();
    for (offset, bytes) in lanes {
        if !matches!(bytes, 1 | 2 | 4 | 8) {
            return None;
        }
        let end = offset.checked_add(u64::from(bytes))?;
        if let Some(last) = ranges.last_mut()
            && offset < last.1
        {
            last.1 = last.1.max(end);
        } else {
            ranges.push((offset, end));
        }
    }
    let mut result = Vec::new();
    for (start, end) in ranges {
        raw_lanes(start, end, &mut result);
    }
    Some(result)
}

impl Exporter<'_, '_> {
    /// Selects the storage that becomes lanes and declares its variables.
    pub(super) fn plan_promotion(&mut self) -> Result<(), LowerError> {
        let graph = self.graph;
        let mut users: Vec<Vec<(NodeId, usize)>> = vec![Vec::new(); graph.len()];
        let mut slots: HashMap<u32, Vec<NodeId>> = HashMap::default();
        let mut slot_order = Vec::new();
        let mut phis = Vec::new();
        for node in graph.ids() {
            for (slot, &input) in graph.inputs(node).iter().enumerate() {
                users[graph.resolve(input).index()].push((node, slot));
            }
            match graph.op(node) {
                Opcode::StackSlot => {
                    let index = graph.payload(node);
                    let nodes = slots.entry(index).or_default();
                    if nodes.is_empty() {
                        slot_order.push(index);
                    }
                    nodes.push(node);
                }
                Opcode::Phi => phis.push(node),
                _ => {}
            }
        }
        let mut block_of = HashMap::default();
        for (index, block) in self.schedule.blocks.iter().enumerate() {
            for &node in &block.chain {
                block_of.insert(node, index);
            }
        }
        let mut analysis = Analysis {
            exporter: self,
            users,
            block_of,
            returned: HashSet::default(),
        };
        let mut returned_nodes = Vec::new();
        for &phi in &phis {
            if let Some(nodes) = analysis.returned_merge(phi) {
                analysis.returned.insert(phi);
                returned_nodes.extend(nodes);
            }
        }
        let mut candidates: Vec<Candidate> = slot_order
            .into_iter()
            .map(|index| analysis.candidate(Root::Slot, &slots[&index], Vec::new()))
            .collect();
        for (node, lanes) in self.received_roots() {
            candidates.push(analysis.candidate(Root::Received(node), &[node], lanes));
        }
        for phi in phis {
            if analysis.is_address_phi(phi) && !analysis.returned.contains(&phi) {
                candidates.push(analysis.candidate(Root::Merged(phi), &[phi], Vec::new()));
            }
        }
        drop(analysis);
        self.promotion.returned.extend(returned_nodes);
        self.settle_merges(&mut candidates);

        if self.machine.report_storage {
            let mut kept: std::collections::BTreeMap<Kept, usize> =
                std::collections::BTreeMap::new();
            for reason in candidates.iter().filter_map(|candidate| candidate.kept) {
                *kept.entry(reason).or_default() += 1;
            }
            let reasons: Vec<String> = kept
                .iter()
                .map(|(reason, count)| format!("\"{reason}\":{count}"))
                .collect();
            eprintln!(
                "kfrc-storage {{\"symbol\":\"{}\",\"variables\":{},\"memory\":{},\"kept\":{{{}}}}}",
                graph.symbol,
                candidates.len() - kept.values().sum::<usize>(),
                kept.values().sum::<usize>(),
                reasons.join(","),
            );
        }
        for candidate in candidates {
            if candidate.kept.is_some() {
                continue;
            }
            let lanes = partition(&candidate.lanes).expect("settled candidates partition");
            let storage = self.promotion.storages.len();
            let lanes = lanes
                .into_iter()
                .map(|(offset, bytes)| {
                    let ty = integer_type(bytes * 8).expect("partition admits scalar widths");
                    Lane {
                        offset,
                        bytes,
                        variable: self.builder.declare_var(ty),
                    }
                })
                .collect();
            self.promotion.storages.push(lanes);
            match candidate.root {
                Root::Slot => {}
                Root::Received(node) => {
                    self.promotion.received.insert(node, storage);
                }
                Root::Merged(phi) => {
                    self.promotion.merged.insert(phi, storage);
                }
            }
            for (node, offset) in candidate.derived {
                self.promotion.places.insert(node, (storage, offset));
            }
        }
        Ok(())
    }

    /// Resolves the dependencies between merged storage and its inputs: a
    /// merge needs every input to be promoted storage, storage that feeds a
    /// merge needs the merge to be promoted, and every lane read through a
    /// merge is a lane of each input. Candidates that fail are marked kept
    /// until nothing changes.
    fn settle_merges(&self, candidates: &mut [Candidate]) {
        let graph = self.graph;
        let mut owner: HashMap<NodeId, (usize, u64)> = HashMap::default();
        let mut merged: HashMap<NodeId, usize> = HashMap::default();
        for (index, candidate) in candidates.iter().enumerate() {
            for &(node, offset) in &candidate.derived {
                owner.insert(node, (index, offset));
            }
            if let Root::Merged(phi) = candidate.root {
                merged.insert(phi, index);
            }
        }
        // Copying is byte transport. Give each side its complete range once;
        // the sides need no common field types or identical lane boundaries.
        for candidate in candidates
            .iter_mut()
            .filter(|candidate| candidate.kept.is_none())
        {
            for copy in &candidate.copies {
                if let Some(end) = copy.offset.checked_add(copy.bytes) {
                    raw_lanes(copy.offset, end, &mut candidate.lanes);
                } else {
                    candidate.kept = Some("copy range overflow");
                    break;
                }
            }
        }
        loop {
            let mut changed = false;
            for index in 0..candidates.len() {
                if candidates[index].kept.is_some() {
                    continue;
                }
                let feeds_memory = candidates[index].merges.iter().any(|phi| {
                    merged
                        .get(phi)
                        .is_none_or(|&merge| candidates[merge].kept.is_some())
                });
                if feeds_memory {
                    candidates[index].kept = Some("merged with storage in memory");
                    changed = true;
                    continue;
                }
                let Root::Merged(phi) = candidates[index].root else {
                    continue;
                };
                // Lanes are copied into the merge on each incoming edge. The
                // copy stands for the address only if the merge happens at
                // most once and no input is accessed again from the merge
                // block onwards. Every recorded site of an input lies in a
                // predecessor (its last one is the edge itself), so a site
                // in the merge block is an access behind the merge.
                let region = self.schedule.block_of_head[&graph.input(phi, 0)];
                let behind = self.blocks_behind(region);
                let aliased = behind[region]
                    || graph.inputs(phi)[1..].iter().any(|&input| {
                        owner
                            .get(&graph.resolve(input))
                            .is_some_and(|&(source, _)| {
                                candidates[source]
                                    .sites
                                    .iter()
                                    .any(|&site| site == region || behind[site])
                            })
                    });
                if aliased {
                    candidates[index].kept = Some("merge with a live input");
                    changed = true;
                    continue;
                }
                // The merge holds whatever its inputs hold: every lane read
                // through the merge is a lane of each input, and every lane
                // an input writes is a lane of the merge, which a whole-value
                // transfer of the merge then carries.
                let lanes = candidates[index].lanes.clone();
                for &input in &graph.inputs(phi)[1..] {
                    let input = graph.resolve(input);
                    if graph.op(input) == Opcode::Undefined {
                        continue;
                    }
                    match owner.get(&input) {
                        Some(&(source, base)) if candidates[source].kept.is_none() => {
                            for &(offset, bytes) in &lanes {
                                let lane = (base + offset, bytes);
                                if !candidates[source].lanes.contains(&lane) {
                                    candidates[source].lanes.push(lane);
                                    changed = true;
                                }
                            }
                            let written: Vec<(u64, u8)> = candidates[source]
                                .lanes
                                .iter()
                                .filter(|&&(offset, _)| offset >= base)
                                .map(|&(offset, bytes)| (offset - base, bytes))
                                .collect();
                            for lane in written {
                                if !candidates[index].lanes.contains(&lane) {
                                    candidates[index].lanes.push(lane);
                                    changed = true;
                                }
                            }
                        }
                        _ => {
                            candidates[index].kept = Some("merge of storage in memory");
                            changed = true;
                            break;
                        }
                    }
                }
            }
            for candidate in candidates.iter_mut() {
                if candidate.kept.is_none() && partition(&candidate.lanes).is_none() {
                    candidate.kept = Some("invalid byte ranges");
                    changed = true;
                }
            }
            if !changed {
                return;
            }
        }
    }

    /// Blocks that control can reach after entering `block`; `block` itself
    /// is among them exactly when it lies on a cycle.
    fn blocks_behind(&self, block: usize) -> Vec<bool> {
        let blocks = &self.schedule.blocks;
        let mut behind = vec![false; blocks.len()];
        let mut work = blocks[block].successors.clone();
        while let Some(next) = work.pop() {
            if !behind[next] {
                behind[next] = true;
                work.extend(blocks[next].successors.iter().copied());
            }
        }
        behind
    }

    /// Composite parameters and call results that arrive in register lanes,
    /// with the lanes the arrival writes.
    fn received_roots(&self) -> Vec<(NodeId, Vec<(u64, u8)>)> {
        let graph = self.graph;
        let mut roots = Vec::new();
        let mut consider = |node: Option<NodeId>, mode: &PassMode| {
            let Some(node) = node else { return };
            if !matches!(graph.value_type(node), ValueType::LayoutValue(_)) {
                return;
            }
            if let Some(lanes) = transfer_lanes(self, mode, node) {
                roots.push((node, lanes));
            }
        };
        for (index, mode) in graph.abi.args.iter().enumerate() {
            consider(self.projection(graph.start, 2 + index as u32), mode);
        }
        for block in &self.schedule.blocks {
            for &node in &block.chain {
                let abi = match graph.op(node) {
                    Opcode::Call => &graph.callees[graph.payload(node) as usize].abi,
                    Opcode::CallIndirect => &graph.call_abis[graph.payload(node) as usize],
                    _ => continue,
                };
                consider(self.projection(node, 2), &abi.ret);
            }
        }
        roots
    }
}
