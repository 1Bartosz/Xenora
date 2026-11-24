# SPDX-License-Identifier: GPL-3.0-or-later
import unittest

from qa.git_scope import (
    colliding_scopes,
    deleted_paths,
    effective_paths,
    parse_name_status,
)


class GitScope(unittest.TestCase):
    def test_renames_and_deletes(self):
        changes = parse_name_status([
            "M\tscripts/qa/a.py",
            "D\tscripts/qa/old.py",
            "R100\tscripts/qa/x.py\tscripts/qa/y.py",
        ])
        paths = effective_paths(changes)
        self.assertIn("scripts/qa/a.py", paths)
        self.assertIn("scripts/qa/y.py", paths)
        self.assertIn("scripts/qa/x.py", paths)
        self.assertNotIn("scripts/qa/old.py", paths)
        self.assertIn("scripts/qa/old.py", deleted_paths(changes))

    def test_directory_collision(self):
        hits = colliding_scopes(["scripts/qa/a.py"], ["scripts/qa/"])
        self.assertTrue(hits)
        self.assertFalse(colliding_scopes(["scripts/release/a.py"], ["scripts/qa/"]))


if __name__ == "__main__":
    unittest.main()
