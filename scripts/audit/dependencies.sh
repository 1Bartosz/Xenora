#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu
cd "$(dirname "$0")/../.."
scratch=$(mktemp)
trap 'rm -f "$scratch"' EXIT
cargo metadata --format-version 1 --locked > "$scratch"
python3 - "$scratch" <<'PY'
import json
import re
import sys

with open(sys.argv[1], encoding="utf-8") as metadata:
    graph = json.load(metadata)
packages = {package["id"]: package["name"] for package in graph["packages"]}
nodes = {node["id"]: node["dependencies"] for node in graph["resolve"]["nodes"]}
graphics = {"isa", "decoder", "ref", "ir", "shader", "msl", "metal", "gpu", "memory"}
reference_forbidden = graphics - {"ref"}
forbidden_cpu = re.compile(r"(^|[-_])(x86(?:[-_]?64)?|iced|zydis|cpu)([-_]|$)")
violations = []
checked = 0

for root in graph["workspace_members"]:
    name = packages[root]
    component = name.removeprefix("xenora-")
    if component not in graphics:
        continue
    checked += 1
    pending = list(nodes[root])
    seen = set()
    while pending:
        dependency = pending.pop()
        if dependency in seen:
            continue
        seen.add(dependency)
        target = packages[dependency]
        if forbidden_cpu.search(target) or (
            component == "ref"
            and target.removeprefix("xenora-") in reference_forbidden
        ):
            violations.append(f"{name} depends on {target}")
        pending.extend(nodes[dependency])

for violation in sorted(set(violations)):
    print(violation, file=sys.stderr)
if not checked:
    sys.exit("No graphics crates were present in the resolved Cargo graph.")
if violations:
    sys.exit(1)
print(f"Checked transitive dependency direction for {checked} graphics crates.")
PY
