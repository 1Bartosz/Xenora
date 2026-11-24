# SPDX-License-Identifier: GPL-3.0-or-later
import json
import unittest
from pathlib import Path

from qa.rdna2_plan import (
    Inventory,
    classify_unsupported,
    coverage_diff,
    discrepancies,
    dumps_candidates,
    load_inventories,
    plan_batches,
    summarize_inventory,
)

ROOT = Path(__file__).resolve().parents[2]


class Rdna2Plan(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = load_inventories(ROOT)
        cls.inv = summarize_inventory(cls.data)

    def test_dynamic_baseline_not_hardcoded(self):
        self.assertGreater(self.inv.identified, 1000)
        self.assertGreater(self.inv.declared, 0)
        self.assertLess(self.inv.declared, self.inv.identified)
        self.assertEqual(self.inv.unsupported, self.inv.identified - self.inv.declared)

    def test_candidates_are_not_counted_as_implemented(self):
        cands = classify_unsupported(self.data, limit=25)
        self.assertTrue(cands)
        for cand in cands:
            self.assertIn("candidate is not implemented", cand.notes)
            self.assertNotIn((cand.family, cand.name), self.data["declared"])
        payload = json.loads(dumps_candidates(cands))
        self.assertEqual(payload["count"], len(cands))
        batches = plan_batches(cands)
        self.assertTrue(batches)

    def test_discrepancies_and_diff(self):
        issues = discrepancies(self.data)
        self.assertIsInstance(issues, list)
        before = Inventory(
            identified=self.inv.identified,
            declared=self.inv.declared - 1 if self.inv.declared else 0,
            decoder_supported=self.inv.decoder_supported,
            reference_verified=self.inv.reference_verified,
            metal_verified=self.inv.metal_verified,
            unsupported=self.inv.unsupported + 1,
            by_family={},
        )
        delta = coverage_diff(before, self.inv)
        self.assertEqual(delta["declared_delta"], self.inv.declared - before.declared)


if __name__ == "__main__":
    unittest.main()
