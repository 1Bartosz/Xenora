# Infrastructure QA campaign ledger

Running record for the infrastructure and engineering-quality campaign.
Update after each meaningful commit batch. Do not invent passing tests.

## Baseline

- Branch: `main` synchronized with `origin/main` before each batch
- Hard exclusions: CPU/runtime crates, RDNA2 crates, `app/`, fixtures, GPU tests
- Approved paths: see `infrastructure-qa-ownership.md`

## Commits

| SHA | Summary | Tests |
| --- | --- | --- |
| `dcc6f7a` | fix(agent-queue): skip no-op gh issue edit on comment-only actions. | focused suite before commit |
| `261493b` | test(agent-queue): add read-only claim conflict prediction coverage. | focused suite before commit |
| `87a5ada` | docs(coordination): record infrastructure QA ownership manifest. | focused suite before commit |
| `7fa9820` | fix(agent-queue): accept summary --now after the subcommand. | focused suite before commit |
| `f902617` | fix(ci): grant agent-queue audit pull-requests:read. | focused suite before commit |
| `f5c72a4` | docs(coordination): record agent-queue disposable smoke evidence. | focused suite before commit |
| `a10ac43` | docs(coordination): add infrastructure QA campaign ledger. | focused suite before commit |
| `8b5cee5` | feat(qa): preserve command exit status for orchestration pipes. | focused suite before commit |
| `1b555a5` | feat(qa): add structured multi-suite test orchestration reports. | focused suite before commit |
| `19879c8` | feat(qa): add advisory PR scope, DCO and artifact validators. | focused suite before commit |
| `f772081` | feat(qa): analyze existing benchmark samples for percentiles. | focused suite before commit |
| `53f73a8` | feat(qa): compare orchestration history and emit handoff reports. | focused suite before commit |
| `fc1587e` | feat(qa): add Apple Silicon environment diagnostic helpers. | focused suite before commit |
| `8e039d9` | feat(qa): predict claim file-scope conflicts before editing. | focused suite before commit |
| `f6350ad` | feat(release): verify ARM64 Mach-O arch, linkage and app bundles. | focused suite before commit |
| `eaf78a2` | feat(release): build checksum manifests and license inventories. | focused suite before commit |
| `8694751` | docs(testing): index infrastructure QA and release tooling. | focused suite before commit |
| `27c3d28` | docs(changelog): note infrastructure QA tooling and agent-queue smoke fixes. | focused suite before commit |

## Smoke repository

- Repo: `ranveersoni98/xenora-agent-queue-smoke`
- See `agent-queue-smoke.md` for Actions evidence

## Limitations

- Production agent PR checks remain advisory
- Independent `/verified` on smoke PRs needs a second eligible account
- New QA/release tools are local/advisory until review
