#!/usr/bin/env python3
"""Extract only counters actually emitted by the frozen compiler; never infer visits."""
import argparse
import json
from pathlib import Path
import re

p = argparse.ArgumentParser()
p.add_argument('results', type=Path)
a = p.parse_args()
rows = []
for log in sorted(a.results.rglob('*build.stderr')):
    for line in log.read_text().splitlines():
        match = re.match(r'kfrc (optimization|inlining) (.+): (\w+) \{(.*)\}', line)
        if not match:
            continue
        kind, symbol, record, fields = match.groups()
        counters = {key: int(value) for key, value in re.findall(r'\b(\w+): (\d+)(?:,|$)', fields)}
        rows.append({'log': str(log.relative_to(a.results)), 'kind': kind, 'symbol': symbol,
                     'record': record, 'reported_counters': counters, 'raw': line})
summary = {'scope': 'compiler-reported counters only; not every pass is instrumented; node/rewrite counts are not inferred visits',
           'complete_per_pass_visits': False, 'records': rows}
(a.results/'reported-pass-statistics.json').write_text(json.dumps(summary, indent=2)+'\n')
print(f'Extracted {len(rows)} reported records; complete pass visit accounting unavailable')
