//! Address-mode selection for graph loads and stores.
//!
//! A load/store whose address is a chain of `PtrOffset(base, Const)` nodes
//! accesses `[base + displacement]`. The decision is made on the typed graph,
//! so a pointer whose every use is such an access is never materialized.
use super::*;

impl Lowering<'_> {
    /// The base node and imm32 displacement an access to `address` uses.
    pub(super) fn address_mode(&self, address: NodeId) -> (NodeId, i32) {
        let graph = self.graph;
        let mut base = address;
        let mut displacement = 0i32;
        while graph.op(base) == Opcode::PtrOffset {
            let Ok(step) = self.constant(graph.input(base, 1)) else {
                break;
            };
            let Some(next) = i64::from(displacement)
                .checked_add(step)
                .and_then(|value| i32::try_from(value).ok())
            else {
                break;
            };
            displacement = next;
            base = graph.input(base, 0);
        }
        (base, displacement)
    }

    /// `PtrOffset` nodes whose every use is a load/store address that folds
    /// past them, directly or through another folded `PtrOffset`.
    pub(super) fn address_only_offsets(&self) -> HashSet<NodeId> {
        let graph = self.graph;
        let mut users: HashMap<NodeId, Vec<(NodeId, usize)>> = HashMap::new();
        let mut control_users = HashSet::new();
        for node in graph.ids() {
            for (slot, &input) in graph.inputs(node).iter().enumerate() {
                users.entry(graph.resolve(input)).or_default().push((node, slot));
            }
            let control = graph.control(node);
            if !control.is_none() {
                control_users.insert(graph.resolve(control));
            }
        }
        let mut memo = HashMap::new();
        let mut folded = HashSet::new();
        for node in graph.ids() {
            if graph.op(node) == Opcode::PtrOffset
                && self.folds_away(node, &users, &control_users, &mut memo)
            {
                folded.insert(node);
            }
        }
        folded
    }

    fn folds_away(
        &self,
        node: NodeId,
        users: &HashMap<NodeId, Vec<(NodeId, usize)>>,
        control_users: &HashSet<NodeId>,
        memo: &mut HashMap<NodeId, bool>,
    ) -> bool {
        if let Some(&known) = memo.get(&node) {
            return known;
        }
        let graph = self.graph;
        let passes = |from: NodeId| {
            // Walking from `from` must consume `node` itself.
            let (base, _) = self.address_mode(from);
            let mut cursor = from;
            while cursor != base {
                if cursor == node {
                    return true;
                }
                cursor = graph.input(cursor, 0);
            }
            false
        };
        let result = !control_users.contains(&node)
            && users.get(&node).is_some_and(|uses| {
                !uses.is_empty()
                    && uses.iter().all(|&(user, slot)| match graph.op(user) {
                        Opcode::Load => slot == 1 && passes(node),
                        Opcode::Store => slot == 1 && passes(node),
                        Opcode::PtrOffset => {
                            slot == 0
                                && self.folds_away(user, users, control_users, memo)
                                && passes(user)
                        }
                        _ => false,
                    })
            });
        memo.insert(node, result);
        result
    }
}
