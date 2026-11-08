# Map rustc self-profile labels (events generic-activity,query-provider) to phase categories.
# Input: one summaries.json = { "<crate>-<pid>": { "<label>": {time, self_time, calls} } }.
# Defines category(label); used by phase-categories.jq and top-labels.jq.
# Ported from the categorization used for the 2026-10-10 E2B release profiles, with the MIR
# bucket split into "build" (needed by borrowck/const eval) and "opt" (only for codegen/metadata).

def pfx($l; $xs): any($xs[]; . as $p | $l | startswith($p));
def isin($l; $xs): any($xs[]; . == $l);

def category($l):
  if isin($l; ["LLVM_thinlto", "LLVM_passes", "LLVM_fatlto", "finish_ongoing_codegen", "codegen_crate",
                 "join_worker_thread", "drop_compiler", "drop_dep_graph"]) then "wait"
  elif pfx($l; ["LLVM_", "codegen_module_optimize", "codegen_module_perform_lto", "target_machine_factory",
              "codegen_thin_generate_lto_work", "lto_generate_symbols_below_threshold"]) then "be_llvm"
  elif isin($l; ["codegen_module", "fn_abi_of_instance", "codegen_fn_attrs", "symbol_name", "codegen_select_candidate",
                 "compute_debuginfo_type_name"]) then "be_codegen_ir"
  elif pfx($l; ["monomorphization_collector", "cgu_partitioning"])
       or isin($l; ["collect_and_partition_mono_items", "items_of_instance", "mono_instance_mir",
                    "assert_symbols_are_distinct", "size_estimate", "partition_and_assert_distinct_symbols"]) then "be_mono"
  elif isin($l; ["run_linker", "link_crate", "link_binary", "link_rlib", "link_altering_rlib",
                 "link_binary_check_files_are_writeable", "link_binary_remove_temps"]) then "be_link"
  elif isin($l; ["generate_crate_metadata", "write_crate_metadata", "generate_crate_metadata_stub"])
       or pfx($l; ["metadata_encode"]) then "md_encode"
  elif isin($l; ["parse_crate", "expand_crate", "pre_AST_expansion_lint_checks", "AST_validation",
                 "configure_and_expand", "macro_expand_crate", "setup_global_ctxt"])
       or pfx($l; ["expand_"]) then "fe_parse_expand"
  elif pfx($l; ["incr_comp_"]) then "fe_incr_overhead"
  elif isin($l; ["hir_crate", "late_resolve_crate", "finalize_imports", "drop_ast", "resolver_for_lowering_raw",
                 "lower_to_hir", "hir_crate_items", "hir_module_items", "opt_hir_owner_nodes", "resolve_bound_vars",
                 "named_variable_map", "hir_attr_map", "resolutions"])
       or pfx($l; ["hir_"]) then "fe_resolve_lower"
  elif isin($l; ["evaluate_obligation", "normalize_canonicalized_projection", "implied_outlives_bounds",
                 "dropck_outlives", "is_copy_raw", "is_sized_raw", "is_freeze_raw", "needs_drop_raw",
                 "resolve_instance_raw", "try_normalize_generic_arg_after_erasing_regions",
                 "specialization_graph_of", "trait_impls_of", "method_autoderef_steps",
                 "normalize_canonicalized_inherent_projection", "normalize_canonicalized_free_alias"])
       or pfx($l; ["type_op_", "evaluate_", "normalize_"]) then "fe_trait"
  elif isin($l; ["mir_borrowck", "MIR_borrow_checking", "check_liveness"]) then "fe_borrowck"
  elif isin($l; ["typeck", "type_check_crate", "check_well_formed", "compare_impl_item", "fn_sig", "type_of",
                 "predicates_of", "generics_of", "param_env", "associated_items", "impl_trait_header",
                 "impl_super_outlives", "opaque_types_defined_by", "region_scope_tree", "variances_of", "adt_def",
                 "coherent_trait", "inherent_impls", "crate_inherent_impls", "check_match", "associated_item",
                 "type_of_opaque", "collect_return_position_impl_trait_in_trait_tys"])
       or pfx($l; ["explicit_", "check_mod_", "inferred_outlives", "crate_inherent_impls", "typeck_"]) then "fe_typeck"
  elif isin($l; ["mir_built", "thir_body", "check_unsafety", "mir_promoted", "mir_const_qualif", "promoted_mir",
                 "mir_keys", "thir_abstract_const", "build_mir", "mir_for_ctfe"])
       or pfx($l; ["thir_"]) then "fe_mir_build"
  elif isin($l; ["optimized_mir", "mir_drops_elaborated_and_const_checked", "mir_callgraph_cyclic",
                 "mir_inliner_callees", "cross_crate_inlinable", "deduced_param_attrs", "mir_shims",
                 "check_mono_item", "mir_coroutine_witnesses", "coroutine_layout"])
       or pfx($l; ["mir_pass_"]) then "fe_mir_opt"
  elif pfx($l; ["eval_to_", "eval_static", "const_", "valtree_", "lit_to_const"]) then "fe_const_eval"
  elif pfx($l; ["layout_of", "naive_layout_of", "fn_abi_of_fn_ptr"]) then "fe_layout"
  elif pfx($l; ["metadata_decode_entry", "metadata_register_crate", "metadata_load_macro", "crate_loading", "metadata_"])
       or isin($l; ["visible_parent_map", "crate_hash", "crate_name", "used_crates", "externs"]) then "md_decode"
  elif ($l | contains("lint"))
       or isin($l; ["live_symbols_and_ignored_derived_traits", "MIR_effect_checking", "check_private_in_public",
                    "effective_visibilities"]) then "fe_lint"
  elif pfx($l; ["self_profile"]) or isin($l; ["query_key_hash_verify_for", "get_apple_sdk_root"]) then "profiler"
  else "fe_other" end;

