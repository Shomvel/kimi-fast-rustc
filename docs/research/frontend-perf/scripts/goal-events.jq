# Input: one crox chrome_profiler.json (array of trace events). Arg: --arg crate NAME.
# Output: TSV lines "tid ts dur label key crate" for every complete event, sorted later.
# key is the query key (args.arg0) only for trait/type-op queries, "-" otherwise; tabs and
# newlines in keys are replaced by spaces. Durations are microseconds.
def goal_label: test("^(evaluate_obligation|type_op_|normalize_canonicalized_|implied_outlives_bounds|dropck_outlives|method_autoderef_steps|is_copy_raw|is_sized_raw|is_freeze_raw|is_unpin_raw|needs_drop_raw|resolve_instance_raw)");
.[] | select(.ph == "X")
| [ (.tid | tostring), (.ts | tostring), (.dur | tostring), .name,
    (if (.name | goal_label) then ((.args.arg0 // "-") | gsub("[\t\n]"; " ")) else "-" end), $crate ]
| @tsv
