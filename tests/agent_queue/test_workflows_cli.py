# SPDX-License-Identifier: GPL-3.0-or-later
import json
import os
import re
import subprocess
import sys
import tempfile
import unittest

from helpers import ROOT, issue

def dump(obj, path):
    with open(path, "w") as fh:
        json.dump(obj, fh)


WF = os.path.join(ROOT, ".github", "workflows")
QUEUE_WORKFLOWS = ["agent-claims.yml", "agent-pr-checks.yml"]
USER_TEXT = re.compile(r"\$\{\{\s*github\.event\.(comment|issue|pull_request)\.(body|title)|github\.head_ref")


def run_blocks(text):
    """Text of every `run:` block (literal block scalar or single line)."""
    out, lines, i = [], text.split("\n"), 0
    while i < len(lines):
        m = re.match(r"^(\s*)(?:- )?run:\s*(.*)$", lines[i])
        if m:
            indent, rest = len(m.group(1)), m.group(2)
            if rest in ("|", ">", "|-", ">-"):
                block = []
                i += 1
                while i < len(lines) and (not lines[i].strip() or len(lines[i]) - len(lines[i].lstrip()) > indent):
                    block.append(lines[i])
                    i += 1
                out.append("\n".join(block))
                continue
            out.append(rest)
        i += 1
    return out


class Workflows(unittest.TestCase):
    def read(self, name):
        with open(os.path.join(WF, name), encoding="utf-8") as fh:
            return fh.read()

    def test_top_level_permissions_and_no_user_text_in_run(self):
        for name in QUEUE_WORKFLOWS:
            text = self.read(name)
            self.assertRegex(text, r"(?m)^permissions:", name)
            blocks = run_blocks(text)
            self.assertTrue(blocks, name)
            for b in blocks:
                self.assertNotRegex(b, r"\$\{\{", "%s: expression inside run" % name)
                self.assertIsNone(USER_TEXT.search(b), name)

    def test_checker_detects_violation(self):
        bad = "jobs:\n  a:\n    steps:\n      - run: |\n          echo ${{ github.event.comment.body }}\n"
        self.assertTrue(any(re.search(r"\$\{\{", b) for b in run_blocks(bad)))

    def test_pr_workflow_never_checks_out_pr_code(self):
        text = self.read("agent-pr-checks.yml")
        self.assertNotRegex(text, r"github\.event\.pull_request\.head|ref:\s*\$\{\{")
        self.assertIn("pull_request_target", text)

    def test_actions_pinned_by_sha(self):
        for name in QUEUE_WORKFLOWS:
            for use in re.findall(r"uses:\s*(\S+)", self.read(name)):
                self.assertRegex(use, r"@[0-9a-f]{40}$", name)


class Cli(unittest.TestCase):
    def run_cli(self, *args, stdin=None):
        env = dict(os.environ, PYTHONPATH=os.path.join(ROOT, "scripts"))
        return subprocess.run([sys.executable, "-m", "agent_queue", "--config", os.path.join(ROOT, ".github", "agent-queue.json")] + list(args),
                              input=stdin, capture_output=True, text=True, env=env)

    def test_shipped_config_parses(self):
        r = self.run_cli("config-get", "upstream")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout.strip(), "ranveersoni98/Xenora")

    def test_acceptance_authorization_end_to_end(self):
        from agent_queue import lease as L, tasks as T
        i = issue(1)
        digest = T.acceptance_hash("cargo test -p gpu")
        with tempfile.TemporaryDirectory() as d:
            ip, cp = os.path.join(d, "i.json"), os.path.join(d, "c.json")
            dump(i, ip)
            dump([], cp)
            r = self.run_cli("acceptance", "--issue", ip, "--comments", cp)
            self.assertEqual(r.returncode, 3)
            self.assertEqual(r.stdout, "")
            dump([{"user": "github-actions[bot]", "body": L.acceptance_marker(digest), "created_at": "2026-01-10T10:00:00Z"}], cp)
            r = self.run_cli("acceptance", "--issue", ip, "--comments", cp)
            self.assertEqual((r.returncode, r.stdout.strip()), (0, "cargo test -p gpu"))
            i["body"] = i["body"].replace("cargo test -p gpu", "cargo test -p gpu && evil")
            dump(i, ip)
            self.assertEqual(self.run_cli("acceptance", "--issue", ip, "--comments", cp).returncode, 3)

    def test_scripts_parse(self):
        for s in ("agent-queue.sh", "agent-sync.sh"):
            r = subprocess.run(["sh", "-n", os.path.join(ROOT, "scripts", s)], capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stderr)

    def test_event_normalisation(self):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "e.json")
            dump({"issue": {"number": 4, "pull_request": {}}, "comment": {"body": "/verified abc1234", "user": {"login": "q"}}}, p)
            out = json.loads(self.run_cli("event", "--path", p).stdout)
            self.assertEqual(out, {"kind": "pr", "number": 4, "actor": "q", "body": "/verified abc1234"})


if __name__ == "__main__":
    unittest.main()
