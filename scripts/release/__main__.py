# SPDX-License-Identifier: GPL-3.0-or-later
"""CLI entry: python3 -m release <command> ..."""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

from .bundle import check_app_bundle, diagnose_codesign_output
from .macho import check_archs, check_otool_linkage, consistent_deployment_targets
from .manifest import build_manifest, dumps_manifest, license_inventory
from .readiness import (
    build_report,
    build_sbom,
    collect_artifacts,
    dumps_report,
    inspect_app,
    inspect_binary,
    require_evidence_files,
)


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    p = argparse.ArgumentParser(prog="release")
    sub = p.add_subparsers(dest="command", required=True)

    a = sub.add_parser("check-arch")
    a.add_argument("--path", required=True)
    a.add_argument("--lipo", required=True, help="lipo -archs output")

    l = sub.add_parser("check-linkage")
    l.add_argument("--path", required=True)
    l.add_argument("--otool", required=True, help="otool -L output text file or - for stdin")

    b = sub.add_parser("check-bundle")
    b.add_argument("--app", required=True)

    m = sub.add_parser("manifest")
    m.add_argument("--revision", required=True)
    m.add_argument("paths", nargs="+")

    lic = sub.add_parser("licenses")
    lic.add_argument("--entry", action="append", default=[], help="name:license")

    ready = sub.add_parser("readiness", help="build a release-readiness report")
    ready.add_argument("--revision", required=True)
    ready.add_argument("--app", default="")
    ready.add_argument("--binary", default="")
    ready.add_argument("--lipo", default="")
    ready.add_argument("--otool-l", default="")
    ready.add_argument("--otool-header", default="")
    ready.add_argument("--codesign", default="")
    ready.add_argument("--artifact", action="append", default=[])
    ready.add_argument("--sbom-entry", action="append", default=[], help="name:version:license")
    ready.add_argument("--evidence", action="append", default=[], help="required evidence file path")
    ready.add_argument("--live", action="store_true", help="run lipo/otool/codesign on --binary/--app")

    args = p.parse_args(argv)

    if args.command == "check-arch":
        result = check_archs(args.path, args.lipo)
        json.dump({"path": result.path, "archs": list(result.archs), "ok": result.ok,
                   "problems": list(result.problems)}, sys.stdout, indent=2)
        print()
        return 0 if result.ok else 1

    if args.command == "check-linkage":
        text = sys.stdin.read() if args.otool == "-" else Path(args.otool).read_text(encoding="utf-8")
        result = check_otool_linkage(args.path, text)
        json.dump({"path": result.path, "ok": result.ok, "problems": list(result.problems)}, sys.stdout, indent=2)
        print()
        return 0 if result.ok else 1

    if args.command == "check-bundle":
        result = check_app_bundle(Path(args.app))
        json.dump({"root": result.root, "ok": result.ok, "problems": list(result.problems),
                   "present": list(result.present)}, sys.stdout, indent=2)
        print()
        return 0 if result.ok else 1

    if args.command == "manifest":
        print(dumps_manifest(build_manifest([Path(p) for p in args.paths], revision=args.revision)), end="")
        return 0

    if args.command == "licenses":
        entries = []
        for item in args.entry:
            name, lic = item.split(":", 1)
            entries.append((name, lic))
        print(license_inventory(entries), end="")
        return 0

    if args.command == "readiness":
        binary_findings = []
        app_findings = []
        if args.live and args.binary:
            lipo = subprocess.check_output(["lipo", "-archs", args.binary], text=True)
            otool_l = subprocess.check_output(["otool", "-L", args.binary], text=True)
            header = subprocess.check_output(["otool", "-l", args.binary], text=True)
            binary_findings = inspect_binary(
                path=args.binary, lipo_archs=lipo, otool_l=otool_l, otool_header=header
            )
        elif args.binary:
            binary_findings = inspect_binary(
                path=args.binary,
                lipo_archs=args.lipo,
                otool_l=Path(args.otool_l).read_text(encoding="utf-8") if args.otool_l else "",
                otool_header=Path(args.otool_header).read_text(encoding="utf-8") if args.otool_header else "",
            )
        if args.app:
            codesign_out = args.codesign
            if args.live and not codesign_out:
                codesign_out = subprocess.run(
                    ["codesign", "-dv", "--verbose=2", args.app],
                    capture_output=True, text=True,
                ).stderr
            app_findings = inspect_app(Path(args.app), codesign_out)
        artifacts, artifact_findings = collect_artifacts(
            [Path(p) for p in args.artifact], revision=args.revision
        ) if args.artifact else ([], [])
        sbom_entries = []
        for item in args.sbom_entry:
            name, version, lic = item.split(":", 2)
            sbom_entries.append((name, version, lic))
        evidence_findings = require_evidence_files([Path(p) for p in args.evidence])
        report = build_report(
            revision=args.revision,
            binary_findings=binary_findings,
            app_findings=app_findings,
            artifacts=artifacts,
            artifact_findings=artifact_findings,
            evidence_findings=evidence_findings,
            sbom=build_sbom(sbom_entries),
        )
        print(dumps_report(report), end="")
        return 0 if report.ok else 1

    return 2


if __name__ == "__main__":
    raise SystemExit(main())
