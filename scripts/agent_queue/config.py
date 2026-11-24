# SPDX-License-Identifier: GPL-3.0-or-later
import json

AREAS = ("cpu", "gpu", "runtime", "shared", "qa")
PRIORITIES = ("critical", "high", "medium", "low")
DEFAULT = {"lease_hours": 24, "maintainers": [], "contributors": {}, "upstream": ""}


def parse(text):
    data = json.loads(text)
    cfg = dict(DEFAULT)
    cfg.update({k: data[k] for k in DEFAULT if k in data})
    if not isinstance(cfg["lease_hours"], (int, float)) or cfg["lease_hours"] <= 0:
        raise ValueError("lease_hours must be a positive number")
    return cfg


def load(path):
    with open(path, encoding="utf-8") as fh:
        return parse(fh.read())


def is_maintainer(cfg, login):
    return login in cfg["maintainers"]


def may_work_on(cfg, login, area):
    if is_maintainer(cfg, login):
        return True
    return area in cfg["contributors"].get(login, [])


def is_verifier(cfg, login):
    return is_maintainer(cfg, login) or "qa" in cfg["contributors"].get(login, [])
