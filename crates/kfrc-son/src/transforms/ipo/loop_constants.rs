//! A bounded profitability hint, never a loop or memory legality proof.
//!
//! Discover cycles only in the immutable original caller. Nested calls inherit
//! their original site's hint, so inlining cannot invalidate an analysis used
//! for correctness. Ordinary plan checks still validate every copied object.
use crate::analysis::cfg::ControlFlow;
use crate::{FunctionGraph, NodeId};
use kfrc_body::fx_hash::{HashMap, HashSet};

pub(super) struct LoopConstants {
    enabled: bool,
    bytes_left: usize,
    fuel: usize,
    pub work: usize,
    cfg: Option<ControlFlow>,
    origins: HashMap<NodeId, NodeId>,
    cyclic: HashMap<NodeId, bool>,
}

impl LoopConstants {
    pub fn new(bytes: usize, fuel: usize) -> Self {
        Self {
            enabled: bytes > 0,
            bytes_left: bytes,
            fuel,
            work: 0,
            cfg: None,
            origins: HashMap::default(),
            cyclic: HashMap::default(),
        }
    }

    fn pay(&mut self, amount: usize) -> bool {
        let paid = amount.min(self.fuel);
        self.fuel -= paid;
        self.work += paid;
        paid == amount
    }

    pub fn remaining(&mut self, original: &FunctionGraph, call: NodeId) -> Option<usize> {
        if !self.enabled {
            return None;
        }
        let origin = self.origins.get(&call).copied().unwrap_or(call);
        if let Some(&cyclic) = self.cyclic.get(&origin) {
            return cyclic.then_some(self.bytes_left);
        }
        if self.cfg.is_none() {
            // Charge the whole indexing input before allocating the CFG.
            // len() also accounts for dead slots skipped by ids().
            if !self.pay(original.len()) {
                return None;
            }
            for node in original.ids() {
                if !self.pay(original.inputs(node).len()) {
                    return None;
                }
            }
            self.cfg = Some(ControlFlow::new(original));
        }
        let mut visited = HashSet::default();
        let mut pending = vec![origin];
        while let Some(node) = pending.pop() {
            if !visited.insert(node) {
                continue;
            }
            let cost = 1usize.saturating_add(self.cfg.as_ref()?.successors(node).len());
            if !self.pay(cost) {
                return None;
            }
            for &next in self.cfg.as_ref()?.successors(node) {
                if next == origin {
                    self.cyclic.insert(origin, true);
                    return Some(self.bytes_left);
                }
                pending.push(next);
            }
        }
        self.cyclic.insert(origin, false);
        None
    }

    pub fn inherit(&mut self, parent: NodeId, nested: NodeId) {
        if self.enabled {
            let origin = self.origins.get(&parent).copied().unwrap_or(parent);
            self.origins.insert(nested, origin);
        }
    }

    pub fn imported(&mut self, bytes: usize) {
        self.bytes_left -= bytes;
    }
}
