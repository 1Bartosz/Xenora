#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later

"""Render the feature and instruction tables from the checked-in status registry."""

import argparse
from pathlib import Path
import sys
import tomllib


def table(headers, rows):
    def line(cells):
        return "| " + " | ".join(str(cell).replace("|", r"\|") for cell in cells) + " |"

    return "\n".join([line(headers), line(["---"] * len(headers)), *(line(row) for row in rows)])


def replace_block(path, name, body, check):
    original = path.read_text()
    start = f"<!-- status:{name}:start -->\n"
    stop = f"\n<!-- status:{name}:end -->"
    head, _, tail = original.partition(start)
    if not tail or stop not in tail:
        raise ValueError(f"{path}: missing status markers")
    updated = head + start + body + stop + tail.split(stop, 1)[1]
    if updated == original:
        return True
    if check:
        print(f"{path}: status table differs from docs/status.toml", file=sys.stderr)
        return False
    path.write_text(updated)
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    check = parser.parse_args().check
    root = Path(__file__).resolve().parents[2]
    docs = root / "docs"
    status = tomllib.loads((docs / "status.toml").read_text())
    claims = tomllib.loads((docs / "claims.toml").read_text())
    classes = {claim["id"]: claim["class"] for claim in claims["claim"]}

    features = table(
        ("Feature", "Target", "Implementation", "Correctness", "Hardware", "Evidence", "Limits"),
        [
            (
                feature["id"],
                feature["architecture"],
                feature["implementation"],
                feature["correctness"],
                feature["hardware_verification"] + (
                    f" on {feature['hardware_target']}" if "hardware_target" in feature else ""
                ),
                ", ".join(sorted({classes[claim] for claim in feature["claims"]})),
                feature["limitations"],
            )
            for feature in status["feature"]
        ],
    )
    instructions = table(
        ("Form", "Operands", "Size or word", "Recognized", "Decoded", "LLVM",
         "Lowering", "Reference", "CPU", "GPU"),
        [
            (
                f"`{item['form']}`",
                item["encoding"],
                item["representation"],
                "yes" if item["recognized"] else "no",
                "yes" if item["decoded"] else "no",
                item["llvm"],
                item["lowering"],
                item["reference"],
                item["cpu"],
                item["gpu"],
            )
            for item in status["instruction"]
        ],
    )
    checks = [
        replace_block(root / "README.md", "features", features, check),
        replace_block(docs / "shaders/instruction-coverage.md", "instructions", instructions, check),
    ]
    return 0 if all(checks) else 1


if __name__ == "__main__":
    sys.exit(main())
