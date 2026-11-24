# SPDX-License-Identifier: GPL-3.0-or-later
import unittest

import helpers  # noqa: F401
from agent_queue.patterns import patterns_overlap as ov, path_matches as pm, sets_overlap


class Patterns(unittest.TestCase):
    def test_matches(self):
        self.assertTrue(pm("a/b.rs", "a/b.rs"))
        self.assertTrue(pm("a/b/c.rs", "a/"))
        self.assertFalse(pm("ab/c.rs", "a/"))
        self.assertTrue(pm("a/b.rs", "a/*.rs"))
        self.assertFalse(pm("a/x/b.rs", "a/*.rs"))
        self.assertTrue(pm("a/x/y/b.rs", "a/**/b.rs"))
        self.assertTrue(pm("a/b.rs", "a/**/b.rs"))
        self.assertTrue(pm("a/x/y.rs", "a/**"))

    def test_overlap_exact_dir_glob(self):
        self.assertTrue(ov("a/b.rs", "a/b.rs"))
        self.assertFalse(ov("a/b.rs", "a/c.rs"))
        self.assertTrue(ov("a/", "a/b/c.rs"))
        self.assertTrue(ov("a/b/", "a/"))
        self.assertFalse(ov("a/", "b/"))
        self.assertTrue(ov("a/**", "a/x/y.rs"))
        self.assertTrue(ov("a/*.rs", "a/"))
        self.assertTrue(ov("a/**/gpu/*.rs", "a/x/"))
        self.assertFalse(ov("a/x*", "a/y*"))
        self.assertFalse(ov("a/b.rs", "a/*.txt"))

    def test_sets(self):
        self.assertEqual(sets_overlap(["x/a.rs", "y/"], ["z.rs", "y/q.rs"]), ("y/", "y/q.rs"))
        self.assertIsNone(sets_overlap(["x/a.rs"], ["y/"]))
