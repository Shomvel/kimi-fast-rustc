# Sum self time per phase category. Input: summaries.json; --arg crates "a,b" (empty = all).
# Output: [ {cat, secs} ]. Categories are defined in phase-lib.jq.
include "phase-lib";

($crates | split(",") | map(select(length > 0))) as $want
| [ to_entries[]
    | (.key | sub("-[0-9]+$"; "")) as $crate
    | select(($want | length) == 0 or ($want | index($crate)) != null)
    | .value | to_entries[]
    | {cat: category(.key), secs: .value.self_time} ]
| group_by(.cat)
| map({cat: .[0].cat, secs: (map(.secs) | add)})
