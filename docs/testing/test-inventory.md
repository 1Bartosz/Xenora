# Compare test inventories across commits

Run this check from a Xenora checkout with Python 3.11 or later and the
native ARM64 Rust toolchain. Choose the base commit whose tests the candidate
should preserve. Comparing against `main` can catch tests missing from a
branch that predates their merge.

```sh
python3 scripts/audit/test_inventory.py main HEAD
python3 scripts/audit/test_inventory.py main HEAD --allow intended-losses.tsv
python3 scripts/audit/test_inventory.py --self-test
```

The script resolves both refs to commits, then uses a temporary detached Git
worktree for each commit. It builds the workspace with the lockfile and
default features in offline mode, using Cargo's cached dependencies. It lists
tests and then lists ignored tests using the same build. It removes each
temporary worktree after listing, including when a build fails. Git needs
permission to update its worktree registry.

Each test is identified by `package/kind:target/test_name`. The report shows
removed, added, newly ignored and newly un-ignored tests. Renames appear as a
removal and an addition. Exit code 1 means at least one unexplained removal or
newly ignored test. Exit code 0 means every loss was explained or there were
no losses. Exit code 2 means the check could not complete, for example because
a ref did not resolve or Cargo could not build it.

To explain an intended removal or a newly ignored test, copy its exact key
from the report into a UTF-8 file, followed by a literal tab and a reason.
Use one entry per line. Blank lines and lines starting with `#` are allowed.

```text
xenora-texture/test:bc_formats/old_case	Replaced by new_case, which covers both endpoint orders.
```

The report still shows explained losses. Keep the reasons specific enough for
a reviewer to verify.

The inventory covers Cargo's default workspace test targets for the host and
build configuration. Feature-gated targets such as those requiring
`gpu-tests` are outside that inventory. The check lists tests without
running their bodies or verifying CPU or GPU behavior. Benchmarks are
excluded. Rustdoc includes source positions in doctest names, so moving a
doctest can appear as a removal and an addition.
