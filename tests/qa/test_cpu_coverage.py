# SPDX-License-Identifier: GPL-3.0-or-later
import json
import tempfile
import unittest
from pathlib import Path

from qa.cpu_coverage import (
    blocker_manifest,
    build_report,
    compare_reports,
    dumps_report,
    scan_imports,
    scan_opcodes,
)

ROOT = Path(__file__).resolve().parents[2]


class CpuCoverage(unittest.TestCase):
    def test_scan_opcodes_positive_and_negative(self):
        text = """
        match opcode {
            0x89 => Op::Mov,
            0x90 => unimplemented!("nop"),
            other => unsupported(0xFF),
        }
        let litter = 0xAB;
        """
        entries = {e.opcode: e.status for e in scan_opcodes(text, source="t.rs")}
        self.assertEqual(entries.get("89"), "supported")
        self.assertEqual(entries.get("90"), "unsupported")

    def test_scan_imports_kinds(self):
        text = '''
        fn handler() { "CreateFileW" }
        let alias = forward("ReadFile");
        todo!("stub for {}", "Sleep");
        '''
        kinds = {e.name: e.kind for e in scan_imports(text, source="imports.rs")}
        self.assertEqual(kinds.get("CreateFileW"), "functional")
        self.assertEqual(kinds.get("ReadFile"), "alias")
        self.assertEqual(kinds.get("Sleep"), "stub")

    def test_repo_report_and_blockers(self):
        report = build_report(ROOT)
        self.assertTrue(report.opcodes or report.imports or report.notes)
        payload = json.loads(dumps_report(report))
        blockers = blocker_manifest(report, limit=10)
        self.assertIsInstance(blockers, list)
        delta = compare_reports(payload, payload)
        self.assertEqual(delta["newly_supported"], [])


if __name__ == "__main__":
    unittest.main()
