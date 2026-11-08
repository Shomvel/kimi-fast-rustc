# Input: [ {cat, secs} ] from phase-categories.jq. Output: markdown table rows.
# work = everything except "wait" (coordinator waiting) and "profiler";
# front end = fe_* + md_* (parse … borrowck, MIR, metadata read/write).
def r1: (. * 10 | round) / 10;
def pct($x; $t): if $t == 0 then "–" else (($x / $t * 1000 | round) / 10 | tostring) + "%" end;
(map(select(.cat != "wait" and .cat != "profiler")) ) as $w
| ($w | map(.secs) | add) as $work
| ($w | map(select(.cat | test("^(fe|md)_"))) | map(.secs) | add // 0) as $fe
| ["| 类别 | 秒（累计） | 占全部工作 | 占前端 |", "|---|---:|---:|---:|"]
  + ([ "fe_parse_expand", "fe_resolve_lower", "fe_typeck", "fe_trait", "fe_borrowck", "fe_mir_build",
       "fe_mir_opt", "fe_const_eval", "fe_layout", "fe_lint", "fe_incr_overhead", "fe_other",
       "md_decode", "md_encode", "be_mono", "be_codegen_ir", "be_llvm", "be_link" ]
     | map(. as $c | ($w | map(select(.cat == $c)) | .[0].secs // 0) as $s
           | "| \($c) | \($s | r1) | \(pct($s; $work)) | \(if ($c | test("^(fe|md)_")) then pct($s; $fe) else "" end) |"))
  + [ "| **前端合计** | \($fe | r1) | \(pct($fe; $work)) | 100% |",
      "| **全部工作** | \($work | r1) | 100% | |" ]
| .[]
