"""Refuse unavailable native readonly qualification before imports or effects.

The audited installation supplies no complete readonly source/dependency/config
producer or dispatcher-captured timeout snapshot. Machine argv is installation
metadata only. No raw-config/default-timeout substitute can qualify this API.
"""
import argparse
import json
import sys
import unicodedata


def unavailable(reason):
    return {"schema_version": 1, "status": "unsupported", "reason": reason,
            "runtime_descriptor": None, "profile": None, "home": None,
            "physical_home": None, "interpreter": None, "source_root": None,
            "enabled": None, "disabled": None, "callback_timeout_ms": None,
            "api": None, "evidence_stage": "unavailable"}


def main():
    # -I ignores PYTHONDONTWRITEBYTECODE. No native module is imported here.
    sys.dont_write_bytecode = True
    parser = argparse.ArgumentParser()
    parser.add_argument("--profile", required=True)
    options = parser.parse_args()
    profile = options.profile
    valid = 0 < len(profile.encode()) <= 256 and not any(unicodedata.category(c) == "Cc" for c in profile)
    reason = "native_read_only_boundary_unavailable" if valid else "profile_unavailable"
    print(json.dumps(unavailable(reason), separators=(",", ":")))


if __name__ == "__main__":
    main()
