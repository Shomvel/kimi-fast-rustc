//! One immutable R graph per eligible definition, shared by its instances.
//! Definition eligibility is explicit and cached; instance eligibility (the
//! concrete callees of its call sites) is decided before specialization. Once
//! specialization starts, an error is fatal; it never retries the mono path.

use kfrc_body::fx_hash::HashMap;
use std::collections::BTreeMap;

use rustc_middle::ty::{Instance, InstanceKind, TyCtxt};
use rustc_span::def_id::DefId;

use crate::convert::definition::{self, Definition, Eligibility};
use crate::profile;

#[derive(Default)]
pub(crate) struct Definitions<'tcx> {
    entries: HashMap<DefId, Eligibility<Definition<'tcx>>>,
    definitions_built: usize,
    instances_specialized: usize,
    holes_answered: usize,
    branches_pruned: usize,
    nodes_copied: usize,
    mono_reasons: BTreeMap<String, usize>,
}

impl<'tcx> Definitions<'tcx> {
    pub(crate) fn specialize(
        &mut self,
        tcx: TyCtxt<'tcx>,
        instance: Instance<'tcx>,
    ) -> Result<Option<kfrc_son::specialize::Specialization>, String> {
        let InstanceKind::Item(def_id) = instance.def else {
            *self
                .mono_reasons
                .entry("compiler-generated shim".into())
                .or_default() += 1;
            return Ok(None);
        };
        if let std::collections::hash_map::Entry::Vacant(entry) = self.entries.entry(def_id) {
            let name = tcx.def_path_str(def_id);
            let definition = profile::measure("prepare_r", &name, || {
                definition::convert_definition(tcx, def_id)
            })?;
            if let Eligibility::Eligible(definition) = &definition {
                self.definitions_built += 1;
                if std::env::var_os("KFRC_SPECIALIZATION_STATS").is_some() {
                    eprintln!(
                        "kfrc R prepared {name}: count=1 stats={:?}",
                        definition.prepared.preparation_stats()
                    );
                }
                if std::env::var_os("KFRC_DUMP_GRAPH").is_some() {
                    eprintln!(
                        "kfrc definition {name}:\n{}",
                        definition.prepared.graph().dump()
                    );
                }
            }
            entry.insert(definition);
        }
        let definition = match &self.entries[&def_id] {
            Eligibility::Eligible(definition) => definition,
            Eligibility::NotEligible(reason) => {
                *self.mono_reasons.entry(reason.clone()).or_default() += 1;
                return Ok(None);
            }
        };
        if let Eligibility::NotEligible(reason) =
            definition::instance_eligibility(tcx, definition, instance)
        {
            *self.mono_reasons.entry(reason).or_default() += 1;
            return Ok(None);
        }
        let symbol = tcx.symbol_name(instance).name.to_string();
        let specialized = profile::measure("specialize_r", &symbol, || {
            definition::specialize(tcx, definition, instance)
        })?;
        self.instances_specialized += 1;
        self.holes_answered += specialized.stats.holes_answered;
        self.branches_pruned += specialized.stats.branches_pruned;
        self.nodes_copied += specialized.stats.nodes_copied;
        if std::env::var_os("KFRC_DUMP_GRAPH").is_some()
            || std::env::var_os("KFRC_SPECIALIZATION_STATS").is_some()
        {
            eprintln!(
                "kfrc specialization {symbol}: {:?}; answers={:?}",
                specialized.stats, specialized.answers
            );
        }
        Ok(Some(specialized))
    }

    pub(crate) fn report(&self) {
        if std::env::var_os("KFRC_DUMP_GRAPH").is_some()
            || std::env::var_os("KFRC_SPECIALIZATION_STATS").is_some()
        {
            eprintln!(
                "kfrc R coverage: definitions_built={} instances_specialized={} holes_answered={} branches_pruned={} nodes_copied={} mono_reasons={:?}",
                self.definitions_built,
                self.instances_specialized,
                self.holes_answered,
                self.branches_pruned,
                self.nodes_copied,
                self.mono_reasons,
            );
        }
    }
}
