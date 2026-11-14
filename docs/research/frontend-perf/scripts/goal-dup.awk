# Input: "label key crate self_us" lines (goal-self-time.awk output, all crates of one build).
# For every (label, key): crates that computed it and total self time. Work that a cross-crate
# shared cache could have saved = for keys computed in k>1 crates, total * (k-1)/k (equal cost
# per crate assumed). Prints per-label rows and a total.
BEGIN { FS = OFS = "\t" }
{
  id = $1 SUBSEP $2
  if (!((id, $3) in seen)) { seen[id, $3] = 1; ncr[id]++ }
  tot[id] += $4; calls[id]++
  lt[$1] += $4; lc[$1]++
}
END {
  for (id in tot) {
    split(id, p, SUBSEP); l = p[1]
    keys[l]++
    if (ncr[id] > 1) { dk[l]++; save[l] += tot[id] * (ncr[id] - 1) / ncr[id]; dcalls[l] += calls[id] * (ncr[id] - 1) / ncr[id] }
  }
  printf "| 查询 | 执行次数 | 不同 key | 跨 crate 重复的 key | self 秒 | 可省 self 秒 | 可省比例 |\n|---|---:|---:|---:|---:|---:|---:|\n"
  for (l in lt) {
    printf "| `%s` | %d | %d | %d | %.2f | %.2f | %.1f%% |\n", l, lc[l], keys[l], dk[l], lt[l] / 1e6, save[l] / 1e6, (lt[l] > 0 ? 100 * save[l] / lt[l] : 0)
    T += lt[l]; S += save[l]; C += lc[l]; DC += dcalls[l]
  }
  printf "| **合计** | %d | | | %.2f | %.2f | %.1f%% |\n", C, T / 1e6, S / 1e6, (T > 0 ? 100 * S / T : 0)
  printf "\n可省的执行次数约 %.0f / %d（%.1f%%）。\n", DC, C, (C > 0 ? 100 * DC / C : 0)
}
