# SPDX-License-Identifier: GPL-3.0-or-later
import unittest
from pathlib import Path

from qa.runtime_inventory import build_runtime_inventory, compact_runtime, parse_services_text

ROOT = Path(__file__).resolve().parents[2]

SAMPLE = """
# comment
service kernel32.dll GetCurrentProcess() -> handle
    handler get_current_process
    status implemented
    alias api-ms-win-core-processthreads-l1-1-0.dll
    fails never
    ref processthreadsapi.h GetCurrentProcess
    test identity_services_return_documented_pseudo_handles_and_ids

service ucrtbase.dll sqrt(x: f64) -> f64
    handler crt_sqrt
    status partial
    fails NaN for a negative argument
    ref math.h sqrt
    test crt_math_guest_dispatch
"""


class RuntimeInventory(unittest.TestCase):
    def test_parse_separates_aliases(self):
        services = parse_services_text(SAMPLE)
        self.assertEqual(len(services), 2)
        self.assertEqual(services[0].status, "implemented")
        self.assertEqual(services[0].aliases, ["api-ms-win-core-processthreads-l1-1-0.dll"])
        self.assertEqual(services[1].library, "ucrtbase.dll")

    def test_live_registry(self):
        inv = build_runtime_inventory(ROOT)
        self.assertGreaterEqual(inv.service_count, 20)
        self.assertEqual(
            inv.service_count,
            inv.by_status.get("implemented", 0)
            + inv.by_status.get("partial", 0)
            + inv.by_status.get("stub", 0),
        )
        self.assertEqual(inv.independently_verified, [])
        self.assertGreater(inv.alias_count, 0)
        # Aliases are separate records pointing at canonical services.
        self.assertTrue(all("canonical" in a and "alias" in a for a in inv.aliases))
        self.assertTrue(all("!" in a["canonical"] for a in inv.aliases))
        text = compact_runtime(inv)
        self.assertIn(f"services={inv.service_count}", text)
        self.assertIn("verified=0", text)


if __name__ == "__main__":
    unittest.main()
