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
| _(filled as commits land)_ |  |  |

## Smoke repository

- Repo: `ranveersoni98/xenora-agent-queue-smoke`
- See `agent-queue-smoke.md` for Actions evidence

## Limitations

- Production agent PR checks remain advisory
- Independent `/verified` on smoke PRs needs a second eligible account
- New QA/release tools are local/advisory until review
