# SPDX-License-Identifier: GPL-3.0-or-later
import unittest
from pathlib import Path

from qa.cpu_inventory import (
    build_inventory,
    compact_inventory,
    expand_forms,
    mnemonic_covers_op,
    parse_fixture_mnemonics,
    parse_native_ops,
)

ROOT = Path(__file__).resolve().parents[2]


class CpuInventory(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.inv = build_inventory(ROOT)

    def test_registry_forms_not_heuristic(self):
        self.assertEqual(self.inv.schema, 1)
        self.assertEqual(self.inv.architecture, "x86-64")
        self.assertGreater(self.inv.form_count, 200)
        self.assertGreater(self.inv.operation_count, 100)
        self.assertIn("1", self.inv.by_map)
        self.assertIn("0f", self.inv.by_map)
        self.assertEqual(
            self.inv.form_count,
            sum(self.inv.by_map.values()),
        )

    def test_native_and_shared_partition(self):
        native = set(self.inv.native_ops)
        shared = set(self.inv.shared_semantic_ops)
        ops = set(self.inv.operations)
        self.assertTrue(native)
        self.assertTrue(shared)
        self.assertTrue(native.issubset(ops))
        self.assertTrue(shared.issubset(ops))
        self.assertEqual(native | shared, ops)
        self.assertEqual(native & shared, set())

    def test_unsupported_catalog_separate(self):
        self.assertIn("MMX", self.inv.unsupported_catalog)
        for name in self.inv.unsupported_catalog:
            self.assertNotIn(name.lower(), self.inv.operations)

    def test_expand_count_and_fixtures(self):
        expanded = expand_forms(
            [{"op": "push", "opcode": 0x50, "count": 8, "enc": "op_reg", "size": "q"}]
        )
        self.assertEqual(len(expanded), 8)
        self.assertEqual(expanded[0].opcode, 0x50)
        self.assertEqual(expanded[7].opcode, 0x57)
        mnems = parse_fixture_mnemonics(["00 cb | add bl, cl\n66 01 cb | add bx, cx\n"])
        self.assertEqual(mnems, {"add"})
        self.assertTrue(mnemonic_covers_op("je", "jcc"))
        self.assertTrue(mnemonic_covers_op("imul", "imul2"))

    def test_compact_and_no_coverage_inflation(self):
        text = compact_inventory(self.inv)
        self.assertIn(f"forms={self.inv.form_count}", text)
        # Notes must deny inflation.
        joined = " ".join(self.inv.notes).lower()
        self.assertIn("does not inflate", joined)


if __name__ == "__main__":
    unittest.main()
