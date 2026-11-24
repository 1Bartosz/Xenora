# SPDX-License-Identifier: GPL-3.0-or-later
import unittest

from qa.pr_validate import (
    check_artifacts,
    check_changelog_required,
    check_dco,
    check_native_arch_strings,
    check_report_commit_identity,
    check_scope,
    merge_reports,
)


class PRValidate(unittest.TestCase):
    def test_scope_and_artifacts(self):
        scope = check_scope(["scripts/qa/a.py", "crates/loader/x.rs"], ["scripts/qa/", "tests/qa/"])
        self.assertFalse(scope.ok)
        arts = check_artifacts(["scripts/qa/a.py", "foo/__pycache__/x.pyc", "bar.o"])
        self.assertTrue(any(f.code == "artifact" for f in arts.findings))

    def test_dco_and_changelog(self):
        bad = check_dco(["feat: no trailer\n"])
        self.assertFalse(bad.ok)
        good = check_dco(["feat: x\n\nSigned-off-by: A U <a@b.c>\n"])
        self.assertTrue(good.ok)
        log = check_changelog_required(["scripts/qa/x.py"])
        self.assertTrue(any(f.code == "changelog" for f in log.findings))

    def test_evidence_sha_and_arch(self):
        sha = "a" * 40
        mismatch = check_report_commit_identity(sha, "b" * 40)
        self.assertFalse(mismatch.ok)
        arch = check_native_arch_strings([
            "foo: Mach-O 64-bit executable x86_64",
            "bar: Mach-O 64-bit executable arm64",
        ])
        self.assertFalse(arch.ok)
        merged = merge_reports(mismatch, arch)
        self.assertGreaterEqual(len(merged.findings), 2)


if __name__ == "__main__":
    unittest.main()
