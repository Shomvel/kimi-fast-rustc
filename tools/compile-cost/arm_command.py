#!/usr/bin/env python3
"""Print the rustc argv recorded in a semver_arms.py arm.json, NUL-separated."""
import json
import sys

for word in json.load(open(sys.argv[1]))['command']:
    sys.stdout.write(word + '\0')
