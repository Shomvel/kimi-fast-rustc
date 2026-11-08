# Aggregate the -Zthreads probe: one row per thread count with per-sample and mean values.
# Usage: jq -s -f threads-table.jq threads/<project>/t*-s*/result.json
def mean: if length == 0 then null else add / length end;
group_by(.threads)
| map({
    project: .[0].project,
    threads: .[0].threads,
    samples: length,
    exits: map(.exit),
    wall: map(.wall),
    cpu: map(.cpu),
    instructions_u: map(.instructions_u),
    wall_mean: (map(.wall) | mean),
    cpu_mean: (map(.cpu) | mean),
    instructions_u_mean: (map(.instructions_u) | if any(. == null) then null else mean end),
    max_rss_bytes_max: (map(.max_rss_bytes) | max)
  })
