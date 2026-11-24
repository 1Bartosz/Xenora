# SPDX-License-Identifier: GPL-3.0-or-later
"""File ownership patterns: exact paths, directory prefixes ("dir/") and globs ("*", "**")."""
import re


def _norm(p):
    p = p.strip().replace("\\", "/")
    while p.startswith("./"):
        p = p[2:]
    return p.lstrip("/")


def _is_glob(p):
    return any(c in p for c in "*?[")


def _regex(pattern):
    out, i = [], 0
    while i < len(pattern):
        if pattern.startswith("**/", i):
            out.append("(?:.*/)?")
            i += 3
        elif pattern.startswith("**", i):
            out.append(".*")
            i += 2
        elif pattern[i] == "*":
            out.append("[^/]*")
            i += 1
        elif pattern[i] == "?":
            out.append("[^/]")
            i += 1
        else:
            out.append(re.escape(pattern[i]))
            i += 1
    return re.compile("^" + "".join(out) + "$")


def path_matches(path, pattern):
    path, pattern = _norm(path), _norm(pattern)
    if not pattern:
        return False
    if pattern.endswith("/"):
        return path.startswith(pattern)
    if _is_glob(pattern):
        return bool(_regex(pattern).match(path))
    return path == pattern


def _prefix(pattern):
    """Literal leading part of a pattern (the whole directory for a "dir/" pattern)."""
    for i, c in enumerate(pattern):
        if c in "*?[":
            return pattern[:i]
    return pattern


def patterns_overlap(a, b):
    """True when some path could match both patterns. Errs towards True."""
    a, b = _norm(a), _norm(b)
    if not a or not b:
        return False
    a_exact = not _is_glob(a) and not a.endswith("/")
    b_exact = not _is_glob(b) and not b.endswith("/")
    if a_exact:
        return path_matches(a, b)
    if b_exact:
        return path_matches(b, a)
    pa, pb = _prefix(a), _prefix(b)
    return pa.startswith(pb) or pb.startswith(pa)


def sets_overlap(files_a, files_b):
    """First overlapping pair (a, b) between two pattern lists, or None."""
    for a in files_a:
        for b in files_b:
            if patterns_overlap(a, b):
                return (a, b)
    return None
