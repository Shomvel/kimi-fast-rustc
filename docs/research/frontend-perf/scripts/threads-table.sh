#!/usr/bin/env bash
# Markdown table of the -Zthreads probe of one project.
# Usage: threads-table.sh <results/threads/<project>>   (directory with t<N>-s<k>/result.json)
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
[ $# -eq 1 ] || { echo "usage: $0 <threads-dir>" >&2; exit 2; }
jq -s -f "$here/threads-table.jq" "$1"/t*-s*/result.json | jq -r -f "$here/threads-md.jq"
