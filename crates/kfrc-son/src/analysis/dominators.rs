//! Immediate dominance over a reachable control-flow snapshot.
use super::cfg::ControlFlow;
use crate::graph::NodeId;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Dominators {
    idom: Arc<[usize]>,
}

impl Dominators {
    pub fn new(cfg: &ControlFlow) -> Self {
        cfg.dominators()
    }

    pub(super) fn build(cfg: &ControlFlow) -> Self {
        let mut idom = vec![usize::MAX; cfg.order.len()];
        if idom.is_empty() {
            return Self { idom: idom.into() };
        }
        idom[0] = 0;
        let mut changed = true;
        while changed {
            changed = false;
            for index in 1..cfg.order.len() {
                let mut new_idom = usize::MAX;
                for &predecessor in &cfg.predecessors[index] {
                    if idom[predecessor] == usize::MAX {
                        continue;
                    }
                    new_idom = if new_idom == usize::MAX {
                        predecessor
                    } else {
                        intersect(&idom, predecessor, new_idom)
                    };
                }
                if new_idom != idom[index] {
                    idom[index] = new_idom;
                    changed = true;
                }
            }
        }
        Self { idom: idom.into() }
    }

    pub fn dominates_position(&self, a: usize, mut b: usize) -> bool {
        loop {
            if a == b {
                return true;
            }
            if b == 0 {
                return false;
            }
            b = self.idom[b];
        }
    }

    pub fn dominates(&self, cfg: &ControlFlow, a: NodeId, b: NodeId) -> bool {
        match (cfg.position.get(&a), cfg.position.get(&b)) {
            (Some(&a), Some(&b)) => self.dominates_position(a, b),
            _ => false,
        }
    }
}

fn intersect(idom: &[usize], mut a: usize, mut b: usize) -> usize {
    while a != b {
        while a > b {
            a = idom[a];
        }
        while b > a {
            b = idom[b];
        }
    }
    a
}
