//! Crate-owned definitions. Graphs carry symbol references, not copies of data.
//!
//! Anonymous immutable allocations may be emitted locally in each using CGU.
//! Every other allocation has one owner, preserving mutable state and identity.

use crate::codegen::CompileError;
use kfrc_body::{DataBinding, DataObject, EncodedFunction};
use kfrc_son::FunctionGraph;
use rustc_data_structures::fingerprint::Fingerprint;
use rustc_data_structures::stable_hasher::StableHasher;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::Hash;

pub(crate) type Aliases = BTreeMap<String, String>;

pub(crate) fn anonymous(object: &DataObject) -> bool {
    object.binding == DataBinding::Local
        && object.unnamed_address
        && !object.writable
        && !object.thread_local
}

#[derive(Clone, Default)]
pub(crate) struct ProgramData {
    pub objects: BTreeMap<String, DataObject>,
    pub owners: BTreeMap<String, usize>,
}

impl ProgramData {
    pub fn insert(&mut self, object: DataObject, owner: usize) -> Result<(), CompileError> {
        let symbol = object.symbol.clone();
        if let Some(old) = self.objects.get(&symbol) {
            if old != &object {
                return Err(CompileError {
                    symbol: symbol.clone(),
                    message: format!("conflicting definitions of data `{symbol}`"),
                });
            }
        } else {
            self.objects.insert(symbol.clone(), object);
        }
        self.owners
            .entry(symbol)
            .and_modify(|old| *old = (*old).min(owner))
            .or_insert(owner);
        Ok(())
    }

    /// Intern leaves before their users, so nested Location/vtable/literal
    /// allocations compare canonical relocation targets as well as bytes.
    /// Cyclic allocations retain their existing symbols and relocation cycle;
    /// no recursive expansion or guess about pointer identity is needed.
    pub fn canonicalize(&mut self) -> Aliases {
        let mut remaining = BTreeMap::new();
        let mut users = BTreeMap::<String, Vec<String>>::new();
        for (symbol, object) in &self.objects {
            if !anonymous(object) {
                continue;
            }
            let dependencies: BTreeSet<_> = object
                .relocations
                .iter()
                .filter(|relocation| self.objects.get(&relocation.target).is_some_and(anonymous))
                .map(|relocation| relocation.target.clone())
                .collect();
            remaining.insert(symbol.clone(), dependencies.len());
            for dependency in dependencies {
                users.entry(dependency).or_default().push(symbol.clone());
            }
        }
        let mut ready: BTreeSet<_> = remaining
            .iter()
            .filter_map(|(symbol, &count)| (count == 0).then_some(symbol.clone()))
            .collect();
        let mut aliases = Aliases::new();
        let mut interned = BTreeMap::<String, DataObject>::new();
        while let Some(symbol) = ready.pop_first() {
            let mut object = self.objects[&symbol].clone();
            rewrite_data(&mut object, &aliases);
            // Hashes name objects; equality of the complete contents licenses
            // merging. A hash collision never silently merges unequal data.
            let mut hasher = StableHasher::new();
            object.bytes.hash(&mut hasher);
            object.align.hash(&mut hasher);
            for relocation in &object.relocations {
                relocation.offset.hash(&mut hasher);
                relocation.target.hash(&mut hasher);
                relocation.addend.hash(&mut hasher);
            }
            let hash: Fingerprint = hasher.finish();
            let prefix = format!(".Lkfrc_const_{hash}");
            let mut name = prefix.clone();
            let mut collision = 0;
            loop {
                object.symbol = name.clone();
                match interned.get(&name).or_else(|| self.objects.get(&name)) {
                    Some(old) if old != &object => {
                        collision += 1;
                        name = format!("{prefix}_{collision}");
                    }
                    _ => break,
                }
            }
            aliases.insert(symbol.clone(), name.clone());
            interned.entry(name).or_insert(object);
            for user in users.get(&symbol).into_iter().flatten() {
                let count = remaining.get_mut(user).expect("constant user is indexed");
                *count -= 1;
                if *count == 0 {
                    ready.insert(user.clone());
                }
            }
        }
        let old = std::mem::take(&mut self.objects);
        let old_owners = std::mem::take(&mut self.owners);
        for (symbol, mut object) in old {
            let name = aliases.get(&symbol).unwrap_or(&symbol).clone();
            if let Some(canonical) = interned.remove(&name) {
                object = canonical;
            } else {
                rewrite_data(&mut object, &aliases);
            }
            object.symbol = name.clone();
            self.objects.entry(name.clone()).or_insert(object);
            let owner = old_owners[&symbol];
            self.owners
                .entry(name)
                .and_modify(|old| *old = (*old).min(owner))
                .or_insert(owner);
        }
        aliases
    }

    /// Include function addresses reached through live data relocations.
    pub fn expand_references(&self, references: &mut BTreeSet<String>) {
        let mut pending = references.clone();
        let mut visited = BTreeSet::new();
        while let Some(symbol) = pending.pop_first() {
            if !visited.insert(symbol.clone()) {
                continue;
            }
            if let Some(object) = self.objects.get(&symbol) {
                for relocation in &object.relocations {
                    references.insert(relocation.target.clone());
                    pending.insert(relocation.target.clone());
                }
            }
        }
    }
}

pub(crate) fn rewrite_name(name: &mut String, aliases: &Aliases) {
    if let Some(canonical) = aliases.get(name) {
        *name = canonical.clone();
    }
}

pub(crate) fn rewrite_names(names: &mut BTreeSet<String>, aliases: &Aliases) {
    *names = std::mem::take(names)
        .into_iter()
        .map(|mut name| {
            rewrite_name(&mut name, aliases);
            name
        })
        .collect();
}

pub(crate) fn rewrite_graph(graph: &mut FunctionGraph, aliases: &Aliases) {
    for (symbol, _) in &mut graph.symbols {
        rewrite_name(symbol, aliases);
    }
    for callee in &mut graph.callees {
        rewrite_name(&mut callee.symbol, aliases);
    }
}

pub(crate) fn rewrite_code(function: &mut EncodedFunction, aliases: &Aliases) {
    for relocation in &mut function.relocations {
        rewrite_name(&mut relocation.target, aliases);
    }
}

fn rewrite_data(object: &mut DataObject, aliases: &Aliases) {
    for relocation in &mut object.relocations {
        rewrite_name(&mut relocation.target, aliases);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kfrc_body::DataRelocation;

    fn data(symbol: &str, target: Option<&str>) -> DataObject {
        DataObject {
            symbol: symbol.into(),
            bytes: vec![0; 8],
            align: 8,
            unnamed_address: true,
            relocations: target
                .map(|target| DataRelocation {
                    offset: 0,
                    target: target.into(),
                    addend: 0,
                })
                .into_iter()
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn nested_constants_share_contents_and_preserve_relocation_addends() {
        let mut pool = ProgramData::default();
        for (owner, prefix) in [(1, "first"), (0, "second")] {
            pool.insert(data(&format!("{prefix}_leaf"), None), owner)
                .unwrap();
            pool.insert(
                data(&format!("{prefix}_root"), Some(&format!("{prefix}_leaf"))),
                owner,
            )
            .unwrap();
        }
        let mut other = data("addend", Some("first_leaf"));
        other.relocations[0].addend = 1;
        pool.insert(other, 0).unwrap();
        let aliases = pool.canonicalize();
        assert_eq!(aliases["first_leaf"], aliases["second_leaf"]);
        assert_eq!(aliases["first_root"], aliases["second_root"]);
        assert_ne!(aliases["first_root"], aliases["addend"]);
        assert_eq!(pool.objects.len(), 3);
        let before = pool.objects.clone();
        pool.canonicalize();
        assert_eq!(pool.objects, before);
    }

    #[test]
    fn identity_mutability_tls_alignment_and_cycles_are_preserved() {
        let mut pool = ProgramData::default();
        for object in [
            data("plain", None),
            DataObject {
                align: 16,
                ..data("aligned", None)
            },
            DataObject {
                unnamed_address: false,
                ..data("identity", None)
            },
            DataObject {
                writable: true,
                ..data("mutable", None)
            },
            DataObject {
                thread_local: true,
                ..data("tls", None)
            },
            data("cycle_a", Some("cycle_b")),
            data("cycle_b", Some("cycle_a")),
        ] {
            pool.insert(object, 0).unwrap();
        }
        let aliases = pool.canonicalize();
        assert_eq!(pool.objects.len(), 7);
        assert_ne!(aliases["plain"], aliases["aligned"]);
        for name in ["identity", "mutable", "tls", "cycle_a", "cycle_b"] {
            assert!(!aliases.contains_key(name));
            assert!(pool.objects.contains_key(name));
        }
        let mut references = BTreeSet::from(["cycle_a".into()]);
        pool.expand_references(&mut references);
        assert_eq!(
            references,
            BTreeSet::from(["cycle_a".into(), "cycle_b".into()])
        );
    }
}
