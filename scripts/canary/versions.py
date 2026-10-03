"""Version helpers for the harness canary (nested spec §D1): stable filter, candidate selection,
verified_max / known_broken from harness-versions.json. Pure stdlib."""
import json
import re

# The product's own canonical `Version::parse` rule: drops -alpha.*, -<platform> and pre-release builds.
STABLE = re.compile(r"^(0|[1-9][0-9]{0,5})\.(0|[1-9][0-9]{0,5})\.(0|[1-9][0-9]{0,5})$")

MODES = ("latest", "since-verified")


def version_key(v):
    return tuple(int(p) for p in v.split("."))


def stable(versions):
    """Plain X.Y.Z versions only, de-duplicated, numerically sorted ascending."""
    return sorted({v for v in versions if isinstance(v, str) and STABLE.match(v)}, key=version_key)


def load_versions_json(path):
    with open(path, encoding="utf-8") as f:
        doc = json.load(f)
    if not isinstance(doc, dict) or doc.get("schema_version") != 1 or not isinstance(doc.get("rows"), list):
        raise ValueError(f"{path}: not a harness-versions.json (schema_version 1)")
    return doc


def _rows(doc, harness):
    return [r for r in doc["rows"] if r.get("harness") == harness]


def verified_max(doc, harness):
    """Greatest verified version of the harness, or None when it has no rows."""
    vs = [r["version"] for r in _rows(doc, harness) if isinstance(r.get("version"), str) and STABLE.match(r["version"])]
    return max(vs, key=version_key) if vs else None


def known_broken(doc, harness):
    """Inclusive (min, max) ranges across the harness's rows; None = open end. Order-stable, de-duplicated."""
    out = []
    for r in _rows(doc, harness):
        for rng in r.get("known_broken") or []:
            t = (rng.get("min"), rng.get("max"))
            for end in t:
                if end is not None and not STABLE.match(end):
                    raise ValueError(f"known_broken bound {end!r} is not X.Y.Z")
            if t not in out:
                out.append(t)
    return out


def candidates(npm_list, mode, doc, harness, explicit=None):
    """Ascending candidate versions from an `npm view <pkg> versions --json` list.

    latest = the greatest stable version; since-verified = every stable version above verified_max;
    mode 'list' (or any mode with `explicit`) = those exact versions, each of which must exist in
    npm_list (else ValueError)."""
    if explicit is not None:
        present = set(npm_list)
        for v in explicit:
            if v not in present or not STABLE.match(v):
                raise ValueError(f"version {v!r} is not a stable version in the registry list")
        return sorted(set(explicit), key=version_key)
    st = stable(npm_list)
    if mode == "latest":
        return st[-1:]
    if mode == "since-verified":
        vm = verified_max(doc, harness)
        if vm is None:
            return st
        return [v for v in st if version_key(v) > version_key(vm)]
    raise ValueError(f"unknown versions mode {mode!r}")


def in_range(v, rng):
    lo, hi = rng
    k = version_key(v)
    return (lo is None or k >= version_key(lo)) and (hi is None or k <= version_key(hi))


def in_any_range(v, ranges):
    return any(in_range(v, r) for r in ranges)


def excluded_by_known_broken(cands, ranges):
    """Split ascending candidates into (kept, excluded). A candidate inside a known_broken range is
    excluded, except that an open-ended range (max None) never excludes the newest candidate."""
    kept, excluded = [], []
    for i, v in enumerate(cands):
        newest = i == len(cands) - 1
        hit = [r for r in ranges if in_range(v, r)]
        if hit and not (newest and all(r[1] is None for r in hit)):
            excluded.append(v)
        else:
            kept.append(v)
    return kept, excluded
