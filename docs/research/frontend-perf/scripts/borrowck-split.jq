# Input: summaries.json. Output: markdown rows for mir_borrowck and the borrowck sub-activities a
# recent nightly records, as self seconds summed over processes and share of their total.
[ .[] | to_entries[] | select(.key | test("borrowck|^MIR_borrow_checking$|^check_liveness$")) | {l: .key, s: .value.self_time} ]
| group_by(.l) | map({l: .[0].l, s: (map(.s) | add)}) | sort_by(-.s)
| (map(.s) | add) as $t
| "| 事件 | self 秒 | 占借用检查 |", "|---|---:|---:|",
  (.[] | "| `\(.l)` | \((.s * 100 | round) / 100) | \((.s / $t * 1000 | round) / 10)% |"),
  "| **合计** | \(($t * 100 | round) / 100) | 100% |"
