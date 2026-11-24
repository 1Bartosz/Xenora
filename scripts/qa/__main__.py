# SPDX-License-Identifier: GPL-3.0-or-later
"""CLI entry: python3 -m qa <command> ..."""
from __future__ import annotations

import argparse
import json
import shlex
import subprocess
import sys
from pathlib import Path

from .bench_regress import IncompatibleComparison, compare_records, dumps_comparison, load_record
from .bench_stats import export_json, human_report, summarize
from .claim_conflict import ClaimSnapshot, format_conflicts, predict_conflicts
from .cpu_coverage import blocker_manifest, build_report as build_cpu_report, compare_reports, dumps_report as dumps_cpu
from .env_check import (
    check_branch_sync,
    check_git_worktree_clean,
    check_llvm_tool,
    check_rosetta_translated,
    check_rustc_host,
    check_swift_version,
    check_uname_machine,
    format_report,
    summarize as summarize_env,
)
from .git_scope import colliding_scopes, effective_paths, parse_name_status
from .orchestrate import SuiteSpec, dumps_report, failure_log_excerpt, run_suites
from .pr_validate import (
    check_artifacts,
    check_changelog_required,
    check_dco,
    check_report_commit_identity,
    check_scope,
    merge_reports,
)
from .rdna2_plan import (
    classify_unsupported,
    coverage_diff,
    discrepancies,
    dumps_candidates,
    dumps_inventory,
    load_inventories,
    plan_batches,
    summarize_inventory,
)
from .verify import dumps_evidence, evidence_is_stale, load_evidence, run_profile


def _root() -> Path:
    return Path(__file__).resolve().parents[2]


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    p = argparse.ArgumentParser(prog="qa")
    sub = p.add_subparsers(dest="command", required=True)

    o = sub.add_parser("orchestrate", help="run command suites and emit JSON")
    o.add_argument("--name", action="append", default=[])
    o.add_argument("--run", action="append", default=[], dest="runs",
                   help="shell argv string for one suite")
    o.add_argument("--kind", action="append", default=[])
    o.add_argument("--stop-on-fail", action="store_true")
    o.add_argument("--failures", action="store_true")

    b = sub.add_parser("bench-stats", help="summarize numeric samples")
    b.add_argument("samples", nargs="+", type=float)
    b.add_argument("--metric", default="latency")
    b.add_argument("--json", action="store_true")
    b.add_argument("--host", default="unknown")
    b.add_argument("--toolchain", default="unknown")
    b.add_argument("--revision", default="unknown")

    br = sub.add_parser("bench-compare", help="compare two recorded benchmark JSON files")
    br.add_argument("--baseline", required=True)
    br.add_argument("--current", required=True)
    br.add_argument("--threshold", type=float, default=1.10)

    c = sub.add_parser("predict-conflicts", help="read-only claim overlap check")
    c.add_argument("--candidate", action="append", default=[])
    c.add_argument("--held", action="append", default=[], help="number:holder:path[,path...]")

    g = sub.add_parser("scope-collisions", help="collisions from git name-status input")
    g.add_argument("--claimed", action="append", default=[], help="claimed path pattern")
    g.add_argument("--name-status-file", default="-")

    v = sub.add_parser("validate-pr", help="advisory PR validation from JSON stdin")
    v.add_argument("--allowed", action="append", default=[])

    r = sub.add_parser("rdna2", help="read-only RDNA2 expansion planning")
    r.add_argument("action", choices=("inventory", "candidates", "plan", "discrepancies", "diff"))
    r.add_argument("--limit", type=int, default=0)
    r.add_argument("--before", default="", help="JSON inventory for diff")
    r.add_argument("--after", default="", help="JSON inventory for diff")

    cpu = sub.add_parser("cpu-coverage", help="read-only CPU/runtime coverage scan")
    cpu.add_argument("--blockers", action="store_true")
    cpu.add_argument("--compare", default="", help="previous JSON report path")

    ver = sub.add_parser("verify", help="run a verification profile")
    ver.add_argument("profile", choices=("quick", "cpu", "gpu", "app", "full", "release"))
    ver.add_argument("--revision", default="")
    ver.add_argument("--no-stop", action="store_true")
    ver.add_argument("--check-stale", default="", help="existing evidence JSON to staleness-check")

    env = sub.add_parser("env", help="environment diagnostics from live commands")

    args = p.parse_args(argv)
    root = _root()

    if args.command == "orchestrate":
        if not args.name or len(args.name) != len(args.runs):
            print("orchestrate requires matching --name and --run lists", file=sys.stderr)
            return 2
        specs = []
        for i, name in enumerate(args.name):
            kind = args.kind[i] if i < len(args.kind) else "cpu"
            specs.append(SuiteSpec(name, tuple(shlex.split(args.runs[i])), kind))
        report = run_suites(specs, stop_on_fail=args.stop_on_fail)
        if args.failures:
            print(failure_log_excerpt(report), end="")
        else:
            print(dumps_report(report), end="")
        return report.overall_returncode()

    if args.command == "bench-stats":
        stats = summarize(args.samples)
        if args.json:
            print(export_json(stats, host=args.host, toolchain=args.toolchain,
                              revision=args.revision, metric=args.metric), end="")
        else:
            print(human_report(stats, metric=args.metric), end="")
        return 0

    if args.command == "bench-compare":
        baseline = load_record(json.loads(Path(args.baseline).read_text(encoding="utf-8")))
        current = load_record(json.loads(Path(args.current).read_text(encoding="utf-8")))
        try:
            payload = compare_records(baseline, current, threshold=args.threshold)
        except IncompatibleComparison as exc:
            print(str(exc), file=sys.stderr)
            return 2
        print(dumps_comparison(payload), end="")
        return 1 if payload["regressing"] else 0

    if args.command == "predict-conflicts":
        held = []
        for item in args.held:
            number, holder, paths = item.split(":", 2)
            held.append(ClaimSnapshot(int(number), holder, tuple(paths.split(",")), True))
        hits = predict_conflicts(args.candidate, held)
        print(format_conflicts(hits), end="")
        return 1 if hits else 0

    if args.command == "scope-collisions":
        text = sys.stdin.read() if args.name_status_file == "-" else Path(args.name_status_file).read_text(encoding="utf-8")
        changes = parse_name_status(text.splitlines())
        paths = sorted(effective_paths(changes))
        hits = colliding_scopes(paths, args.claimed)
        json.dump({"paths": paths, "collisions": [{"path": a, "pattern": b} for a, b in hits]}, sys.stdout, indent=2)
        print()
        return 1 if hits else 0

    if args.command == "validate-pr":
        payload = json.load(sys.stdin)
        report = merge_reports(
            check_scope(payload.get("files", []), args.allowed),
            check_dco(payload.get("messages", [])),
            check_artifacts(payload.get("files", [])),
            check_changelog_required(payload.get("files", [])),
            check_report_commit_identity(payload.get("report_sha", ""), payload.get("head_sha", "")),
        )
        json.dump({"ok": report.ok, "findings": [f.__dict__ for f in report.findings]}, sys.stdout, indent=2)
        print()
        return 0 if report.ok else 1

    if args.command == "rdna2":
        if args.action == "diff":
            before = json.loads(Path(args.before).read_text(encoding="utf-8"))
            after = json.loads(Path(args.after).read_text(encoding="utf-8"))
            from .rdna2_plan import Inventory
            def inv(d):
                return Inventory(**{k: d[k] for k in (
                    "identified", "declared", "decoder_supported", "reference_verified",
                    "metal_verified", "unsupported", "by_family")})
            print(json.dumps(coverage_diff(inv(before), inv(after)), indent=2, sort_keys=True))
            return 0
        data = load_inventories(root)
        inv = summarize_inventory(data)
        if args.action == "inventory":
            print(dumps_inventory(inv), end="")
            return 0
        if args.action == "candidates":
            limit = args.limit or None
            print(dumps_candidates(classify_unsupported(data, limit=limit)), end="")
            return 0
        if args.action == "plan":
            cands = classify_unsupported(data, limit=(args.limit or None))
            print(json.dumps(plan_batches(cands), indent=2, sort_keys=True))
            return 0
        if args.action == "discrepancies":
            print(json.dumps(discrepancies(data), indent=2, sort_keys=True))
            return 0

    if args.command == "cpu-coverage":
        report = build_cpu_report(root)
        if args.compare:
            prev = json.loads(Path(args.compare).read_text(encoding="utf-8"))
            print(json.dumps(compare_reports(prev, report.to_dict()), indent=2, sort_keys=True))
            return 0
        if args.blockers:
            print(json.dumps(blocker_manifest(report), indent=2, sort_keys=True))
            return 0
        print(dumps_cpu(report), end="")
        return 0

    if args.command == "verify":
        if args.check_stale:
            evidence = load_evidence(Path(args.check_stale).read_text(encoding="utf-8"))
            stale = evidence_is_stale(evidence, root)
            json.dump({"stale": stale, "fingerprint": evidence.source_fingerprint}, sys.stdout, indent=2)
            print()
            return 1 if stale else 0
        revision = args.revision or subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=root, text=True
        ).strip()
        evidence = run_profile(
            args.profile, root=root, revision=revision, stop_on_fail=not args.no_stop
        )
        print(dumps_evidence(evidence), end="")
        return evidence.overall_returncode

    if args.command == "env":
        machine = subprocess.check_output(["uname", "-m"], text=True)
        translated = subprocess.check_output(
            ["sysctl", "-n", "sysctl.proc_translated"], text=True
        )
        rustc = subprocess.check_output(["rustc", "-vV"], text=True)
        swift = subprocess.run(["swift", "--version"], capture_output=True, text=True).stdout
        clang_path = subprocess.run(["which", "clang"], capture_output=True, text=True).stdout
        clang_ver = subprocess.run(["clang", "--version"], capture_output=True, text=True).stdout
        porcelain = subprocess.check_output(["git", "status", "--porcelain"], cwd=root, text=True)
        findings = [
            check_uname_machine(machine),
            check_rosetta_translated(translated),
            check_rustc_host(rustc),
            check_swift_version(swift),
            check_llvm_tool(clang_path, clang_ver),
            check_git_worktree_clean(porcelain),
            check_branch_sync(ahead=0, behind=0),
        ]
        report = summarize_env(findings)
        print(format_report(report), end="")
        return 0 if report.ok else 1

    return 2


if __name__ == "__main__":
    raise SystemExit(main())
