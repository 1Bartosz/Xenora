# SPDX-License-Identifier: GPL-3.0-or-later
import json
import tempfile
import unittest
from pathlib import Path

from qa.rdna2_plan import (
    Inventory,
    build_manifests,
    classify_unsupported,
    compact_candidates,
    compact_inventory,
    coverage_diff,
    discrepancies,
    dumps_candidates,
    load_inventories,
    parse_claims_file,
    plan_batches,
    reconcile_with_coverage_report,
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

    def test_family_status_limit_and_priority(self):
        reuse = classify_unsupported(
            self.data, families=["SOP2"], statuses=["reuse-primitives"], limit=5
        )
        self.assertLessEqual(len(reuse), 5)
        for cand in reuse:
            self.assertEqual(cand.family, "SOP2")
            self.assertEqual(cand.status, "reuse-primitives")
            self.assertEqual(cand.priority_rank, 0)
        needs_rules = classify_unsupported(
            self.data, statuses=["needs-operand-rules"], limit=3
        )
        for cand in needs_rules:
            self.assertEqual(cand.status, "needs-operand-rules")
            self.assertEqual(cand.priority_rank, 2)
        # Priority ordering: reuse before needs-operand-rules when unfiltered.
        mixed = classify_unsupported(self.data, limit=20)
        ranks = [c.priority_rank for c in mixed]
        self.assertEqual(ranks, sorted(ranks))

    def test_variant_only_rules_do_not_make_a_family_reuse_ready(self):
        # A rule that only encoding variants use (vop3_vgpr2 re-encodes declared
        # VOP2 forms) cannot host a new VOP3 form such as V_FMA_F32.
        variant_rules = {
            variant["operands"] for variant in self.data["registry"].get("encoding_variant", [])
        }
        if not variant_rules:
            self.skipTest("no encoding variants declared")
        vop3 = classify_unsupported(self.data, families=["VOP3"])
        self.assertTrue(vop3)
        for cand in vop3:
            self.assertNotIn(cand.operand_layout, variant_rules, cand.name)
            self.assertNotEqual(cand.status, "reuse-primitives", cand.name)

    def test_manifests_exclude_claims_and_stay_deterministic(self):
        cands = classify_unsupported(self.data, statuses=["reuse-primitives"], limit=50)
        with tempfile.TemporaryDirectory() as tmp:
            claims = Path(tmp) / "claims.json"
            claims.write_text(
                json.dumps({"claims": [{"families": ["SOP2"], "paths": ["crates/isa/**"]}]}),
                encoding="utf-8",
            )
            fams, paths = parse_claims_file(claims)
            self.assertIn("SOP2", fams)
            # Family+path claims must not path-block every other family.
            self.assertEqual(paths, set())
            manifests = build_manifests(
                cands, exclude_families=sorted(fams), claimed_paths=sorted(paths), limit=5
            )
            self.assertGreater(len(manifests), 0)
            for item in manifests:
                self.assertNotEqual(item["instruction"]["family"], "SOP2")
                self.assertIn("permitted_files", item)
                self.assertIn("acceptance_tests", item)
                self.assertIn("specification_references", item)
                self.assertIn("dependencies", item)
            again = build_manifests(
                cands, exclude_families=sorted(fams), claimed_paths=sorted(paths), limit=5
            )
            self.assertEqual(manifests, again)

            path_only = Path(tmp) / "path_only.json"
            path_only.write_text(
                json.dumps({"claims": [{"paths": ["crates/isa/**"]}]}),
                encoding="utf-8",
            )
            _, path_claims = parse_claims_file(path_only)
            self.assertIn("crates/isa/**", path_claims)
            blocked = build_manifests(
                cands, claimed_paths=sorted(path_claims), limit=5
            )
            self.assertEqual(blocked, [])

    def test_reconcile_does_not_inflate_coverage(self):
        report = reconcile_with_coverage_report(ROOT, self.data)
        self.assertIn("mismatches", report)
        self.assertEqual(
            report["planner_totals"]["identified"],
            report["canonical_totals"]["identified"],
        )
        # Declared/decoder may differ in representation but planner must not
        # silently bump counts to hide mismatches.
        before = self.inv.declared
        _ = reconcile_with_coverage_report(ROOT, self.data)
        after = summarize_inventory(self.data).declared
        self.assertEqual(before, after)
        self.assertTrue(report["ok"] or isinstance(report["mismatches"], list))

    def test_compact_helpers_and_diff(self):
        text = compact_inventory(self.inv)
        self.assertIn("identified=", text)
        cands = classify_unsupported(self.data, limit=2)
        self.assertIn("candidates=2", compact_candidates(cands))
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
        self.assertIsInstance(discrepancies(self.data), list)


if __name__ == "__main__":
    unittest.main()
