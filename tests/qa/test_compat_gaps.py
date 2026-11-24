# SPDX-License-Identifier: GPL-3.0-or-later
import unittest

from qa.compat_gaps import analyze_gaps, classify_subsystem, compact_gaps


class CompatGaps(unittest.TestCase):
    def test_groups_missing_by_subsystem(self):
        report = analyze_gaps(
            {
                "workload": "synthetic-a",
                "imports": [
                    {"module": "kernel32.dll", "symbol": "VirtualAlloc"},
                    {"module": "kernel32.dll", "symbol": "VirtualFree"},
                    {"module": "kernel32.dll", "symbol": "GetCurrentProcess"},
                    {"module": "ucrtbase.dll", "symbol": "sqrt"},
                    {"module": "kernel32.dll", "symbol": "CreateFileW"},
                ],
            },
            implemented=["kernel32.dll!GetCurrentProcess"],
            partial=["kernel32.dll!VirtualAlloc", "ucrtbase.dll!sqrt"],
        )
        self.assertEqual(report.workload, "synthetic-a")
        self.assertEqual(len(report.present), 1)
        self.assertEqual(len(report.partial), 2)
        self.assertEqual(len(report.missing), 2)  # VirtualFree, CreateFileW
        self.assertIn("memory", report.by_subsystem)
        self.assertIn("memory", report.shared_primitives)
        self.assertIn("binaries are not executed", " ".join(report.notes).lower())
        self.assertIn("missing=2", compact_gaps(report))

    def test_subsystem_rules(self):
        self.assertEqual(classify_subsystem("VirtualAlloc"), "memory")
        self.assertEqual(classify_subsystem("sqrtf"), "crt-math")
        self.assertEqual(classify_subsystem("NoSuchThing"), "other")


if __name__ == "__main__":
    unittest.main()
