# Input (slurped): [ {name, cats: [ {cat, secs} ]} ] — one entry per build.
# Output: markdown matrix, one column per build: share of the front end per category, then the
# front end's seconds and share of all work (work excludes "wait" and "profiler").
def pct($x; $t): if $t == 0 then "–" else (($x / $t * 1000 | round) / 10 | tostring) + "%" end;
def r1: if . < 1 then (. * 100 | round) / 100 else (. * 10 | round) / 10 end;
def fe: test("^(fe|md)_");
map(. + { work: ([.cats[] | select(.cat != "wait" and .cat != "profiler") | .secs] | add),
          fesum: ([.cats[] | select(.cat | fe) | .secs] | add) }) as $b
| ([ "fe_parse_expand", "fe_resolve_lower", "fe_typeck", "fe_trait", "fe_borrowck", "fe_mir_build", "fe_mir_opt",
     "fe_const_eval", "fe_layout", "fe_lint", "fe_incr_overhead", "fe_other", "md_decode", "md_encode" ]) as $rows
| "| 占前端 | " + ($b | map(.name) | join(" | ")) + " |",
  "|---|" + ($b | map("---:") | join("|")) + "|",
  ( $rows[] as $c
    | "| \($c) | " + ($b | map(. as $x | pct(([$x.cats[] | select(.cat == $c) | .secs] | add // 0); $x.fesum)) | join(" | ")) + " |" ),
  "| **前端秒（累计）** | " + ($b | map(.fesum | r1 | tostring) | join(" | ")) + " |",
  "| **前端占全部工作** | " + ($b | map(pct(.fesum; .work)) | join(" | ")) + " |"
