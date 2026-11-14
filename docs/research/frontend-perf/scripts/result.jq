# Build result.json for one measured build.
# Inputs (all via jq options):
#   --arg project P --arg scenario S
#   --argjson start T0 --argjson end T1      bash $EPOCHREALTIME before/after the build
#   --argjson exit N                         exit status of the build
#   --argjson profiles N                     .mm_profdata files written (all rustc processes, incl. cargo's `___` probe)
#   --argjson rustc_processes N              profiles excluding the `___` target-info probe
#   --argjson extra OBJ                      extra fields merged into the result (e.g. threads/sample)
#   --rawfile time FILE                      `/usr/bin/time -v -o FILE` output
#   --slurpfile perf FILE                    perfcount output ({"instructions_u": N|null, "cycles_u": N|null}) or {}
# Usage: jq -n -f result.jq <options>
def field($name):
  [$time | split("\n")[] | select(startswith("\t" + $name + ": ")) | ltrimstr("\t" + $name + ": ")]
  | if length == 0 then null else (first | tonumber) end;
{
  project: $project,
  scenario: $scenario,
  exit: $exit,
  wall: ($end - $start),
  user: field("User time (seconds)"),
  system: field("System time (seconds)"),
  max_rss_bytes: (field("Maximum resident set size (kbytes)") | if . == null then null else . * 1024 end),
  instructions_u: ($perf[0].instructions_u // null),
  cycles_u: ($perf[0].cycles_u // null),
  profiles: $profiles,
  rustc_processes: $rustc_processes
}
| .cpu = (if .user != null and .system != null then .user + .system else null end)
| . + $extra
