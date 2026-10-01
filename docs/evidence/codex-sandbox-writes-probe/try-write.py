#!/usr/bin/env python3
"""One write attempt, reported in one line. Usage: try-write.py open-rw|append|sqlite PATH"""
import sqlite3, sys
mode, path = sys.argv[1], sys.argv[2]
try:
    if mode == "open-rw":
        open(path, "r+b").close()
    elif mode == "append":
        open(path, "ab").close()
    elif mode == "sqlite":
        c = sqlite3.connect(path, timeout=2)
        c.execute("create table probe_x(a)")
        c.commit()
    print(f"WRITABLE ({mode})")
except Exception as e:  # noqa: BLE001
    print(f"DENIED ({mode}): {type(e).__name__}: {e}")
    sys.exit(1)
