# Input: slurped result.json files of the profile scenarios (e2b-frontend-profile.sh).
# Output: markdown rows: project, scenario, wall, user+sys, instructions:u, rustc processes.
def r2: (. * 100 | round) / 100;
"| 项目 | 场景 | 墙钟 s | user+sys s | instructions:u G | rustc 进程 |", "|---|---|---:|---:|---:|---:|",
(sort_by(.project, .scenario)[]
 | "| \(.project) | \(.scenario) | \(.wall | r2) | \(.cpu | r2) | \(if .instructions_u then (.instructions_u / 1e9 | r2) else "–" end) | \(.rustc_processes) |")
