#!/usr/bin/env python3
"""Exercise the actual history admission SQL; no Rust build or invented store.

The SQLite settings declaration and SQL constants come directly from production
source. VM progress counts and query plans must stay independent of unrelated
rows; malformed TEXT/BLOB namespace keys still reach bounded admission metadata.
"""
from pathlib import Path
import argparse
import re
import sqlite3

ROOT = next(p for p in Path(__file__).resolve().parents if (p / 'AGENTS.md').is_file())
SOURCE = 'core/crates/vw-store/src/verification_review.rs'
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--legacy-scan-negative-control', action='store_true')
args = parser.parse_args()
source = (ROOT / SOURCE).read_text()

def constant(name):
    return re.search(r'\b' + name + r': &str = "([^"]+)";', source).group(1)

prefix = constant('VERIFICATION_REVIEW_PREFIX')
sql = ("SELECT typeof(key),length(CAST(key AS BLOB)),typeof(value),length(CAST(value AS BLOB)),"
       "substr(CAST(key AS BLOB),1,?3) FROM settings WHERE "
       "substr(CAST(key AS BLOB),1,?1)=CAST(?2 AS BLOB) LIMIT ?4"
       if args.legacy_scan_negative_control else constant('HISTORY_ADMISSION_SQL'))
end = prefix[:-1] + chr(ord(prefix[-1]) + 1)
if not args.legacy_scan_negative_control:
    assert constant('VERIFICATION_REVIEW_PREFIX_END') == end
params = ((len(prefix), prefix, len(prefix) + 64, 65) if args.legacy_scan_negative_control
          else (prefix, end, len(prefix) + 64, 65))
schema = re.search(r'CREATE TABLE settings \([^;]+;', (ROOT / 'contracts/storage.sql').read_text()).group(0)

def measured(connection):
    steps = 0
    def step():
        nonlocal steps
        steps += 1
        return 0
    connection.set_progress_handler(step, 1)
    try:
        rows = connection.execute(sql, params).fetchall()
    finally:
        connection.set_progress_handler(None, 0)
    return rows, steps

def database(unrelated):
    connection = sqlite3.connect(':memory:')
    connection.execute(schema)
    for key in ('before.', 'zz.after.'):
        connection.executemany('INSERT INTO settings VALUES (?,?)',
            ((key + str(n), 'ordinary') for n in range(unrelated)))
        connection.executemany('INSERT INTO settings VALUES (?,?)',
            ((sqlite3.Binary((key + str(n)).encode()), 'ordinary') for n in range(unrelated)))
    return connection

for unrelated in (0, 10_000):
    with database(unrelated) as connection:
        rows, steps = measured(connection)
        print(f'empty: unrelated={4 * unrelated}, rows={len(rows)}, VMsteps={steps}', flush=True)
        if args.legacy_scan_negative_control:
            assert not rows
            if unrelated:
                assert steps > 50_000, 'Legacy scan negative control did not reproduce'
                print('CONFIRMED legacy SQL scans unrelated settings')
            continue
        assert not rows and steps <= 100, 'Empty history scans unrelated settings'
        plan = [row[3] for row in connection.execute('EXPLAIN QUERY PLAN ' + sql, params)]
        assert sum('SEARCH settings USING INDEX' in detail for detail in plan) == 2, plan
        assert not any('SCAN settings' in detail for detail in plan), plan
        keys = [prefix + 'a' * 64, prefix + 'A' * 64, prefix, prefix + 'x' * (2 * 1024 * 1024),
                sqlite3.Binary((prefix + 'b' * 64).encode()), sqlite3.Binary(prefix.encode()),
                prefix + '\x00' + 'c' * 64]
        connection.executemany('INSERT INTO settings VALUES (?,?)', ((key, '{}') for key in keys))
        rows, steps = measured(connection)
        assert len(rows) == len(keys) and steps <= 400
        assert sum(row[0] == 'blob' for row in rows) == 2
        assert all(len(row[4]) <= len(prefix) + 64 for row in rows)
        assert any(row[1] > 2 * 1024 * 1024 and len(row[4]) == len(prefix) + 64 for row in rows)
        print(f'malformed namespace: unrelated={4 * unrelated}, rows={len(rows)}, VMsteps={steps}', flush=True)
        connection.execute('DELETE FROM settings WHERE key>=? AND key<?', (prefix, end))
        connection.execute('DELETE FROM settings WHERE key>=CAST(? AS BLOB) AND key<CAST(? AS BLOB)', (prefix, end))
        connection.executemany('INSERT INTO settings VALUES (?,?)',
            ((prefix + f'{n:064x}', '{}') for n in range(100)))
        rows, steps = measured(connection)
        assert len(rows) == 65 and steps <= 1600
        print(f'namespace saturation: unrelated={4 * unrelated}, rows={len(rows)}, VMsteps={steps}', flush=True)
print('PASS actual indexed TEXT/BLOB history SQL: sparse settings, malformed keys and65-row sentinel')
