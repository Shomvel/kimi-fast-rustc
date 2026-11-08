# Critical path of a Cargo build from its --timings UNIT_DATA array.
# Input: UNIT_DATA (array of units with i, name, mode, target, start, duration, unblocked_units,
# unblocked_rmeta_units, sections). A unit's "frontend" section ends when its rmeta is written.
# Walks back from the last unit to finish: each unit's critical predecessor is the latest
# event (rmeta of a unit listing it in unblocked_rmeta_units, or completion of a unit listing it
# in unblocked_units). Prints the path and the split of its length into front end, codegen,
# build-script runs, unsplit rustc units (no sections, e.g. check builds) and waiting gaps.
def fe_len: ((.sections // []) | map(select(.[0] == "frontend")) | .[0][1].end) // null;
def r2: (. * 100 | round) / 100;
def unit_name: "\(.name)\(if .target == "" then "" else " " + .target end)\(if .mode == "run-custom-build" then " (build script run)" else "" end)";
. as $units
| (map({key: (.i | tostring), value: .}) | from_entries) as $by
| def preds($v):
    [ $units[] as $u
      | ( if (($u.unblocked_rmeta_units // []) | index($v.i)) != null
          then {u: $u, kind: "rmeta", t: ($u.start + ($u | fe_len // $u.duration))} else empty end ),
        ( if (($u.unblocked_units // []) | index($v.i)) != null
          then {u: $u, kind: "full", t: ($u.start + $u.duration)} else empty end ) ];
  def walk($v; $via):
    ( {unit: $v, via: $via} ),
    ( preds($v) | if length == 0 then empty else (max_by(.t)) as $p
        | {gap: ($v.start - $p.t)}, walk($p.u; $p.kind) end );
  ($units | max_by(.start + .duration)) as $last
| [ walk($last; "end") ] | reverse
| . as $steps
| [ $steps[] | select(.unit) | .unit as $u | .via as $via
    | ($u | fe_len) as $fe
    | if $u.mode == "run-custom-build" then {name: ($u | unit_name), script: $u.duration}
      elif $fe == null then {name: ($u | unit_name), unsplit: (if $via == "rmeta" then $u.duration else $u.duration end)}
      elif $via == "rmeta" then {name: ($u | unit_name), fe: $fe}
      else {name: ($u | unit_name), fe: $fe, cg: ($u.duration - $fe)} end ] as $segs
| ([ $steps[] | select(.gap) | .gap ] | add // 0) as $gaps
| ($last.start + $last.duration) as $total
| "| 单元 | 前端 | codegen | 构建脚本运行 | 未拆分 |", "|---|---:|---:|---:|---:|",
  ( $segs[] | "| \(.name) | \(.fe // 0 | r2) | \(.cg // 0 | r2) | \(.script // 0 | r2) | \(.unsplit // 0 | r2) |" ),
  "| **关键路径合计** | \([$segs[].fe // 0] | add | r2) | \([$segs[].cg // 0] | add | r2) | \([$segs[].script // 0] | add | r2) | \([$segs[].unsplit // 0] | add | r2) |",
  "",
  "等待间隙（调度/等 token）\($gaps | r2) s；构建总长 \($total | r2) s。",
  "全部单元区间和：前端 \([$units[] | select(.mode != "run-custom-build") | (fe_len // empty)] | add // 0 | r2) s，codegen \([$units[] | select(.mode != "run-custom-build") | (fe_len) as $f | select($f != null) | .duration - $f] | add // 0 | r2) s，未拆分 \([$units[] | select(.mode != "run-custom-build") | select(fe_len == null) | .duration] | add // 0 | r2) s，构建脚本运行 \([$units[] | select(.mode == "run-custom-build") | .duration] | add // 0 | r2) s（墙钟区间，不是 CPU）。"
