"""Planning metadata for pinned rustc compiletest tests; not a compiletest replacement.

The authoritative executor is the upstream standalone compiletest binary. At
rust commit e408947bfd200af42db322daf0fadfe7e26d3bd1 it accepts a standalone
Config (src-root, src-test-suite-root, build-root, build-test-suite-root,
compile-lib-path, run-lib-path, sysroot-base, host/target, stage and tools).
Its local build_helper and miropt-test-tools dependencies do not require the
bootstrap driver. See src/tools/compiletest/src/{lib.rs,common.rs,directives/}.

This module expands revisions and extracts scheduling/provenance metadata. Known
protocol that needs compiletest's full Config is explicitly delegated to that
executor; unknown or malformed protocol is blocked. No Rust source is rewritten.
"""

from __future__ import annotations

import hashlib
import re
from pathlib import Path
from typing import Any, Mapping

UPSTREAM_COMMIT = "e408947bfd200af42db322daf0fadfe7e26d3bd1"
# Exact list from the pinned src/tools/compiletest/src/directives/directive_names.rs.
KNOWN_DIRECTIVES = frozenset("""
add-minicore assembly-output aux-bin aux-build aux-codegen-backend aux-crate
build-aux-docs build-fail build-pass cdb-check cdb-command check-fail
check-pass check-run-results check-stdout check-test-line-numbers-match compare-output-by-lines compile-flags
disable-gdb-pretty-printers doc-flags dont-check-compiler-stderr dont-check-compiler-stdout dont-check-failure-status dont-require-annotations
edition error-pattern exact-llvm-major-version exec-env failure-status filecheck-flags
forbid-output force-host gdb-check gdb-command ignore-16bit ignore-32bit
ignore-64bit ignore-aarch64 ignore-aarch64-pc-windows-msvc ignore-aarch64-unknown-linux-gnu ignore-aix ignore-android
ignore-apple ignore-arm ignore-arm-unknown-linux-gnueabi ignore-arm-unknown-linux-gnueabihf ignore-arm-unknown-linux-musleabi ignore-arm-unknown-linux-musleabihf
ignore-auxiliary ignore-avr ignore-backends ignore-beta ignore-cdb ignore-compare-mode-next-solver
ignore-compare-mode-polonius ignore-coverage-map ignore-coverage-run ignore-cross-compile ignore-eabi ignore-elf
ignore-emscripten ignore-endian-big ignore-enzyme ignore-freebsd ignore-fuchsia ignore-gdb
ignore-gdb-version ignore-gnu ignore-haiku ignore-horizon ignore-i586 ignore-i686-pc-windows-gnu
ignore-i686-pc-windows-msvc ignore-illumos ignore-ios ignore-linux ignore-lldb ignore-llvm-version
ignore-loongarch32 ignore-loongarch64 ignore-macabi ignore-macos ignore-msp430 ignore-msvc
ignore-musl ignore-netbsd ignore-nightly ignore-none ignore-nto ignore-nvptx64
ignore-nvptx64-nvidia-cuda ignore-openbsd ignore-pass ignore-powerpc ignore-powerpc64 ignore-remote
ignore-riscv64 ignore-rustc-debug-assertions ignore-rustc_abi-x86-sse2 ignore-s390x ignore-sgx ignore-sparc64
ignore-spirv ignore-stable ignore-stage1 ignore-stage2 ignore-std-debug-assertions ignore-test
ignore-thumb ignore-thumbv8m.base-none-eabi ignore-thumbv8m.main-none-eabi ignore-tvos ignore-uefi ignore-unix
ignore-unknown ignore-uwp ignore-visionos ignore-vxworks ignore-wasi ignore-wasm
ignore-wasm32 ignore-wasm32-unknown-unknown ignore-wasm64 ignore-watchos ignore-windows ignore-windows-gnu
ignore-windows-msvc ignore-x32 ignore-x86 ignore-x86_64 ignore-x86_64-apple-darwin ignore-x86_64-pc-windows-gnu
ignore-x86_64-unknown-linux-gnu incremental known-bug lldb-check lldb-command llvm-cov-flags
max-llvm-major-version min-cdb-version min-gdb-version min-lldb-version min-llvm-version min-system-llvm-version
minicore-compile-flags needs-asm-support needs-backends needs-crate-type needs-deterministic-layouts needs-dlltool
needs-dynamic-linking needs-enzyme needs-force-clang-based-tests needs-git-hash needs-llvm-components needs-llvm-zstd
needs-offload needs-profiler-runtime needs-relocation-model-pic needs-run-enabled needs-rust-lld needs-rustc-debug-assertions
needs-sanitizer-address needs-sanitizer-cfi needs-sanitizer-dataflow needs-sanitizer-hwaddress needs-sanitizer-kcfi needs-sanitizer-leak
needs-sanitizer-memory needs-sanitizer-memtag needs-sanitizer-realtime needs-sanitizer-safestack needs-sanitizer-shadow-call-stack needs-sanitizer-support
needs-sanitizer-thread needs-std-debug-assertions needs-std-remap-debuginfo needs-subprocess needs-symlink needs-target-has-atomic
needs-target-std needs-threads needs-unwind needs-wasmtime needs-xray no-auto-check-cfg
no-prefer-dynamic normalize-stderr normalize-stderr-32bit normalize-stderr-64bit normalize-stdout only-16bit
only-32bit only-64bit only-aarch64 only-aarch64-apple-darwin only-aarch64-unknown-linux-gnu only-aarch64-unknown-uefi
only-apple only-arm only-arm64ec only-avr only-beta only-bpf
only-cdb only-dist only-eabihf only-elf only-emscripten only-gnu
only-i686-pc-windows-gnu only-i686-pc-windows-msvc only-i686-unknown-linux-gnu only-ios only-linux only-loongarch32
only-loongarch64 only-loongarch64-unknown-linux-gnu only-macos only-mips only-mips64 only-msp430
only-msvc only-musl only-nightly only-nvptx64 only-powerpc only-riscv32
only-riscv64 only-rustc_abi-x86-sse2 only-s390x only-sparc only-sparc64 only-stable
only-thumb only-tvos only-uefi only-unix only-visionos only-wasm32
only-wasm32-unknown-unknown only-wasm32-wasip1 only-watchos only-windows only-windows-gnu only-windows-msvc
only-x86 only-x86_64 only-x86_64-apple-darwin only-x86_64-fortanix-unknown-sgx only-x86_64-pc-windows-gnu only-x86_64-pc-windows-msvc
only-x86_64-unknown-linux-gnu pp-exact pretty-compare-only pretty-mode proc-macro reference
regex-error-pattern remap-src-base revisions run-crash run-fail run-fail-or-crash
run-flags run-pass run-rustfix rustc-env rustfix-only-machine-applicable should-fail
should-ice stderr-per-bitwidth test-mir-pass unique-doc-out-dir unset-exec-env unset-rustc-env
unused-revision-names
""".split())

MODES = frozenset("check-pass build-pass run-pass check-fail build-fail run-fail run-crash run-fail-or-crash".split())
BOOLS = frozenset("force-host check-stdout check-run-results dont-check-compiler-stdout dont-check-compiler-stderr dont-check-failure-status no-prefer-dynamic no-auto-check-cfg stderr-per-bitwidth".split())
VALUE_DIRECTIVES = frozenset("edition revisions compile-flags run-flags aux-build aux-crate aux-bin proc-macro aux-codegen-backend rustc-env exec-env unset-rustc-env unset-exec-env error-pattern regex-error-pattern forbid-output failure-status normalize-stdout normalize-stderr normalize-stderr-32bit normalize-stderr-64bit".split())
OUTPUT_KINDS = ("stderr", "stdout", "run.stderr", "run.stdout", "fixed", "32bit.stderr", "64bit.stderr", "16bit.stderr", "svg", "windows.svg", "coverage", "cov-map")
SUBSTITUTIONS = frozenset("cwd src-base build-base rust-src-base sysroot-base target-linker target".split())
BUILTIN_CFGS = frozenset("debug_assertions overflow_checks proc_macro test target_arch target_os target_env target_vendor target_abi target_endian target_family target_feature target_has_atomic target_pointer_width unix windows panic".split())


def split_flags(value: str) -> list[str]:
    """Match pinned compiletest's single-quote splitter, intentionally not shlex."""
    return [flag for i, part in enumerate(value.split("'"))
            for flag in ([part] if i % 2 else part.split())]


def _inner_attributes(text: str) -> list[str]:
    """Find literal inner attributes without treating comments/strings as code."""
    tokens = []
    index = 0
    while index < len(text):
        if text[index].isspace():
            index += 1
            continue
        if text.startswith("//", index):
            end = text.find("\n", index)
            index = len(text) if end < 0 else end + 1
            continue
        if text.startswith("/*", index):
            depth = 1
            index += 2
            while index < len(text) and depth:
                if text.startswith("/*", index):
                    depth += 1
                    index += 2
                elif text.startswith("*/", index):
                    depth -= 1
                    index += 2
                else:
                    index += 1
            continue
        raw = re.match(r'(?:br|cr|r)(#*)"', text[index:])
        if raw:
            end_marker = '"' + raw[1]
            end = text.find(end_marker, index + raw.end())
            end = len(text) if end < 0 else end + len(end_marker)
        elif text[index] == '"':
            end = index + 1
            while end < len(text):
                if text[end] == "\\":
                    end += 2
                elif text[end] == '"':
                    end += 1
                    break
                else:
                    end += 1
        else:
            char = re.match(r"'(?:\\.|[^'\\])'", text[index:])
            word = re.match(r"[A-Za-z_][A-Za-z0-9_]*", text[index:])
            end = index + (char.end() if char else word.end() if word else 1)
        tokens.append(text[index:end])
        index = end
    attributes = []
    index = 0
    while index + 2 < len(tokens):
        if tokens[index:index + 3] != ["#", "!", "["]:
            index += 1
            continue
        start, depth = index + 3, 1
        index = start
        while index < len(tokens) and depth:
            depth += (tokens[index] == "[") - (tokens[index] == "]")
            index += 1
        attributes.append(" ".join(tokens[start:index - 1]))
    return attributes


def _static_auxiliary_type(text: str, flags: list[str]) -> tuple[bool, str]:
    """Conservatively recognize one literal lib/rlib type; do not expand Rust cfg."""
    types = []
    for attribute in _inner_attributes(text):
        if re.match(r"crate_type\b", attribute) or re.match(r"cfg_attr\b", attribute) and re.search(r"\bcrate_type\b", attribute):
            direct = re.fullmatch(r'crate_type\s*=\s*"(lib|rlib|[^"\\]*)"', attribute)
            if direct is None:
                return False, "crate_type requires cfg/macro/unknown attribute evaluation"
            types.append(direct[1])
    index = 0
    while index < len(flags):
        flag = flags[index]
        if flag == "--crate-type":
            index += 1
            if index >= len(flags):
                return False, "--crate-type has no value"
            value = flags[index]
        elif flag.startswith("--crate-type="):
            value = flag.partition("=")[2]
        else:
            index += 1
            continue
        if not value or "," in value or "{{" in value:
            return False, "multiple or unresolved --crate-type values"
        types.append(value)
        index += 1
    unique = set(types)
    if len(unique) != 1:
        return False, "no unique explicit crate_type=lib/rlib (rustc defaults to bin)"
    chosen = unique.pop()
    return (True, chosen) if chosen in ("lib", "rlib") else (False, f"crate_type={chosen} is not a static Rust library")


def auxiliary_profile_blocks(variant: Mapping[str, Any], root: str | Path) -> list[str]:
    """Explain auxiliary conflicts with the native Linux abort/static PoC profile.

    Upstream compiletest remains the executor. Its default auxiliary crate type
    is a *trailing* --crate-type=dylib on Linux. Only an auxiliary's own active
    no-prefer-dynamic suppresses that flag; main-test properties are not copied.
    The auxiliary must then explicitly declare a simple lib/rlib type. Recursive
    auxiliary paths all resolve against the original main test's auxiliary/,
    with the same revision (runtest.rs build_auxiliary/resolve_aux_path).
    """
    if not variant.get("auxiliary"):
        return []
    root = Path(root).resolve()
    source = (root / variant["source"]).resolve()
    auxiliary_root = source.parent / "auxiliary"
    revision = variant.get("revision")
    reasons: list[str] = []
    visiting: set[Path] = set()
    visited: set[tuple[Path, str]] = set()

    def block(label: str, reason: str) -> None:
        reasons.append(f"auxiliary {label}: {reason}")

    def visit(entry: Mapping[str, Any]) -> None:
        kind, relative = entry.get("kind"), entry.get("path")
        label = str(relative or entry.get("source", "<unknown>"))
        if kind not in ("aux-build", "aux-crate", "aux-bin", "proc-macro", "aux-codegen-backend") or not isinstance(relative, str):
            block(label, "unknown auxiliary declaration")
            return
        path = (auxiliary_root / relative).resolve()
        try:
            label = path.relative_to(root).as_posix()
        except ValueError:
            block(label, "source escapes test source root")
            return
        if path in visiting:
            block(label, "cyclic auxiliary dependency")
            return
        key = (path, kind)
        if key in visited:
            return
        visited.add(key)
        if not path.is_file():
            block(label, "source is missing")
            return
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeError) as error:
            block(label, f"cannot read source: {error}")
            return
        directives, errors = _directives(text)
        for error in errors:
            block(label, error)
        active = [line for line in directives if line["revision"] in (None, revision)]
        flags = [flag for line in active if line["name"] == "compile-flags"
                 for flag in split_flags(line["value"] or "")]
        if kind in ("proc-macro", "aux-codegen-backend"):
            block(label, f"{kind} requires unsupported dynamic compiler loading")
        elif kind != "aux-bin":
            no_dynamic = any(line["name"] == "no-prefer-dynamic" for line in active)
            if not no_dynamic:
                block(label, "own active no-prefer-dynamic is absent; official compiletest appends --crate-type=dylib on native Linux")
            else:
                static, detail = _static_auxiliary_type(text, flags)
                if not static:
                    block(label, detail)
                if kind == "aux-crate":
                    # Pinned compiletest returns AuxType::Dylib even when
                    # no-prefer-dynamic suppresses the crate-type flag. Its
                    # explicit --extern filename is therefore still lib*.so.
                    block(label, "official aux-crate --extern expects a dylib filename even with no-prefer-dynamic; static profile cannot preserve this contract")
        visiting.add(path)
        for line in active:
            child_kind, value = line["name"], line["value"]
            if child_kind not in ("aux-build", "aux-crate", "aux-bin", "proc-macro", "aux-codegen-backend") or not value:
                continue
            relative = value.strip()
            if child_kind == "aux-crate":
                _, separator, relative = relative.partition("=")
                if not separator or not relative:
                    block(label, f"line {line['line']}: invalid aux-crate NAME=PATH")
                    continue
            visit({"kind": child_kind, "path": relative})
        visiting.remove(path)

    for auxiliary in variant["auxiliary"]:
        visit(auxiliary)
    return list(dict.fromkeys(reasons))


def _directives(text: str) -> tuple[list[dict[str, Any]], list[str]]:
    lines, errors = [], []
    for number, raw in enumerate(text.splitlines(), 1):
        value = raw.strip()
        if not value.startswith("//@"):
            continue
        value = value[3:].lstrip()
        selector = None
        if value.startswith("["):
            if "]" not in value:
                errors.append(f"line {number}: malformed revision selector")
                continue
            selector, value = value[1:].split("]", 1)
            value = value.lstrip()
        name = re.split(r"[: ]", value, maxsplit=1)[0]
        rest = value[len(name):]
        entry = {"line": number, "raw": raw, "revision": selector,
                 "name": name, "value": rest[1:] if rest.startswith(":") else None,
                 "remark": rest[1:] if rest.startswith(" ") else None}
        lines.append(entry)
        if name not in KNOWN_DIRECTIVES:
            errors.append(f"line {number}: unknown upstream directive {name!r}")
        remark = entry["remark"]
        if remark and remark.split() and remark.split()[0] in KNOWN_DIRECTIVES:
            errors.append(f"line {number}: multiple directives on one line")
        if name in VALUE_DIRECTIVES and not (entry["value"] or "").strip():
            errors.append(f"line {number}: {name} requires a nonempty colon value")
    return lines, errors


def _target_facts(target: str | Mapping[str, Any]) -> tuple[str, dict[str, Any]]:
    """Accept queried cfg facts; infer only the explicitly supported common triples."""
    if isinstance(target, Mapping):
        return str(target["triple"]), dict(target)
    arch = target.split("-", 1)[0]
    facts: dict[str, Any] = {"triple": target}
    # Exact known targets, not substring-based target guessing.
    common = {
        "x86_64-unknown-linux-gnu": ("x86_64", "linux", "gnu", "", 64, ["unix"], "elf"),
        "i686-unknown-linux-gnu": ("x86", "linux", "gnu", "", 32, ["unix"], "elf"),
        "aarch64-unknown-linux-gnu": ("aarch64", "linux", "gnu", "", 64, ["unix"], "elf"),
        "x86_64-apple-darwin": ("x86_64", "macos", "", "", 64, ["unix"], "macho"),
        "aarch64-apple-darwin": ("aarch64", "macos", "", "", 64, ["unix"], "macho"),
        "x86_64-pc-windows-msvc": ("x86_64", "windows", "msvc", "", 64, ["windows"], "coff"),
    }
    if target in common:
        values = common[target]
        facts.update(zip(("arch", "os", "env", "abi", "pointer_width", "families", "binary_format"), values))
        facts["endian"] = "little"
    facts["thumb"] = arch.startswith("thumb")
    facts["apple"] = "apple" in target.split("-")
    return target, facts


def _condition(name: str, triple: str, facts: Mapping[str, Any], host: str | None,
               capabilities: Mapping[str, Any]) -> bool | None:
    if name in capabilities and isinstance(capabilities[name], bool):
        return capabilities[name]
    if name in ("test", "auxiliary"):
        return True
    if name == "cross-compile":
        return None if host is None else triple != host
    if name == "thumb" or name == "apple":
        return facts.get(name)
    if name == "endian-big":
        return None if "endian" not in facts else facts["endian"] == "big"
    if re.fullmatch(r"(?:16|32|64)bit", name):
        return None if "pointer_width" not in facts else int(facts["pointer_width"]) == int(name[:-3])
    if name == "elf":
        return None if "binary_format" not in facts else facts["binary_format"] == "elf"
    # Full target triples occur in the pinned known directive list.
    if re.match(r"^(?:x86_64|i686|aarch64|arm|nvptx64|loongarch64|thumb[^-]*)-", name):
        return triple == name
    groups = {
        "arch": "aarch64 arm arm64ec avr bpf loongarch32 loongarch64 mips mips64 msp430 nvptx64 powerpc powerpc64 riscv32 riscv64 s390x sparc sparc64 spirv wasm32 wasm64 x86 x86_64",
        "os": "aix android emscripten freebsd fuchsia haiku horizon illumos ios linux macos netbsd none nto openbsd tvos uefi visionos vxworks wasi watchos windows",
        "env": "gnu macabi msvc musl newlib sgx unknown uclibc",
        "abi": "eabi eabihf x32",
    }
    for field, values in groups.items():
        if name in values.split():
            return None if field not in facts else facts[field] == name
    if name in ("unix", "wasm", "windows"):
        return None if "families" not in facts else name in facts["families"]
    if name in ("windows-gnu", "windows-msvc"):
        return None if "os" not in facts or "env" not in facts else f"{facts['os']}-{facts['env']}" == name
    return None


def parse_test(path: str | Path, root: str | Path, target: str | Mapping[str, Any],
               revision: str | None = None, *, host: str | None = None,
               capabilities: Mapping[str, Any] | None = None,
               substitutions: Mapping[str, str] | None = None) -> list[dict[str, Any]]:
    """Return JSON-serializable planning records, one per selected revision.

    ``root`` is the source root containing ``ui/``; ``path`` is absolute or relative
    to it. Capabilities use upstream names (e.g. ``needs-unwind``) and optional
    ``can-unwind`` for run-fail metadata. Missing facts remain delegated to the
    authoritative compiletest executor, never silently treated as satisfied.
    """
    root = Path(root).resolve()
    path = Path(path)
    path = (root / path).resolve() if not path.is_absolute() else path.resolve()
    try:
        source = path.relative_to(root).as_posix()
    except ValueError:
        raise ValueError(f"test source escapes source root: {path}") from None
    text = path.read_text(encoding="utf-8")
    lines, errors = _directives(text)
    capabilities = dict(capabilities or {})
    triple, facts = _target_facts(target)
    variables = dict(substitutions or {})
    variables.setdefault("target", triple)
    declared: list[str] = []
    for line in lines:
        if line["name"] != "revisions":
            continue
        if line["revision"] is not None:
            errors.append(f"line {line['line']}: revisions declaration cannot have a revision selector")
        for rev in (line["value"] or "").split():
            if rev in declared or rev in ("true", "false"):
                errors.append(f"line {line['line']}: duplicate or forbidden revision {rev!r}")
            elif not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_-]*", rev):
                errors.append(f"line {line['line']}: unsupported revision name {rev!r}")
            declared.append(rev)
    for line in lines:
        if line["revision"] is not None and line["revision"] not in declared:
            errors.append(f"line {line['line']}: undeclared revision selector {line['revision']!r}")
    revisions: list[str | None] = list(dict.fromkeys(declared)) or [None]
    if revision is not None:
        if revision not in declared:
            errors.append(f"requested revision {revision!r} is not declared")
        revisions = [revision]
    return [_parse_revision(path, root, source, text, lines, errors, rev, declared,
                            triple, facts, host, capabilities, variables) for rev in revisions]


def _parse_revision(path, root, source, text, lines, errors, revision, declared,
                    triple, facts, host, capabilities, variables):
    out: dict[str, Any] = {
        "source": source, "source_sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        "revision": revision, "revisions": declared, "mode": "check-fail", "edition": "2015",
        "target": triple, "host": host, "compile_flags": [], "run_flags": [],
        "rustc_env": {}, "exec_env": {}, "unset_rustc_env": [], "unset_exec_env": [],
        "auxiliary": [], "requires": [], "blocked": list(errors), "skip_reasons": [],
        "delegated_to_compiletest": [], "error_patterns": [], "regex_error_patterns": [],
        "forbid_output": [], "failure_status": None, "expected_failure_status": None,
        "normalizations": [], "expected_outputs": [], "checks": {},
        "directives": [dict(x) for x in lines if x["revision"] in (None, revision)],
    }
    modes = []
    for line in out["directives"]:
        name, value = line["name"], line["value"]
        loc = f"line {line['line']}"
        if name not in KNOWN_DIRECTIVES:
            continue
        if value is not None:
            for token in re.findall(r"\{\{([^{}]+)\}\}", value):
                if token not in SUBSTITUTIONS:
                    out["blocked"].append(f"{loc}: unknown compiletest substitution {{{{{token}}}}}")
                elif token in variables:
                    value = value.replace("{{" + token + "}}", variables[token])
                else:
                    out["delegated_to_compiletest"].append({"line": line["line"], "name": name, "reason": f"substitution {{{{{token}}}}} requires executor Config"})
        if name in MODES:
            modes.append(name)
            out["mode"] = name
        elif name == "edition" and value:
            edition = value.strip()
            bounds = edition.split("..")
            allowed = ("2015", "2018", "2021", "2024", "future")
            if len(bounds) > 2 or not bounds[0] or any(b and b.strip() not in allowed for b in bounds):
                out["blocked"].append(f"{loc}: invalid or unsupported edition {edition!r}")
            elif len(bounds) == 2 and bounds[1] and allowed.index(bounds[1].strip()) <= allowed.index(bounds[0].strip()):
                out["blocked"].append(f"{loc}: edition range must be increasing")
            else:
                out["edition"] = bounds[0].strip()
                if len(bounds) == 2:
                    out["delegated_to_compiletest"].append({"line": line["line"], "name": name, "reason": "edition range resolved against executor --edition"})
        elif name in ("compile-flags", "run-flags") and value:
            flags = split_flags(value)
            out[name.replace("-", "_")].extend(flags)
            for i, flag in enumerate(flags):
                if name == "compile-flags" and (flag == "--edition" or flag.startswith("--edition=") or flag.startswith("-Cincremental=") or flag == "-C" and i + 1 < len(flags) and flags[i + 1].startswith("incremental=")):
                    out["blocked"].append(f"{loc}: compiletest requires edition/incremental directives instead of {flag!r}")
        elif name in ("rustc-env", "exec-env") and value:
            key, separator, env_value = value.partition("=")
            key = key.strip()
            if not key or "\0" in key or "\0" in env_value:
                out["blocked"].append(f"{loc}: invalid environment assignment")
            out[name.replace("-", "_")][key] = env_value if separator else ""
        elif name in ("unset-rustc-env", "unset-exec-env") and value:
            out[name.replace("-", "_")].append(value.strip())
        elif name in ("error-pattern", "regex-error-pattern", "forbid-output") and value:
            key = {"error-pattern": "error_patterns", "regex-error-pattern": "regex_error_patterns", "forbid-output": "forbid_output"}[name]
            out[key].append(value.strip())
        elif name == "failure-status" and value:
            try:
                status = int(value.strip())
                if not -(2 ** 31) <= status < 2 ** 31:
                    raise ValueError
                out["failure_status"] = status
            except ValueError:
                out["blocked"].append(f"{loc}: invalid i32 failure-status")
        elif name in BOOLS:
            out["checks"][name.replace("-", "_")] = True
        elif name.startswith("normalize-") and value:
            match = re.fullmatch(r'\s*"([^"]*)"\s+->\s+"([^"]*)"', value)
            if match:
                out["normalizations"].append({"kind": name, "regex": match[1], "replacement": match[2].replace("\\n", "\n")})
            else:
                out["blocked"].append(f"{loc}: malformed normalization rule")
        elif name in ("aux-build", "aux-crate", "aux-bin", "proc-macro", "aux-codegen-backend") and value:
            aux_name, aux_path = None, value.strip()
            if name == "aux-crate":
                aux_name, separator, aux_path = aux_path.partition("=")
                if not separator or not aux_name or not aux_path:
                    out["blocked"].append(f"{loc}: aux-crate requires NAME=PATH")
                    continue
            aux = (path.parent / "auxiliary" / aux_path).resolve()
            try:
                aux_source = aux.relative_to(root).as_posix()
            except ValueError:
                out["blocked"].append(f"{loc}: auxiliary path escapes source root: {aux_path}")
                continue
            entry = {"kind": name, "name": aux_name, "path": aux_path, "source": aux_source}
            if not aux.is_file():
                out["blocked"].append(f"{loc}: missing auxiliary source {aux_source}")
            else:
                entry["source_sha256"] = hashlib.sha256(aux.read_bytes()).hexdigest()
                aux_lines, aux_errors = _directives(aux.read_text(encoding="utf-8"))
                entry["directives"] = aux_lines
                out["blocked"].extend(f"auxiliary {aux_source}: {error}" for error in aux_errors)
            out["auxiliary"].append(entry)
        elif name.startswith(("only-", "ignore-", "needs-")):
            matched = None
            if name.startswith(("only-", "ignore-")) and name not in ("ignore-backends", "ignore-llvm-version", "ignore-gdb-version", "ignore-pass"):
                prefix, condition = name.split("-", 1)
                matched = _condition(condition, triple, facts, host, capabilities)
                if matched is not None and (matched if prefix == "ignore" else not matched):
                    out["skip_reasons"].append(f"{loc}: {name} excludes target/configuration {triple}")
            elif name.startswith("needs-"):
                matched = capabilities.get(name)
                if not isinstance(matched, bool):
                    matched = None
                if matched is False:
                    out["skip_reasons"].append(f"{loc}: required capability {name} is unavailable")
            out["requires"].append({"name": name, "value": value, "matched": matched, "line": line["line"]})
            if matched is None:
                out["delegated_to_compiletest"].append({"line": line["line"], "name": name, "reason": "requires authoritative executor Config"})
        elif name != "revisions":
            out["delegated_to_compiletest"].append({"line": line["line"], "name": name, "reason": "upstream executor owns this protocol"})
    if len(modes) > 1:
        out["blocked"].append(f"multiple pass/fail directives apply to revision: {modes}")
    flags = out["compile_flags"]
    if revision is not None:
        normalized = revision.lower().replace("-", "_")
        if normalized in BUILTIN_CFGS:
            out["blocked"].append(f"revision {revision!r} collides with builtin cfg")
        if f"--cfg={normalized}" in flags or any(a == "--cfg" and b == normalized for a, b in zip(flags, flags[1:])):
            out["blocked"].append(f"revision {revision!r} has redundant compile-flags --cfg")
        flags.extend(["--cfg", normalized])
    if not out["checks"].get("no_auto_check_cfg"):
        flags.extend(["--check-cfg", "cfg(test,FALSE" + "".join("," + r.lower().replace("-", "_") for r in declared) + ")"])
    flags.insert(0, "--edition=" + out["edition"])
    if out["checks"].get("force_host"):
        if host is None:
            out["delegated_to_compiletest"].append({"name": "force-host", "reason": "host target resolved by executor Config"})
        else:
            out["effective_target"] = host
    else:
        out["effective_target"] = triple
    if out["mode"] in ("check-fail", "build-fail"):
        out["expected_failure_status"] = out["failure_status"] if out["failure_status"] is not None else 1
    if out["mode"] == "run-fail":
        can_unwind = capabilities.get("can-unwind")
        out["run_expectation"] = {"result": "fail", "exit_code_min": 1, "exit_code_max": 127,
                                  "allow_crash": None if can_unwind is None else not can_unwind}
    elif out["mode"].startswith("run-"):
        out["run_expectation"] = {"result": {"run-pass": "pass", "run-crash": "crash", "run-fail-or-crash": "fail-or-crash"}[out["mode"]]}
    # Compiletest has no generic fallback across revisions: foo.rev.stderr is
    # distinct from foo.stderr. Compare-mode fallback belongs to the executor.
    for kind in OUTPUT_KINDS:
        suffix = ".".join(x for x in (revision, kind) if x)
        expected = path.with_suffix("." + suffix)
        if expected.is_file():
            out["expected_outputs"].append({"kind": kind, "source": expected.relative_to(root).as_posix(), "source_sha256": hashlib.sha256(expected.read_bytes()).hexdigest()})
    if re.search(r"//(?:\[[^]]+\])?~", text):
        out["delegated_to_compiletest"].append({"name": "diagnostic-annotations", "reason": "upstream executor validates inline diagnostic annotations"})
    out["blocked"] = list(dict.fromkeys(out["blocked"]))
    out["status"] = "blocked" if out["blocked"] else "skipped" if out["skip_reasons"] else "ready"
    return out
