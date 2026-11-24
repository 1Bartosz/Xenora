# Infrastructure QA tooling

Standalone helpers under `scripts/qa/` and `scripts/release/`. They do not replace
`scripts/audit/*`, `scripts/benchmark.sh` or `scripts/test-gpu.sh`. New checks are
advisory until reviewed.

## Suites

| Path | Role |
| --- | --- |
| `scripts/qa/exit_pipe.py` | Preserve command exit status |
| `scripts/qa/orchestrate.py` | Structured multi-suite runs and failure excerpts |
| `scripts/qa/pr_validate.py` | Advisory PR scope, DCO, artifact and evidence checks |
| `scripts/qa/bench_stats.py` | Median/percentile analysis of existing samples |
| `scripts/qa/regression.py` | Compare historical orchestration JSON reports |
| `scripts/qa/env_check.py` | Host, toolchain and branch diagnostics |
| `scripts/qa/claim_conflict.py` | Read-only file-scope conflict prediction |
| `scripts/release/macho.py` | ARM64 arch and forbidden-linkage checks |
| `scripts/release/bundle.py` | App bundle layout and codesign diagnosis |
| `scripts/release/manifest.py` | Checksums, manifests, license inventory |

## Tests

```sh
PYTHONPATH=scripts python3 -m unittest discover -s tests/qa
PYTHONPATH=scripts python3 -m unittest discover -s tests/release
```
