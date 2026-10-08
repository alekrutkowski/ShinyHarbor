"""Exercise the SQL actually embedded in analytics.rs using local SQLite.

This verifies SQL syntax/aggregation, not Rust's rusqlite integration.
"""
import re
import sqlite3
import time
import unittest
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]
SOURCE = (ROOT / 'src/analytics.rs').read_text(encoding="utf-8")


def extract(start: str) -> str:
    match = re.search(r'"(' + re.escape(start) + r'[^"\\]*)"', SOURCE, re.S)
    if not match:
        raise ValueError(f'Embedded SQL not found: {start}')
    return match.group(1)


class Sql(unittest.TestCase):
    def setUp(self):
        self.conn = sqlite3.connect(':memory:')
        self.conn.executescript(extract('PRAGMA auto_vacuum=INCREMENTAL;'))
        self.now = int(time.time())

    def tearDown(self):
        self.conn.close()

    def record(self, app='hello', stamp=None):
        stamp = stamp or self.now
        self.conn.execute(extract('INSERT INTO traffic VALUES'),
            (stamp // 3600, app, 'NL', 'https://example.org', 1, 1, 0, 0, 0, 10, 100, 5, 0))
        self.conn.execute(extract('INSERT OR IGNORE INTO visitors VALUES'), (stamp // 86400, app, 'daily-keyed-estimate'))

    def test_real_upsert_and_visitor_deduplication(self):
        self.record(); self.record()
        self.assertEqual(self.conn.execute('SELECT requests,pages,bytes_out FROM traffic').fetchone(), (2, 2, 200))
        self.assertEqual(self.conn.execute('SELECT COUNT(*) FROM visitors').fetchone()[0], 1)

    def test_real_app_query(self):
        self.record(); self.record('second')
        rows = self.conn.execute(extract('SELECT app, SUM(requests)'), (self.now // 3600 - 24, self.now // 86400 - 1)).fetchall()
        self.assertEqual(len(rows), 2)
        self.assertTrue(all(row[1] == 1 and row[-1] == 1 for row in rows))

    def test_real_chart_country_referrer_queries(self):
        self.record()
        for prefix in ['SELECT hour, SUM(requests)', 'SELECT country, SUM(pages)', 'SELECT referrer, SUM(pages)']:
            self.assertEqual(len(self.conn.execute(extract(prefix), (self.now // 3600 - 24,)).fetchall()), 1)

    def test_retention_removes_old_traffic(self):
        self.record(stamp=self.now - 35 * 86400); self.record()
        cutoff = self.now - 30 * 86400
        self.conn.execute(extract('DELETE FROM traffic WHERE hour < ?1'), (cutoff // 3600,))
        self.conn.execute(extract('DELETE FROM visitors WHERE day < ?1'), (cutoff // 86400,))
        self.assertEqual(self.conn.execute('SELECT SUM(requests) FROM traffic').fetchone()[0], 1)
        self.assertEqual(self.conn.execute('SELECT COUNT(*) FROM visitors').fetchone()[0], 1)

    def test_schema_does_not_contain_raw_request_fields(self):
        sql = '\n'.join(row[0] for row in self.conn.execute("SELECT sql FROM sqlite_master WHERE type='table'"))
        for forbidden in ['ip_address', 'user_agent', 'password', 'cookie', 'request_body', 'query_string']:
            self.assertNotIn(forbidden, sql)

if __name__ == '__main__':
    unittest.main()
