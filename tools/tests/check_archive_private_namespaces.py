#!/usr/bin/env python3
"""Exercise actual archive predicates in Python and system SQLite, no build."""
import ctypes
import ctypes.util
import hashlib
import json
from pathlib import Path
import re
import sqlite3

root = Path(__file__).resolve().parents[2]
source = root / "core/crates/vw-store/src/archive.rs"
sql = re.findall(r'"((?:DELETE FROM settings|SELECT EXISTS\(SELECT 1 FROM settings)[^"\n]+)"',
                 source.read_text())
deletes = set(value for value in sql if value.startswith("DELETE"))
selects = set(value for value in sql if value.startswith("SELECT"))
assert len(deletes) == 1 and len(selects) == 1
delete, exists = next(iter(deletes)), next(iter(selects))
prefixes = []
for file, name in [("verification_review.rs", "VERIFICATION_REVIEW_PREFIX"),
                   ("local_views.rs", "LOCAL_VIEW_PREFIX")]:
    text = (source.parent / file).read_text()
    prefixes.append(re.search(r"pub const " + name + r': &str = "([^"]+)";', text).group(1))


class PythonDb:
    def __init__(self):
        self.connection = sqlite3.connect(":memory:")

    def query(self, statement):
        return self.connection.execute(statement).fetchall()

    def close(self):
        self.connection.close()


library = ctypes.CDLL(ctypes.util.find_library("sqlite3"))
library.sqlite3_libversion.restype = ctypes.c_char_p
library.sqlite3_open.argtypes = [ctypes.c_char_p, ctypes.POINTER(ctypes.c_void_p)]
library.sqlite3_close.argtypes = [ctypes.c_void_p]
callback_type = ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_void_p, ctypes.c_int,
                                ctypes.POINTER(ctypes.c_char_p), ctypes.POINTER(ctypes.c_char_p))
library.sqlite3_exec.argtypes = [ctypes.c_void_p, ctypes.c_char_p, callback_type,
                               ctypes.c_void_p, ctypes.POINTER(ctypes.c_void_p)]
library.sqlite3_free.argtypes = [ctypes.c_void_p]


class SystemDb:
    def __init__(self):
        self.connection = ctypes.c_void_p()
        assert library.sqlite3_open(b":memory:", ctypes.byref(self.connection)) == 0

    def query(self, statement):
        rows = []

        @callback_type
        def callback(_context, count, values, _names):
            rows.append(tuple(value.decode() if value is not None else None
                              for value in values[:count]))
            return 0

        error = ctypes.c_void_p()
        status = library.sqlite3_exec(self.connection, statement.encode(), callback,
                                     None, ctypes.byref(error))
        if status:
            message = ctypes.cast(error, ctypes.c_char_p).value.decode() if error else "SQL failure"
            library.sqlite3_free(error)
            raise RuntimeError(message)
        return rows

    def close(self):
        assert library.sqlite3_close(self.connection) == 0


def literal(value):
    return "'" + value.replace("'", "''") + "'"


def bound(statement, prefix):
    return (statement.replace("?1", str(len(prefix))).replace("?2", literal(prefix))
            .replace("?3", str(len(prefix.encode()))).replace("?4", "X'" + prefix.encode().hex() + "'"))


def encoding_bytes(text, encoding):
    return text.encode({"UTF-8": "utf-8", "UTF-16le": "utf-16-le", "UTF-16be": "utf-16-be"}[encoding])


def keys(prefix, encoding):
    raw = prefix.encode()
    values = [(True, encoding_bytes(prefix + "synthetic-text", encoding)),
              (True, encoding_bytes(prefix + "\0synthetic-nul", encoding)),
              (False, raw), (False, raw + b"\xff\0\x80synthetic-blob")]
    if encoding != "UTF-8":
        values.append((False, encoding_bytes(prefix, encoding) + b"\xff\0synthetic-native-blob"))
    return [("CAST(X'" + value.hex() + "' AS TEXT)" if text else "X'" + value.hex() + "'")
            for text, value in values]


def probe(factory, encoding):
    db = factory()
    try:
        db.query("PRAGMA encoding=" + literal(encoding))
        db.query("CREATE TABLE settings(key TEXT PRIMARY KEY,value BLOB)")
        assert db.query("PRAGMA encoding")[0][0] == encoding
        flags = sorted(row[0] for row in db.query("PRAGMA compile_options"))
        db.query("INSERT INTO settings VALUES (X'" + prefixes[0].encode().hex() + "','private')")
        prior_blob_match = int(db.query("SELECT EXISTS(SELECT 1 FROM settings WHERE key GLOB "
                                      + literal(prefixes[0] + "*") + ")")[0][0])
        db.query("DELETE FROM settings")
        import_cases = 0
        for prefix in prefixes:
            for key in keys(prefix, encoding):
                db.query("INSERT INTO settings VALUES (" + key + ",'private')")
                assert int(db.query(bound(exists, prefix))[0][0]) == 1
                db.query(bound(delete, prefix))
                assert int(db.query("SELECT COUNT(*) FROM settings")[0][0]) == 0
                import_cases += 1
        keep = [encoding_bytes("synthetic.unrelated", encoding), prefixes[0][:-1].encode() + b"-neighbor\xff"]
        db.query("INSERT INTO settings VALUES ('synthetic.unrelated','keep')")
        db.query("INSERT INTO settings VALUES (X'" + keep[1].hex() + "','keep')")
        for prefix in prefixes:
            for key in keys(prefix, encoding):
                db.query("INSERT INTO settings VALUES (" + key + ",'private')")
        for prefix in prefixes:
            db.query(bound(delete, prefix))
            assert int(db.query(bound(exists, prefix))[0][0]) == 0
        remaining = sorted(row[0] for row in db.query("SELECT hex(CAST(key AS BLOB)) FROM settings"))
        assert remaining == sorted(value.hex().upper() for value in keep)
        return {"individual_namespace_cases": import_cases, "bulk_private_rows_scrubbed": sum(len(keys(p, encoding)) for p in prefixes),
                "unrelated_rows_preserved": 2, "legacy_glob_matches_blob_prefix": bool(prior_blob_match),
                "like_doesnt_match_blobs": "LIKE_DOESNT_MATCH_BLOBS" in flags}
    finally:
        db.close()


print(json.dumps({
    "status": "actual source SQL text/raw-BLOB predicates passed in both runtimes and all three database encodings",
    "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
    "python_sqlite": {"version": sqlite3.sqlite_version,
        "encodings": {encoding: probe(PythonDb, encoding) for encoding in ["UTF-8", "UTF-16le", "UTF-16be"]}},
    "system_sqlite": {"version": library.sqlite3_libversion().decode(),
        "encodings": {encoding: probe(SystemDb, encoding) for encoding in ["UTF-8", "UTF-16le", "UTF-16be"]}},
    "limits": ["SQL fixtures only, no Cargo/native archive workflow executed",
               "Production bundled-SQLite leak was not demonstrated",
               "Actual export/import/BLOB/VACUUM regressions authored; final native run deferred"]
}, indent=2))
