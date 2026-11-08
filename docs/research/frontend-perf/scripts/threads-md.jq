# Input: output of threads-table.jq (one object per -Zthreads value). Output: markdown rows with
# mean wall, mean user+sys, mean instructions:u and the change relative to threads=1.
def r2: (. * 100 | round) / 100;
def pct($x; $b): ((($x / $b - 1) * 1000 | round) / 10 | tostring) + "%";
(map(select(.threads == 1)) | .[0]) as $b
| "| -Zthreads | 墙钟均值 s | 相对 1 | user+sys 均值 s | 相对 1 | instructions:u 均值 G | 相对 1 | 峰值 RSS MiB |",
  "|---:|---:|---:|---:|---:|---:|---:|---:|",
  (sort_by(.threads)[]
   | "| \(.threads) | \(.wall_mean | r2) | \(pct(.wall_mean; $b.wall_mean)) | \(.cpu_mean | r2) | \(pct(.cpu_mean; $b.cpu_mean)) | \(.instructions_u_mean / 1e9 | r2) | \(pct(.instructions_u_mean; $b.instructions_u_mean)) | \(.max_rss_bytes_max / 1048576 | round) |")
