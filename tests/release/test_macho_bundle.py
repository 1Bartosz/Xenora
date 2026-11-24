# SPDX-License-Identifier: GPL-3.0-or-later
import tempfile
import unittest
from pathlib import Path

from release.bundle import check_app_bundle, diagnose_codesign_output
from release.macho import check_archs, check_otool_linkage, consistent_deployment_targets


class MachOBundle(unittest.TestCase):
    def test_arch_and_linkage(self):
        ok = check_archs("/tmp/x", "arm64")
        self.assertTrue(ok.ok)
        bad = check_archs("/tmp/x", "x86_64 arm64")
        self.assertFalse(bad.ok)
        link = check_otool_linkage("/tmp/x", "/tmp/x:\n\t/usr/lib/libSystem.B.dylib")
        self.assertTrue(link.ok)
        vuln = check_otool_linkage("/tmp/x", "/tmp/x:\n\t@rpath/libvulkan.dylib")
        self.assertFalse(vuln.ok)

    def test_deployment_targets(self):
        a = "version.min osx,10.15\n"
        b = "version.min osx,10.15\n"
        c = "version.min osx,11.0\n"
        self.assertTrue(consistent_deployment_targets([a, b])[0])
        self.assertFalse(consistent_deployment_targets([a, c])[0])

    def test_bundle_and_codesign(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "Xenora.app"
            (root / "Contents" / "MacOS").mkdir(parents=True)
            (root / "Contents" / "Info.plist").write_text("{}", encoding="utf-8")
            check = check_app_bundle(root)
            self.assertTrue(check.ok)
            missing = check_app_bundle(Path(tmp) / "Missing.app")
            self.assertFalse(missing.ok)
        unsigned = diagnose_codesign_output("/tmp/x", "code object is not signed at all")
        self.assertFalse(unsigned.ok)
        self.assertIn("unsigned", unsigned.problems)


if __name__ == "__main__":
    unittest.main()
