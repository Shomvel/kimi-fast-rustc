//! Close the signature-editing boundary over semantic and machine-only inputs.
use super::{CompileError, CrateGraphs};
use crate::emission::CguJob;
use crate::machine::CompiledFunction;
use kfrc_object::SymbolBinding;
use std::collections::{BTreeMap, BTreeSet};

impl CrateGraphs {
    pub(crate) fn promote_arguments(
        &mut self,
        jobs: &[CguJob],
        adapters: &BTreeMap<String, CompiledFunction>,
    ) -> Result<(), CompileError> {
        // A mono instance can have several CGU copies. Its ABI is private only
        // when every emitted copy is local, with every caller represented.
        let mut private = self.internal.clone();
        for job in jobs {
            for (symbol, binding) in &job.pending_functions {
                if !matches!(binding, SymbolBinding::Local) {
                    private.remove(symbol);
                }
            }
        }
        self.internal = private.clone();
        let mut outside = BTreeSet::new();
        outside.extend(self.data.objects.values().flat_map(|object| {
            object
                .relocations
                .iter()
                .map(|relocation| relocation.target.clone())
        }));
        for job in jobs {
            crate::emission::data_references(&job.data_objects, &mut outside);
        }
        for (symbol, (machine, data)) in adapters {
            outside.insert(symbol.clone());
            crate::emission::function_references(machine, &mut outside);
            crate::emission::data_references(data, &mut outside);
        }
        private.retain(|symbol| !outside.contains(symbol));
        if std::env::var("KFRC_OPTIMIZE").as_deref() == Ok("0")
            || std::env::var("KFRC_ARGUMENT_PROMOTION").as_deref() == Ok("0")
        {
            return Ok(());
        }
        let stats = kfrc_son::promote_arguments(&mut self.bodies, &private).map_err(|error| {
            CompileError {
                symbol: error.symbol,
                message: error.error.to_string(),
            }
        })?;
        if std::env::var_os("KFRC_SPECIALIZATION_STATS").is_some()
            || std::env::var_os("KFRC_DUMP_GRAPH").is_some()
        {
            eprintln!("kfrc argument promotion: {stats:?}");
        }
        self.collect_references();
        Ok(())
    }
}
