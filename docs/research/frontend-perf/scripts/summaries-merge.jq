# Merge `summarize summarize --json` outputs (one file per rustc process, named
# `<crate>-<pid>.json`) into the per-process summary format of the 2026-10-10
# release run (mirx/measure/profile_round.py):
#   { "<crate>-<pid>": { "<label>": {"time": s, "self_time": s, "calls": n} } }
# Usage: jq -c -n -f summaries-merge.jq DIR/*.json
def seconds: .secs + .nanos / 1e9;
reduce inputs as $summary ({};
  . + { (input_filename | split("/") | last | rtrimstr(".json")):
        ([$summary.query_data[]
          | {key: .label,
             value: {time: (.time | seconds),
                     self_time: (.self_time | seconds),
                     calls: .invocation_count}}]
         | from_entries) })
