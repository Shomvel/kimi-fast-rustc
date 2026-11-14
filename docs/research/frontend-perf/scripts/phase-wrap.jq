# Wrap phase-categories.jq output with a build name (--arg name). Used by phase-matrix.sh.
{name: $name, cats: .}
