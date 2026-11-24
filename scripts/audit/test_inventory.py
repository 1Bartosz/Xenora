#!/usr/bin/env python3
"""Compare Cargo test inventories at two Git commits."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest

RUNNING = re.compile(r"Running (?:unittests )?.+ \((.+)\)")
DOCTEST = re.compile(r"Doc-tests (\S+)")
TEST = re.compile(r"(.+): test")


def parse_list(output, metadata, checkout):
    """Return (package, kind:target, test name) keys from a merged Cargo log."""
    packages = {p["id"]: p["name"] for p in metadata["packages"]}
    doctests = {}
    for package in metadata["packages"]:
        for target in package["targets"]:
            if target["doctest"]:
                name = target["name"].replace("-", "_")
                doctests.setdefault(name, set()).add(
                    (package["name"], f'doc:{target["name"]}')
                )

    executables = {}
    suites = set()
    tests = set()
    suite = None
    for line in output.splitlines():
        line = line.strip()
        if line.startswith('{"reason":'):
            message = json.loads(line)
            if message["reason"] == "compiler-artifact":
                executable = message["executable"]
                package = packages.get(message["package_id"])
                if executable and package:
                    target = message["target"]
                    kind = target["kind"][0]
                    executables[Path(executable)] = (package, f'{kind}:{target["name"]}')
        elif match := RUNNING.fullmatch(line):
            executable = checkout / match[1]
            suite = executables.get(executable)
            if suite is None:
                raise ValueError(f"no Cargo artifact for {line}")
            suites.add(suite)
        elif match := DOCTEST.fullmatch(line):
            matches = doctests.get(match[1], set())
            if len(matches) != 1:
                raise ValueError(f"cannot identify package for {line}")
            suite = next(iter(matches))
            suites.add(suite)
        elif match := TEST.fullmatch(line):
            if suite is None:
                raise ValueError(f"test listed before its target: {line}")
            tests.add((*suite, match[1]))

    if not suites:
        raise ValueError("Cargo listed no test targets")
    return suites, tests


def run(args, cwd, *, merge=False):
    result = subprocess.run(
        args, cwd=cwd, stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT if merge else subprocess.PIPE,
        text=True, encoding="utf-8",
        env={**os.environ, "CARGO_TERM_COLOR": "never", "GIT_NO_LAZY_FETCH": "1"},
    )
    if result.returncode:
        detail = "\n".join((result.stderr or result.stdout).strip().splitlines()[-25:])
        raise RuntimeError(f"{' '.join(map(str, args))} exited {result.returncode}\n{detail}")
    return result.stdout


def inventory(all_output, ignored_output, metadata, checkout):
    suites, tests = parse_list(all_output, metadata, checkout)
    ignored_suites, ignored = parse_list(ignored_output, metadata, checkout)
    if suites != ignored_suites or not ignored <= tests:
        raise ValueError("Cargo's full and ignored listings disagree")
    return tests, ignored


def collect(repo, commit, checkout):
    run(["git", "worktree", "add", "--detach", str(checkout), commit], repo)
    try:
        metadata = json.loads(run([
            "cargo", "metadata", "--offline", "--locked", "--no-deps", "--format-version", "1",
        ], checkout))
        cargo = [
            "cargo", "test", "--offline", "--locked", "--workspace",
            "--target-dir", str(checkout / "target"),
            "--jobs", "2", "--message-format=json", "--", "--list", "--format", "terse",
        ]
        return inventory(
            run(cargo, checkout, merge=True),
            run([*cargo, "--ignored"], checkout, merge=True),
            metadata,
            checkout,
        )
    finally:
        run(["git", "worktree", "remove", "--force", str(checkout)], repo)


def read_allow(path):
    """Read tab-separated inventory keys and reasons."""
    allowed = {}
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        key, _, reason = line.partition("\t")
        parts = key.split("/", 2)
        if len(parts) != 3 or not all(parts) or not reason.strip() or key in allowed:
            raise ValueError(f"{path}:{number}: expected a unique test key and a tab-separated reason")
        allowed[key] = reason.strip()
    return allowed


def report(base, candidate, allowed):
    before, before_ignored = base
    after, after_ignored = candidate
    common = before & after
    changes = (
        ("Removed", before - after, True),
        ("Added", after - before, False),
        ("Newly ignored", (after_ignored - before_ignored) & common, True),
        ("Newly un-ignored", (before_ignored - after_ignored) & common, False),
    )
    blocked = 0
    for label, tests, is_loss in changes:
        print(f"{label}: {len(tests)}")
        for test in sorted(tests):
            key = "/".join(test)
            reason = allowed.get(key) if is_loss else None
            print(f"  {key}" + (f" (allowed: {reason})" if reason else ""))
            blocked += is_loss and reason is None
    print(f"Unapproved losses: {blocked}")
    return int(blocked != 0)


def main(argv):
    if argv == ["--self-test"]:
        program = unittest.main(argv=[sys.argv[0]], exit=False)
        return int(not program.result.wasSuccessful())

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base", help="base commit (normally main)")
    parser.add_argument("candidate", help="candidate commit")
    parser.add_argument("--allow", type=Path, metavar="FILE", help="explained test losses")
    args = parser.parse_args(argv)
    try:
        allowed = read_allow(args.allow) if args.allow else {}
        repo = Path(run(["git", "rev-parse", "--show-toplevel"], Path.cwd()).strip())
        commits = [
            run(["git", "rev-parse", "--verify", "--end-of-options", f"{ref}^{{commit}}"], repo).strip()
            for ref in (args.base, args.candidate)
        ]
        print(f"Default Cargo features. Base {commits[0]}, candidate {commits[1]}", flush=True)
        with tempfile.TemporaryDirectory(prefix="xenora-test-inventory-") as tmp:
            root = Path(tmp).resolve()
            base = collect(repo, commits[0], root / "base")
            candidate = collect(repo, commits[1], root / "candidate")
        print(f"Base tests: {len(base[0])}; candidate tests: {len(candidate[0])}")
        return report(base, candidate, allowed)
    except (OSError, RuntimeError, ValueError) as error:
        print(f"test inventory: {error}", file=sys.stderr)
        return 2
    except KeyboardInterrupt:
        print("test inventory: interrupted", file=sys.stderr)
        return 2


class ParseListTests(unittest.TestCase):
    checkout = Path("/checkout")
    metadata = {"packages": [
        {"id": package, "name": package, "targets": [{"name": package, "doctest": True}]}
        for package in ("one", "two")
    ]}

    def listing(self, package, cases):
        executable = f"/checkout/target/{package}/shared-testhash"
        message = {
            "reason": "compiler-artifact", "package_id": package,
            "executable": executable, "target": {"name": "shared", "kind": ["test"]},
        }
        return f"{json.dumps(message)}\n Running tests/shared.rs ({executable})\n{cases}\n"

    def test_packages_with_same_target_and_test_names(self):
        output = self.listing("one", "module::case: test\nspeed: benchmark")
        output += self.listing("two", "module::case: test")
        output += " Doc-tests two\nsrc/lib.rs - run (line 12): test\n"
        suites, tests = parse_list(output, self.metadata, self.checkout)
        self.assertEqual(suites, {("one", "test:shared"), ("two", "test:shared"), ("two", "doc:two")})
        self.assertEqual(tests, {
            ("one", "test:shared", "module::case"), ("two", "test:shared", "module::case"),
            ("two", "doc:two", "src/lib.rs - run (line 12)"),
        })

    def test_empty_ignored_target_and_relative_executable(self):
        all_output = self.listing("one", "module::case: test")
        all_output = all_output.replace("(/checkout/target/", "(target/")
        ignored_output = self.listing("one", "")
        self.assertEqual(inventory(all_output, ignored_output, self.metadata, self.checkout),
                         ({("one", "test:shared", "module::case")}, set()))

    def test_rejects_unresolved_target(self):
        with self.assertRaisesRegex(ValueError, "no Cargo artifact"):
            parse_list("Running tests/shared.rs (target/shared)", self.metadata, self.checkout)

    def test_rejects_ignored_test_missing_from_full_list(self):
        with self.assertRaisesRegex(ValueError, "listings disagree"):
            inventory(self.listing("one", ""), self.listing("one", "case: test"),
                      self.metadata, self.checkout)


class ReportTests(unittest.TestCase):
    base = ({("p", "test:t", "kept"), ("p", "test:t", "gone"), ("p", "test:t", "quiet")}, set())
    candidate = ({("p", "test:t", "kept"), ("p", "test:t", "quiet"), ("p", "test:t", "new")},
                 {("p", "test:t", "quiet")})

    def test_losses_fail_unless_explained(self):
        import contextlib, io
        with contextlib.redirect_stdout(io.StringIO()) as out:
            self.assertEqual(report(self.base, self.candidate, {}), 1)
        self.assertIn("Unapproved losses: 2", out.getvalue())
        allowed = {"p/test:t/gone": "replaced by new", "p/test:t/quiet": "needs hardware"}
        with contextlib.redirect_stdout(io.StringIO()) as out:
            self.assertEqual(report(self.base, self.candidate, allowed), 0)
        self.assertIn("p/test:t/gone (allowed: replaced by new)", out.getvalue())

    def test_allow_file_needs_a_key_and_a_reason(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "allow.tsv"
            path.write_text("# comment\n\np/test:t/gone\treplaced\n", encoding="utf-8")
            self.assertEqual(read_allow(path), {"p/test:t/gone": "replaced"})
            path.write_text("p/test:t/gone\n", encoding="utf-8")
            with self.assertRaises(ValueError):
                read_allow(path)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
