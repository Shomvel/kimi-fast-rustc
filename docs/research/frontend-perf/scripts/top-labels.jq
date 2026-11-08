# Input: summaries.json. Args: --arg crates "a,b" (empty = all), --arg cat "<category or empty>", --argjson n N.
# Output: markdown rows "| label | category | self s | calls |" for the N labels with the largest summed self time.
include "phase-lib";
($crates | split(",") | map(select(length > 0))) as $want
| [ to_entries[]
    | (.key | sub("-[0-9]+$"; "")) as $crate
    | select(($want | length) == 0 or ($want | index($crate)) != null)
    | .value | to_entries[] | {label: .key, self: .value.self_time, calls: .value.calls} ]
| group_by(.label)
| map({label: .[0].label, cat: category(.[0].label), self: (map(.self) | add), calls: (map(.calls) | add)})
| map(select($cat == "" or .cat == $cat))
| sort_by(-.self) | .[:$n][]
| "| `\(.label)` | \(.cat) | \((.self * 100 | round) / 100) | \(.calls) |"
