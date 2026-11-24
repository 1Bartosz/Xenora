# SPDX-License-Identifier: GPL-3.0-or-later
import json
import tempfile
import unittest
from pathlib import Path

from qa.coverage_history import historical_report, read_snapshot, snapshot_inventory, write_snapshot
from qa.rdna2_plan import Inventory
from qa.stale_branch import classify_sync, format_sync_reports

ROOT = Path(__file__).resolve().parents[2]


class CoverageHistoryStaleBranch(unittest.TestCase):
    def test_snapshot_roundtrip_and_history(self):
        inv = snapshot_inventory(ROOT)
        with tempfile.TemporaryDirectory() as tmp:
            before = Path(tmp) / "before.json"
            after = Path(tmp) / "after.json"
            older = Inventory(
                identified=inv.identified,
                declared=max(0, inv.declared - 3),
                decoder_supported=inv.decoder_supported,
                reference_verified=inv.reference_verified,
                metal_verified=inv.metal_verified,
                unsupported=inv.unsupported + 3,
                by_family={},
            )
            write_snapshot(before, older, revision="aaa")
            write_snapshot(after, inv, revision="bbb")
            rev, loaded = read_snapshot(after)
            self.assertEqual(rev, "bbb")
            self.assertEqual(loaded.declared, inv.declared)
            report = historical_report(before, after)
            self.assertEqual(report["declared_delta"], inv.declared - older.declared)
            self.assertEqual(report["before_revision"], "aaa")

    def test_stale_and_diverged_branches(self):
        ok = classify_sync(branch="main", upstream="origin/main", ahead=0, behind=0)
        self.assertFalse(ok.stale)
        stale = classify_sync(branch="topic", upstream="origin/main", ahead=0, behind=5)
        self.assertTrue(stale.stale)
        diverged = classify_sync(branch="topic", upstream="origin/main", ahead=2, behind=3)
        self.assertTrue(diverged.diverged)
        text = format_sync_reports([ok, stale, diverged])
        self.assertIn("STALE", text)
        self.assertIn("DIVERGED", text)


if __name__ == "__main__":
    unittest.main()
