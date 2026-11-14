#!/usr/bin/env bash
# Check that each attribution arm's rustc/LLVM flags are accepted by rustc 1.94.1
# (LLVM 21) and actually change code generation on a small probe crate.
#
# usage: check-llvm-args.sh WORK
#
# For every arm prints the rustc exit status and:
#   loop-vectorize .. licm : count of "<pass> (success)" optimization remarks (-Cremark=all;
#                            "unrolled loop by a factor of 1" is not counted)
#   jt                     : jump-table labels (.LJTI) in the asm
#   ra                     : register allocator named by -debug-pass=Structure
#   calls                  : `call` instructions in the asm
# An unknown -Cllvm-args option makes rustc exit 1, so exit 0 also proves the option exists.
# Arm flags come from arm-flags.sh in the same directory.
set -uo pipefail
WORK=${1:?usage: check-llvm-args.sh WORK}
HERE=$(cd "$(dirname "$0")" && pwd)
export RUSTUP_HOME=${RUSTUP_HOME:-/opt/rustup} CARGO_HOME=${CARGO_HOME:-/opt/cargo}
export PATH=$CARGO_HOME/bin:$PATH
D=$WORK/llvm-args-check
mkdir -p "$D"
cat > "$D/probe.rs" <<'EOF'
#[inline(never)]
pub fn sum(xs: &[u32]) -> u32 { xs.iter().fold(0u32, |a, &x| a.wrapping_add(x)) }

#[inline(never)]
pub fn scale(xs: &mut [u32; 64], k: u32) { for x in xs.iter_mut() { *x = x.wrapping_mul(k) ^ 7; } }

#[inline(never)]
pub fn count_byte(hay: &[u8], b: u8) -> usize {
    let mut n = 0;
    let mut i = 0;
    while i < hay.len() { if hay[i] == b { n += 1; } i += 1; }
    n
}

#[inline(never)]
pub fn dispatch(op: u8, a: u64, b: u64) -> u64 {
    match op {
        0 => a + b, 1 => a - b, 2 => a * b, 3 => a / (b | 1), 4 => a % (b | 1),
        5 => a & b, 6 => a | b, 7 => a ^ b, 8 => a << (b & 63), 9 => a >> (b & 63),
        10 => a.rotate_left(b as u32), 11 => a.rotate_right(b as u32), 12 => !a, 13 => a.wrapping_neg(),
        _ => 0,
    }
}

pub fn helper(x: u64) -> u64 { if x > 1000 { x.wrapping_mul(0x9e3779b97f4a7c15) >> 7 } else { x ^ 0x55 } }

pub struct Acc { pub n: u64, pub hits: u64 }

#[inline(never)]
pub fn accum(a: &mut Acc, xs: &[u64]) { for &x in xs { a.n += x; if x & 1 == 1 { a.hits += 1; } } }

#[inline(never)]
pub fn twice(v: &Vec<u64>, i: usize, w: &mut u64) -> u64 { let a = v[i]; *w = a; a + v[i] }

#[inline(never)]
pub fn calls(xs: &[u64]) -> u64 { xs.iter().map(|&x| helper(x)).fold(0, |a, x| a ^ x) }

#[inline(never)]
pub fn many_live(v: &[u64; 32]) -> u64 {
    let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h) = (v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7]);
    for i in 8..32 {
        a = a.wrapping_mul(v[i]) ^ h; b = b.rotate_left(5) ^ a; c = c.wrapping_add(b) ^ g; d = d ^ c.wrapping_mul(3);
        e = e.wrapping_sub(d) ^ f; f = f.rotate_right(11) ^ e; g = g.wrapping_add(f) ^ c; h = h ^ g.wrapping_mul(5);
    }
    a ^ b ^ c ^ d ^ e ^ f ^ g ^ h
}
EOF
ARMS=$(bash "$HERE/arm-flags.sh" --list)
PASSES="loop-vectorize slp-vectorizer loop-unroll inline gvn licm"
printf 'arm\texit'; for p in $PASSES; do printf '\t%s' "$p"; done; printf '\tjt\tra\tcalls\n'
for arm in $ARMS; do
    [[ $(bash "$HERE/arm-flags.sh" --kind "$arm") == rustflags ]] || continue
    read -r -a flags <<< "$(bash "$HERE/arm-flags.sh" "$arm")"
    opt=(-O)
    for f in "${flags[@]}"; do [[ $f == -Copt-level=* ]] && opt=(); done   # O1 must not be overridden by -O
    rustc --edition 2021 --crate-type lib "${opt[@]}" -Ccodegen-units=1 -Cdebuginfo=1 "${flags[@]}" \
        --emit=asm -o "$D/$arm.s" "$D/probe.rs" -Cremark=all -Cllvm-args=-debug-pass=Structure 2> "$D/$arm.err"
    rc=$?
    printf '%s\t%s' "$arm" "$rc"
    # "unrolled loop by a factor of 1" is the U arm's forced no-op, not an unroll.
    for p in $PASSES; do printf '\t%s' "$(grep -E " $p \(success\)" "$D/$arm.err" | grep -cv 'factor of 1$')"; done
    jt=$(grep -c '^\.LJTI' "$D/$arm.s" 2>/dev/null)
    ra=$(grep -oE '(Greedy|Basic|Fast|PBQP) Register Allocator' "$D/$arm.err" | sort -u | cut -d' ' -f1 | paste -sd, -)
    calls=$(grep -cE '^\s+call' "$D/$arm.s" 2>/dev/null)
    printf '\t%s\t%s\t%s\n' "$jt" "${ra:-none}" "$calls"
    [[ $rc == 0 ]] || grep -m3 -i 'error\|unknown' "$D/$arm.err" | sed "s/^/  $arm: /"
done
