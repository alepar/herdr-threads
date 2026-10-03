"""Keep private install paths out of reproducible scenario captures."""

import json
from pathlib import Path
import re


_HOME_PATH = re.compile(r"/(?:Users|home)/[^/:\s\"']+(?:/[^:\s\"']+)*")


def public(value):
    """Preserve measurements and decisions while removing local install paths."""
    if isinstance(value, dict):
        return {
            key: (
                "<private-standin-bin>:<inherited-PATH>"
                if key == "PATH"
                else "<local-test-binary>"
                if key == "binary" and isinstance(item, str)
                else public(item)
            )
            for key, item in value.items()
        }
    if isinstance(value, list):
        return [public(item) for item in value]
    if isinstance(value, str):
        return _HOME_PATH.sub("<private-install-path>", value)
    return value


def write_json(path: Path, value):
    path.write_text(json.dumps(public(value), indent=2) + "\n")


def copy_jsonl(source: Path, destination: Path):
    with source.open() as raw, destination.open("w") as shared:
        for line in raw:
            if line.strip():
                shared.write(json.dumps(public(json.loads(line))) + "\n")
