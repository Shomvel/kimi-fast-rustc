#!/usr/bin/env bash
# Usage: w08.sh <workdir>
# Builds w08/lib.rs + w08/bin.rs with rustc -Cincremental, applies one edit per scenario to a
# copy of the baseline incremental state, and reports which CGUs were reused (same inode =
# work product hard-linked from the previous session) and which were re-codegened.
# Env: RUSTC_NIGHTLY (needed for -Zhuman-readable-cgu-names).
set -u
here=$(cd "$(dirname "$0")" && pwd); out=${1:?workdir}; mkdir -p "$out"
RUSTC=${RUSTC_NIGHTLY:-rustc}
echo "rustc: $("$RUSTC" -V)"

edit() { # scenario  < lib.rs  > lib.rs'
  case "$1" in
    base) cat ;;
    E1-comment)   sed 's/E1 改这行注释的文字/E1 已改过的注释文字/' ;;
    E2-lineshift) sed '1i\
// E2 在文件最前面新增的一行
' ;;
    E3-plain-body)   sed 's/x - 10 } else/x - 9 } else/' ;;
    E4-generic-body) sed 's/sum::<u64>() \/\/ E4/sum::<u64>() * 2 \/\/ E4/' ;;
    E5-inline-body)  sed 's/wrapping_mul(3)/wrapping_mul(5)/' ;;
    E6-signature)    sed 's/-> u32 { \/\/ E6/-> u64 { \/\/ E6/' ;;
    E7-add-field)    sed -e 's/pub y: u32, \/\/ E7/pub y: u32, id: u64,/' -e 's/Point { x, y } }/Point { x, y, id: 0 } }/' ;;
    E8-new-impl)     sed 's#// E8#impl Shape for i32 { fn area(\&self) -> u32 { 2 } }#' ;;
    E9-panic-abort)  cat ;;
  esac
}
objs() { # dir -> "name inode cksum" per work-product object
  find "$1" -name '*.o' | while read -r p; do
    printf '%s %s %s\n' "$(basename "$p")" "$(ls -i "$p" | awk '{print $1}')" "$(cksum < "$p" | awk '{print $1}')"
  done | sort
}
report() { # crate before after   ("=" after a name: re-codegened but byte-identical)
  awk -v c="$1" 'NR==FNR {old[$1]=$2; sum[$1]=$3; next}
    { tot++; short=$1; gsub(/\.[0-9a-f]{16}/, "", short); sub(/\.o$/, "", short)
      if (old[$1] == $2) reu++; else { red++; if (sum[$1] == $3) { same++; short = short "=" } list = list " " short } }
    END { if (red == tot) list = " (all)"; printf " | %s CGU %d reused %d redone %d (same bytes %d):%s", c, tot, reu, red, same, list }' "$2" "$3"
}
for cfg in dev rel; do
  case $cfg in
    dev) cf="-Copt-level=0 -Cdebuginfo=2" ;;
    rel) cf="-Copt-level=3 -Clto=off" ;;
  esac
  w="$out/$cfg"; mkdir -p "$w/src" "$w/out"
  echo "== config $cfg: $cf"
  for sc in base E1-comment E2-lineshift E3-plain-body E4-generic-body E5-inline-body E6-signature E7-add-field E8-new-impl E9-panic-abort; do
    pf=""; [ "$sc" = E9-panic-abort ] && pf="-Cpanic=abort"
    if [ "$sc" != base ]; then
      rm -rf "$w/inc-lib" "$w/inc-bin"; cp -R "$w/snap-lib" "$w/inc-lib"; cp -R "$w/snap-bin" "$w/inc-bin"
    fi
    edit "$sc" < "$here/w08/lib.rs" > "$w/src/lib.rs"; cp "$here/w08/bin.rs" "$w/src/bin.rs"
    objs "$w/inc-lib" > "$w/lib.before"; objs "$w/inc-bin" > "$w/bin.before"
    f="$cf $pf --edition 2021 -Zhuman-readable-cgu-names"
    "$RUSTC" $f --crate-type=rlib --crate-name w08lib -Cincremental="$w/inc-lib" --out-dir "$w/out" "$w/src/lib.rs" 2> "$w/$sc.lib.err" || { echo "  $sc: lib build failed"; continue; }
    "$RUSTC" $f --crate-name w08bin -Cincremental="$w/inc-bin" --extern w08lib="$w/out/libw08lib.rlib" -o "$w/out/w08bin" "$w/src/bin.rs" 2> "$w/$sc.bin.err" || { echo "  $sc: bin build failed"; continue; }
    objs "$w/inc-lib" > "$w/lib.after"; objs "$w/inc-bin" > "$w/bin.after"
    printf "  %-16s out=%s" "$sc" "$("$w/out/w08bin" 12 | tr "\n" " " | sed "s/ $//")"
    if [ "$sc" = base ]; then
      echo
      rm -rf "$w/snap-lib" "$w/snap-bin"; cp -R "$w/inc-lib" "$w/snap-lib"; cp -R "$w/inc-bin" "$w/snap-bin"
    else
      report lib "$w/lib.before" "$w/lib.after"; report bin "$w/bin.before" "$w/bin.after"; echo
    fi
  done
done

echo "== O3 asm (host target, cross-crate inline inference off as under -Cincremental): norm1 / line_here"
for sc in base E7-add-field E2-lineshift; do
  edit "$sc" < "$here/w08/lib.rs" > "$out/asm-$sc.rs"
  "$RUSTC" -Copt-level=3 -Ccodegen-units=1 -Zcross-crate-inline-threshold=never --edition 2021 --crate-type=rlib --crate-name w08lib --emit=asm -o "$out/asm-$sc.s" "$out/asm-$sc.rs" 2>/dev/null
  awk -v sc="$sc" '/^_?_ZN|^_?_R/ { f = ($0 ~ /norm1|line_here/); if (f) { name=$0; line="" } next }
    f && /^\t[a-z]/ { s=$0; sub(/^\t/, "", s); gsub(/\t/, " ", s); line = (line == "") ? s : line " ; " s }
    f && /\.cfi_endproc/ { gsub(/_ZN6w08lib[0-9a-z_]*?|17h[0-9a-f]+E:/, "", name); printf "  %-13s %s: %s\n", sc, name, line; f=0 }' "$out/asm-$sc.s"
done
