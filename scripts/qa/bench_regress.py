# SPDX-License-Identifier: GPL-3.0-or-later
"""Benchmark regression intelligence over recorded results.

Rejects incompatible comparisons and requires actual sample data.
"""
from __future__ import annotations

import json
import math
from dataclasses import asdict, dataclass
from typing import Any, Sequence

from .bench_stats import SampleStats, compare_medians, summarize


@dataclass(frozen=True)
class BenchIdentity:
    metric: str
    unit: str
    workload: str
    host: str
    toolchain: str


@dataclass(frozen=True)
class BenchRecord:
    identity: BenchIdentity
    samples: tuple[float, ...]
    revision: str

    def stats(self) -> SampleStats:
        return summarize(self.samples)


class IncompatibleComparison(ValueError):
    pass


def identities_compatible(a: BenchIdentity, b: BenchIdentity) -> bool:
    return (
        a.metric == b.metric
        and a.unit == b.unit
        and a.workload == b.workload
        and a.host == b.host
        and a.toolchain == b.toolchain
    )


def compare_records(
    baseline: BenchRecord,
    current: BenchRecord,
    *,
    threshold: float = 1.10,
    min_samples: int = 3,
    noise_floor_ratio: float = 0.03,
) -> dict[str, Any]:
    if not identities_compatible(baseline.identity, current.identity):
        raise IncompatibleComparison(
            "refusing comparison across different workload/unit/host/toolchain/metric"
        )
    if len(baseline.samples) < min_samples or len(current.samples) < min_samples:
        raise ValueError(f"need at least {min_samples} samples on each side")
    result = compare_medians(baseline.samples, current.samples,
                             metric=baseline.identity.metric, threshold=threshold)
    base_stats = baseline.stats()
    cur_stats = current.stats()
    # Treat changes within noise_floor_ratio of the baseline median as noise.
    noise = abs(base_stats.median) * noise_floor_ratio
    delta = cur_stats.median - base_stats.median
    meaningful = abs(delta) > max(noise, math.ulp(base_stats.median) * 8)
    return {
        "identity": asdict(baseline.identity),
        "baseline_revision": baseline.revision,
        "current_revision": current.revision,
        "baseline_stats": asdict(base_stats),
        "current_stats": asdict(cur_stats),
        "ratio": result.ratio,
        "threshold": threshold,
        "regressing": bool(result.regressing and meaningful),
        "meaningful": meaningful,
        "noise_floor": noise,
    }


def load_record(data: dict[str, Any]) -> BenchRecord:
    ident = data["identity"]
    samples = data.get("samples")
    if not isinstance(samples, list) or not samples:
        raise ValueError("record requires a non-empty samples list; refusing fabricated data")
    return BenchRecord(
        identity=BenchIdentity(
            metric=ident["metric"],
            unit=ident["unit"],
            workload=ident["workload"],
            host=ident["host"],
            toolchain=ident["toolchain"],
        ),
        samples=tuple(float(x) for x in samples),
        revision=str(data["revision"]),
    )


def dumps_comparison(payload: dict[str, Any]) -> str:
    return json.dumps(payload, indent=2, sort_keys=True) + "\n"
