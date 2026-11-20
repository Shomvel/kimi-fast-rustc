"""Summarize a complete per-unit survey without treating observations as gaps."""
import argparse
from collections import Counter, defaultdict
import csv
import hashlib
import json
from pathlib import Path
import re


def category(message):
    first = message.splitlines()[0]
    if "compiler panic" in first:
        return "compiler-panic"
    if "executable adapter" in first or "lang_start" in first or "Termination" in first:
        return "runtime-entry"
    if "size_of_val" in first and "unsized" in first:
        return "DST-size_of_val"
    if "unsized field requires" in first:
        return "DST-tail"
    if "volatile_load" in first or "volatile_store" in first:
        return "volatile-memory"
    if "control node" in first and "unreachable" in first:
        return "unreachable-control"
    if "Never" in first or "uninhabited" in first or "ignored enum is not uniquely inhabited" in first:
        return "uninhabited-value"
    if "Phi field shape mismatch" in first:
        return "enum-phi-shape"
    if "assigned value disagrees with the semantic place type" in first:
        return "semantic-place-type"
    if "unsupported link section or import linkage" in first:
        return "static-linkage"
    if "C-variadic" in first:
        return "C-variadic"
    if match := re.search(r"intrinsic `?(atomic_[a-z_]+)", first):
        return match.group(1)
    if "floating-point scalar ABI" in first:
        return "float-ABI"
    if "union" in first and "SIMD" in first:
        return "union-or-SIMD-ambiguous"
    if "union" in first:
        return "union"
    if "SIMD" in first or "simd_" in first:
        return "SIMD"
    if re.search(r"\b[ui]128\b|bits: 128|128-bit", first):
        return "i128-u128"
    if "aggregate constants require" in first or "aggregate constant requires" in first:
        return "aggregate-constant"
    if "float" in first or re.search(r"\bf(?:16|32|64|128)\b", first):
        return "float-other"
    if "InlineAsm" in first or "global_asm" in first:
        return "assembly"
    if "ThreadLocal" in first or "TLS" in first:
        return "TLS"
    if "unwind" in first.lower():
        return "unwind"
    if "dynamic" in first or "vtable" in first or "dyn " in first or "trait object" in first:
        return "dyn"
    if "LLVM intrinsic" in first:
        return "LLVM-intrinsic"
    if "verification" in first:
        return "verification-other"
    if "atomic" in first:
        return "atomic-other"
    return "other"


def cell(text):
    return str(text).replace("|", "\\|").replace("\n", " ")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--manifest", type=Path, help="formal captured units; include incomplete/unstarted units in denominator")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    units = []
    failures = []
    intrinsics = []
    evidence = []
    selected_units = {}
    for index in args.root.glob("*/index.json"):
        entries = json.loads(index.read_text())
        selected_units[index.parent.name] = {Path(entry["unit"]).name for entry in entries}
        evidence.append(dict(path=str(index.resolve()), sha256=hashlib.sha256(index.read_bytes()).hexdigest()))
    expected = {}
    active_projects = {p.name for p in args.root.iterdir() if p.is_dir()} | set(selected_units)
    if args.manifest:
        entries = json.loads(args.manifest.read_text())
        evidence.append(dict(path=str(args.manifest.resolve()), sha256=hashlib.sha256(args.manifest.read_bytes()).hexdigest()))
        for entry in entries:
            project = entry["project"]
            if project not in active_projects:
                continue
            unit = entry["crate"] + "-" + Path(entry["capture"]).stem.split("-")[-1]
            expected[project, unit] = entry
        for project in active_projects:
            selected_units[project] = {unit for proj, unit in expected if proj == project}
    # Each invocation denotes a target unit, including feature/version duplicates.
    for invocation in sorted(args.root.rglob("invocation.json")):
        directory = invocation.parent
        project = directory.parent.name
        if project in selected_units and directory.name not in selected_units[project]:
            continue
        inv = json.loads(invocation.read_text())
        reports = list((directory / "reports").glob("*.jsonl"))
        environment = inv.get("env", {})
        package = environment.get("CARGO_PKG_NAME", directory.name)
        version = environment.get("CARGO_PKG_VERSION", "")
        rows = [json.loads(line) for report in reports for line in report.read_text().splitlines() if line.strip()]
        summaries = [r for r in rows if r["kind"] == "summary" and r["status"] == "complete"]
        items = [r for r in rows if r["kind"] in ("function", "static", "global_asm", "entry")]
        errors = [r for r in items if r["status"] == "error"]
        expected_summary = f"items={len(items)} failed={len(errors)}"
        complete = (len(reports) == 1 and len(summaries) == 1
                    and summaries[0]["message"] == expected_summary)
        unit = dict(unit=directory.name, project=project, package=package,
                    version=version, complete=complete, items=len(items), failed=len(errors),
                    reports=[str(p.resolve()) for p in reports],
                    machine_opt=environment.get("KFRC_CRANELIFT_OPT", "speed"),
                    backend_sha256=inv.get("backend_sha256"))
        units.append(unit)
        for report in reports:
            evidence.append(dict(path=str(report.resolve()), sha256=hashlib.sha256(report.read_bytes()).hexdigest()))
        for row in errors:
            failures.append(dict(project=unit["project"], unit=unit["unit"], package=package,
                                 version=version, machine_opt=unit["machine_opt"], kind=row["kind"], item=row["item"], symbol=row["symbol"],
                                 category=category(row["message"]), reason=row["message"].splitlines()[0],
                                 message=row["message"], report=str(reports[0].resolve())))
        for row in rows:
            if row["kind"] == "intrinsic":
                intrinsics.append(dict(project=unit["project"], unit=unit["unit"], package=package,
                                       version=version, item=row["item"], intrinsic=row["message"]))
    seen_units = {(u["project"], u["unit"]) for u in units}
    for (project, unit), entry in sorted(expected.items()):
        if (project, unit) not in seen_units:
            units.append(dict(unit=unit, project=project, package=entry["package"],
                              version=entry["version"], complete=False, items=0, failed=0, reports=[]))
    for name, records in (("failures", failures), ("intrinsics", intrinsics), ("units", units)):
        (args.output / (name + ".json")).write_text(json.dumps(records, indent=2, ensure_ascii=False) + "\n")
    (args.output / "evidence-sha256.json").write_text(json.dumps(evidence, indent=2) + "\n")
    if failures:
        columns = [k for k in failures[0] if k != "message"]
        with (args.output / "failures.csv").open("w", newline="") as handle:
            writer = csv.DictWriter(handle, fieldnames=columns, extrasaction="ignore")
            writer.writeheader()
            writer.writerows(failures)
    lines = ["# 全函数调查分类", "", "计数单位为单态化函数/静态项的首个失败；intrinsic观察单列，不能相加当作失败总数。", ""]
    for project in sorted({u["project"] for u in units}):
        selected_units = [u for u in units if u["project"] == project]
        selected = [r for r in failures if r["project"] == project]
        complete = sum(u["complete"] for u in selected_units)
        lines += [f"## {project}", "", f"{complete}/{len(selected_units)} 个target units报告完整；{sum(u['items'] for u in selected_units)}个项，{len(selected)}个首错。", "",
                  "| 缺口 | 首错项数 | 涉及target units | 涉及package versions | 代表函数 |", "|---|---:|---:|---:|---|"]
        configurations = Counter(u.get("machine_opt") for u in selected_units if u["complete"])
        lines += ["完整报告的Cranelift配置：" + ", ".join(f"{mode}={count} units" for mode, count in sorted(configurations.items())), ""]
        counts = Counter(r["category"] for r in selected)
        for kind, count in counts.most_common():
            group = [r for r in selected if r["category"] == kind]
            lines.append(f"| {kind} | {count} | {len({r['unit'] for r in group})} | {len({(r['package'], r['version']) for r in group})} | {cell(group[0]['item'][:180])} |")
        lines += ["", "每类涉及的crate与代表错误：", ""]
        for kind in counts:
            group = [r for r in selected if r["category"] == kind]
            crates = sorted({r["package"] + "@" + r["version"] for r in group})
            lines += [f"- **{kind}**：{', '.join(crates)}。代表：`{cell(group[0]['reason'][:300])}`。"]
        lines += ["", "### 精确错误签名", "", "同类别的不同失败层/错误签名单列，尤其保留SoN与Cranelift panic差异。", "", "| 错误签名 | 项数 | target units | 代表crate / 函数 |", "|---|---:|---:|---|"]
        signatures = Counter(r["reason"] for r in selected)
        for reason, count in signatures.most_common():
            group = [r for r in selected if r["reason"] == reason]
            example = group[0]["package"] + "@" + group[0]["version"] + " / " + group[0]["item"][:150]
            lines.append(f"| {cell(reason)} | {count} | {len({r['unit'] for r in group})} | {cell(example)} |")
        observed = [r for r in intrinsics if r["project"] == project]
        lines += ["", "### MIR intrinsic调用观察（包含受更早错误遮挡的调用）", "", "| intrinsic | 调用点数 | target units |", "|---|---:|---:|"]
        for name, count in Counter(r["intrinsic"] for r in observed).most_common():
            lines.append(f"| {name} | {count} | {len({r['unit'] for r in observed if r['intrinsic'] == name})} |")
        lines += ["", "### 只移除一种首错后的候选范围", "", "下表只按已观察首错计算；同函数的后续缺口和crate级变换仍可能阻止编译。", "", "| 唯一首错类别 | target units |", "|---|---|"]
        by_unit = defaultdict(set)
        for row in selected:
            by_unit[row["unit"]].add(row["category"])
        for kind in counts:
            names = sorted(unit for unit, categories in by_unit.items() if categories == {kind})
            if names:
                lines.append(f"| {kind} | {', '.join(names)} |")
        incomplete = [u["unit"] for u in selected_units if not u["complete"]]
        if incomplete:
            lines += ["", "未完成units：" + ", ".join(incomplete)]
        lines.append("")
    (args.output / "classification.md").write_text("\n".join(lines))
    print(json.dumps(dict(units=len(units), complete=sum(u["complete"] for u in units),
                          items=sum(u["items"] for u in units), failures=len(failures), intrinsic_calls=len(intrinsics))))


if __name__ == "__main__":
    main()
