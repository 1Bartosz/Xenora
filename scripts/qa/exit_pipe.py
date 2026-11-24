# SPDX-License-Identifier: GPL-3.0-or-later
"""Run a command and preserve its exit status through pipes.

Never report success because a later pipe stage (grep, head) succeeded while
the measured command failed.
"""
from __future__ import annotations

import subprocess
from dataclasses import dataclass
from typing import Sequence


@dataclass(frozen=True)
class PipedResult:
    command: tuple[str, ...]
    returncode: int
    stdout: str
    stderr: str


def run_preserving_status(
    command: Sequence[str],
    *,
    cwd: str | None = None,
    timeout: float | None = None,
) -> PipedResult:
    """Run `command` with check disabled and return its real exit status."""
    completed = subprocess.run(
        list(command),
        cwd=cwd,
        capture_output=True,
        text=True,
        timeout=timeout,
        check=False,
    )
    return PipedResult(
        command=tuple(command),
        returncode=int(completed.returncode),
        stdout=completed.stdout,
        stderr=completed.stderr,
    )


def classify_status(returncode: int) -> str:
    if returncode == 0:
        return "passed"
    return "failed"
