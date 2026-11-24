# SPDX-License-Identifier: GPL-3.0-or-later
import json
import os
import subprocess
import sys
import tempfile
import unittest

import pr_template_check as c

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
TEMPLATE = open(os.path.join(ROOT, ".github", "PULL_REQUEST_TEMPLATE.md"), encoding="utf-8").read()

GOOD = """## Summary

Adds X.

Closes #12

## Implementation and evidence

Touches crates/texture only.

## Verification

| Check | Result / evidence |
| --- | --- |
| Formatting and lint | cargo fmt --check: clean |
| Unit / integration tests | cargo test -p xenora-texture: 40 passed |

## Compatibility and limitations

None known.

## Checklist

- [x] I claimed the linked issue before starting, and the claim was still mine when I opened this pull request.
- [x] I reviewed the changes and disclosed material AI assistance below, if applicable.
- [x] My commits are signed off under the DCO.

## AI assistance

None.
"""


class Check(unittest.TestCase):
    def test_good_body_passes(self):
        self.assertEqual(c.check(GOOD), [])

    def test_unfilled_template_fails_on_every_rule(self):
        problems = c.check(TEMPLATE)
        text = "\n".join(problems)
        for want in ('"## Summary" is empty', '"## Implementation and evidence" is empty',
                     '"## Verification" is empty', '"## Compatibility and limitations" is empty',
                     '"## AI assistance" is empty', "No linked issue", "claimed-first", "DCO"):
            self.assertIn(want, text)

    def test_empty_and_freeform_bodies_fail(self):
        self.assertTrue(c.check(""))
        self.assertTrue(c.check("Fixes a bug. Closes #3"))

    def test_missing_section_is_named(self):
        body = GOOD.replace("## Compatibility and limitations\n\nNone known.\n", "")
        self.assertEqual(c.check(body), ['Missing section "## Compatibility and limitations".'])

    def test_unticked_boxes_fail(self):
        body = GOOD.replace("- [x] I claimed", "- [ ] I claimed").replace("- [x] My commits", "- [ ] My commits")
        problems = c.check(body)
        self.assertEqual(len(problems), 2)
        self.assertIn("claimed-first", problems[0])
        self.assertIn("DCO", problems[1])

    def test_issue_link_forms(self):
        for link in ("Fixes #4", "resolves #4", "Closed #4", "No linked issue: requested by the maintainer"):
            self.assertEqual(c.check(GOOD.replace("Closes #12", link)), [], link)
        self.assertIn("No linked issue", "\n".join(c.check(GOOD.replace("Closes #12", "See issue 12"))))

    def test_link_inside_comment_does_not_count(self):
        self.assertTrue(c.check(GOOD.replace("Closes #12", "<!-- Closes #12 -->")))

    def test_new_claim_checkbox_wording_passes(self):
        body = GOOD.replace("I claimed the linked issue before starting, and the claim was still mine when I opened this pull request.",
                            "I claimed the linked issue with a reply before starting, and nobody else held a live claim on it.")
        self.assertEqual(c.check(body), [])

    def test_cli_exit_codes_from_event_file(self):
        script = os.path.join(ROOT, "scripts", "pr_template_check.py")
        with tempfile.TemporaryDirectory() as d:
            for body, want in ((GOOD, 0), (TEMPLATE, 1), (None, 1)):
                path = os.path.join(d, "event.json")
                with open(path, "w", encoding="utf-8") as fh:
                    json.dump({"pull_request": {"body": body}}, fh)
                run = subprocess.run([sys.executable, script, "--event", path], capture_output=True, text=True)
                self.assertEqual(run.returncode, want, run.stdout)


if __name__ == "__main__":
    unittest.main()
