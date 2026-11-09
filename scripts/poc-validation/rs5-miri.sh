#!/usr/bin/env bash
# RS5 fixtures under Miri: Stacked Borrows and Tree Borrows must both accept
# the driver's inputs (R3). Needs a nightly toolchain with the miri component.
set -euo pipefail
here=$(cd "$(dirname "$0")/../../crates/rustc_codegen_kfrc/tests/semantic-ir" && pwd)
work=${1:?fresh work directory}
toolchain=${MIRI_TOOLCHAIN:-nightly}
mkdir -p "$work/src"
cat > "$work/Cargo.toml" <<TOML
[package]
name = "semantic_values"
version = "0.0.0"
edition = "2021"
[lib]
path = "src/lib.rs"
[[bin]]
name = "rs5-driver"
path = "src/main.rs"
[workspace]
TOML
printf '#[path = "%s/support_rs5_permissions.rs"]\npub mod rs5_permissions_probe;\n' "$here" > "$work/src/lib.rs"
printf '#[path = "%s/driver_rs5_permissions.rs"]\nmod rs5_permissions;\nfn main() { rs5_permissions::check(); }\n' "$here" > "$work/src/main.rs"
sha256sum "$here/support_rs5_permissions.rs" "$here/driver_rs5_permissions.rs" 2>/dev/null > "$work/inputs.sha256" \
    || shasum -a 256 "$here/support_rs5_permissions.rs" "$here/driver_rs5_permissions.rs" > "$work/inputs.sha256"
cargo +"$toolchain" miri --version > "$work/miri-version.txt"
for model in stacked tree; do
    flags=""
    [ "$model" = tree ] && flags="-Zmiri-tree-borrows"
    (cd "$work" && MIRIFLAGS="$flags" cargo +"$toolchain" miri run --quiet) > "$work/$model.stdout" 2> "$work/$model.stderr"
    printf '%s: PASS (%s lines)\n' "$model" "$(wc -l < "$work/$model.stdout" | tr -d ' ')"
done
