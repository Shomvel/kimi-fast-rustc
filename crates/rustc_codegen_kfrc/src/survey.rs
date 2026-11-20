//! Opt-in capability survey. Never emits objects or reports compilation success.
//! Each mono item is attempted independently; a failure masks later failures
//! within that item, not later functions. Crate-wide transforms are not tested.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;

use rustc_hir::def_id::LOCAL_CRATE;
use rustc_middle::mir;
use rustc_middle::mir::mono::MonoItem;
use rustc_middle::ty::{self, Instance, TyCtxt};

use crate::{codegen, constants, definitions, permissions, shell_entry};

pub(crate) fn run(tcx: TyCtxt<'_>, directory: &Path) -> ! {
    let name = tcx.crate_name(LOCAL_CRATE);
    let report = (|| -> std::io::Result<_> {
        fs::create_dir_all(directory)?;
        let path = directory.join(format!("{name}-{}.jsonl", std::process::id()));
        let file = File::options().write(true).create_new(true).open(&path)?;
        Ok((path, BufWriter::new(file)))
    })();
    let (path, mut report) = report.unwrap_or_else(|error| {
        tcx.dcx()
            .fatal(format!("kfrc survey: creating report: {error}"))
    });
    let mut record =
        |kind: &str, symbol: &str, item: &str, status: &str, message: &str| {
            writeln!(report,
            "{{\"kind\":{},\"crate\":{},\"symbol\":{},\"item\":{},\"status\":{},\"message\":{}}}",
            json(kind), json(name.as_str()), json(symbol), json(item), json(status), json(message)
        ).and_then(|()| report.flush()).unwrap_or_else(|error| {
            tcx.dcx().fatal(format!("kfrc survey: writing report: {error}"))
        });
        };
    record(
        "begin",
        "",
        "",
        "incomplete",
        "one first failure per mono item; isolated function lowering; no crate-wide transforms or linking",
    );
    let partition = tcx.collect_and_partition_mono_items(());
    let entry_result =
        rustc_middle::ty::print::with_no_trimmed_paths!(shell_entry::UnitEntryPlan::try_new(tcx));
    let entry_failed = entry_result.is_err();
    let entry = match entry_result {
        Ok(entry) => entry,
        Err(message) => {
            record("entry", "", "executable runtime adapter", "error", &message);
            None
        }
    };
    let mut definitions = definitions::Definitions::default();
    let mut permissions = permissions::PermissionTotals::default();
    let empty = codegen::CrateGraphs::default();
    let provider = codegen::CrateProvider { all: &empty };
    let mut seen = BTreeSet::new();
    let mut total = usize::from(entry_failed);
    let mut failures = usize::from(entry_failed);
    for cgu in partition.codegen_units {
        for (item, _) in cgu.items_in_deterministic_order(tcx) {
            let (kind, symbol, label, result) = match item {
                MonoItem::Fn(instance) => {
                    let symbol = tcx.symbol_name(instance).name.to_owned();
                    if !seen.insert(symbol.clone()) {
                        continue;
                    }
                    let label =
                        rustc_middle::ty::print::with_no_trimmed_paths!(format!("{instance}"));
                    record("function_begin", &symbol, &label, "started", "");
                    // Runtime entry adapters have no ordinary MIR conversion.
                    let result = if entry
                        .as_ref()
                        .and_then(|plan| {
                            rustc_middle::ty::print::with_no_trimmed_paths!(
                                plan.compile(tcx, instance)
                            )
                        })
                        .is_some()
                    {
                        Ok(())
                    } else {
                        // Inventory every MIR intrinsic call independently of
                        // conversion, including calls masked by an earlier gap.
                        // Observation is not a claim that an intrinsic fails.
                        let mir = tcx.instance_mir(instance.def);
                        for block in mir.basic_blocks.iter() {
                            if let mir::TerminatorKind::Call { func, .. } = &block.terminator().kind
                                && let ty::FnDef(def_id, _) = *func.ty(&mir.local_decls, tcx).kind()
                                && tcx.intrinsic(def_id).is_some()
                            {
                                record(
                                    "intrinsic",
                                    &symbol,
                                    &label,
                                    "observed",
                                    tcx.item_name(def_id).as_str(),
                                );
                            }
                        }
                        rustc_middle::ty::print::with_no_trimmed_paths!(
                            crate::try_prepare_instance_input(
                                tcx,
                                instance,
                                &mut definitions,
                                &mut permissions
                            )
                        )
                        .and_then(|job| {
                            // Only the owned graph pipeline is isolated: it
                            // cannot poison rustc queries or the shared R cache.
                            // Do not catch fatal rustc diagnostics/conversion.
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                codegen::prepare_graph(job).and_then(|graph| {
                                    codegen::compile_function(graph, &provider, true)
                                })
                            }))
                            .unwrap_or_else(|payload| {
                                let message = payload
                                    .downcast_ref::<String>()
                                    .map(String::as_str)
                                    .or_else(|| payload.downcast_ref::<&str>().copied())
                                    .unwrap_or("non-string panic payload");
                                Err(codegen::CompileError {
                                    symbol: symbol.clone(),
                                    message: format!(
                                        "compiler panic in owned graph pipeline: {message}"
                                    ),
                                })
                            })
                        })
                        .map(|_| ())
                        .map_err(|error| error.message)
                    };
                    ("function", symbol, label, result)
                }
                MonoItem::Static(def_id) => {
                    let symbol = tcx.symbol_name(Instance::mono(tcx, def_id)).name.to_owned();
                    if !seen.insert(symbol.clone()) {
                        continue;
                    }
                    let mut builder = kfrc_body::BodyBuilder::new(
                        &symbol,
                        kfrc_body::FunctionAbi {
                            args: Vec::new(),
                            ret: kfrc_body::PassMode::Ignore,
                            c_variadic: false,
                        },
                        kfrc_body::RecordedTypes::new(),
                    );
                    let result = constants::ConstantCx::new(tcx, &symbol)
                        .static_data(def_id, kfrc_body::DataBinding::Local, &mut builder)
                        .map(|_| ());
                    ("static", symbol, tcx.def_path_str(def_id), result)
                }
                MonoItem::GlobalAsm(item_id) => {
                    let label = format!("{item_id:?}");
                    if !seen.insert(label.clone()) {
                        continue;
                    }
                    (
                        "global_asm",
                        label.clone(),
                        label,
                        Err("global_asm! is not supported".into()),
                    )
                }
            };
            total += 1;
            let (status, message) = match &result {
                Ok(()) => ("ok", ""),
                Err(message) => {
                    failures += 1;
                    ("error", message.as_str())
                }
            };
            record(kind, &symbol, &label, status, message);
        }
    }
    record(
        "summary",
        "",
        "",
        "complete",
        &format!("items={total} failed={failures}"),
    );
    tcx.dcx().fatal(format!(
        "kfrc survey complete: {failures}/{total} items failed; report {}; diagnostic run emits no artifacts",
        path.display()
    ))
}

fn json(text: &str) -> String {
    let mut output = String::from("\"");
    for ch in text.chars() {
        match ch {
            '\"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            ch if ch < ' ' => {
                use std::fmt::Write;
                write!(&mut output, "\\u{:04x}", ch as u32).unwrap();
            }
            ch => output.push(ch),
        }
    }
    output.push('\"');
    output
}
