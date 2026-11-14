# Input: TSV "tid ts dur label key crate", sorted by tid, then ts ascending, then dur descending.
# Computes self time (dur minus direct children) with a per-thread stack and prints, for events
# whose key is not "-": "label key crate self_us".
BEGIN { FS = OFS = "\t" }
function flush(i) { if (key[i] != "-") print lab[i], key[i], cr[i], self[i] }
{
  if ($1 != tid) { while (n > 0) { flush(n); n-- } tid = $1 }
  ts = $2 + 0; d = $3 + 0
  while (n > 0 && end[n] <= ts) { flush(n); n-- }
  if (n > 0) self[n] -= d
  n++; end[n] = ts + d; self[n] = d; lab[n] = $4; key[n] = $5; cr[n] = $6
}
END { while (n > 0) { flush(n); n-- } }
