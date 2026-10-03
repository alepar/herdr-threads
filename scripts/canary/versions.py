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
    if not isinstance(doc, dict) or doc.get("schema_version") not in (1, 2) or not isinstance(doc.get("rows"), list):
        raise ValueError(f"{path}: not a harness-versions.json (schema_version 1 or 2)")
    if doc["schema_version"] == 1:
        doc = upgrade_schema_1(doc)
    return doc


# Canary-only fields a schema-2 row carries and a schema-1 row lacks (null there).
SCHEMA_2_NULL_ROW_FIELDS = ("contract_id", "supported_since", "broken_event", "broken_field", "last_working",
                            "issue_url")


def upgrade_schema_1(doc):
    """Map a schema-1 document to the schema-2 shape (upgraded on read, never written back): every row is
    `verified`, source `manual`, canary-only fields null. The rest of the canary reads only `version`/`known_broken`."""
    rows = []
    for row in doc["rows"]:
        row = dict(row)
        row.setdefault("status", "verified")
        row.setdefault("source", "manual")
        for key in SCHEMA_2_NULL_ROW_FIELDS:
            row.setdefault(key, None)
        rows.append(row)
    out = dict(doc, schema_version=2, rows=rows)
    for key in ("generated_at", "latest_release"):
        out.setdefault(key, None)
    out.setdefault("contracts", {})
    return out


def _rows(doc, harness):
    return [r for r in doc["rows"] if r.get("harness") == harness]


def applies(row, contract_id):
    """Whether a row counts under `contract_id`: rows with a null contract (manual / recipe / schema 1) apply to
    every contract, and `contract_id=None` (main's contract unknown) accepts every row."""
    return contract_id is None or row.get("contract_id") is None or row["contract_id"] == contract_id


def verified_max(doc, harness, contract_id=None):
    """Greatest verified version of the harness under `contract_id`, or None when it has no verified rows. A row
    counts when its `status` is `verified` or absent (schema 1); a `known_broken` row never raises the baseline."""
    vs = [r["version"] for r in _rows(doc, harness)
          if applies(r, contract_id)
          and r.get("status", "verified") == "verified"
          and isinstance(r.get("version"), str) and STABLE.match(r["version"])]
    return max(vs, key=version_key) if vs else None


def known_broken(doc, harness, contract_id=None):
    """Inclusive (min, max) ranges across the harness's rows that apply under `contract_id`; None = open end.
    Order-stable, de-duplicated. A row with `status == "known_broken"` (a canary-written row) also yields
    `(version, version)`."""
    out = []
    for r in _rows(doc, harness):
        if not applies(r, contract_id):
            continue
        if r.get("status") == "known_broken" and isinstance(r.get("version"), str):
            t = (r["version"], r["version"])
            if not STABLE.match(t[0]):
                raise ValueError(f"known_broken row version {t[0]!r} is not X.Y.Z")
            if t not in out:
                out.append(t)
        for rng in r.get("known_broken") or []:
            t = (rng.get("min"), rng.get("max"))
            for end in t:
                if end is not None and not STABLE.match(end):
                    raise ValueError(f"known_broken bound {end!r} is not X.Y.Z")
            if t not in out:
                out.append(t)
    return out


def candidates(npm_list, mode, doc, harness, explicit=None, contract_id=None):
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
        vm = verified_max(doc, harness, contract_id)
        if vm is None:
            return st
        return [v for v in st if version_key(v) > version_key(vm)]
    raise ValueError(f"unknown versions mode {mode!r}")


def reprobe(npm_list, doc, harness, contract_id):
    """Ascending stable, published versions that are `known_broken` under another (non-null) contract and not
    under `contract_id`: they are re-probed so they can gain a row under main's contract. [] when main's
    contract is unknown."""
    if contract_id is None:
        return []
    rows = [r for r in _rows(doc, harness) if r.get("status") == "known_broken"]
    other = {r["version"] for r in rows if r.get("contract_id") not in (None, contract_id)}
    mine = {r["version"] for r in rows if applies(r, contract_id)}
    return [v for v in stable(npm_list) if v in other and v not in mine]


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
