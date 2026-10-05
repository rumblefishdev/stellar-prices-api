"""reingest_0286.py after the 0139 asset-id migration, against a fake ClickHouse.

python3 -m unittest discover -s tools/scripts -p 'reingest_0286_post0139_test.py'
"""

import contextlib
import importlib.util
import io
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
spec = importlib.util.spec_from_file_location("reingest_0286", HERE / "reingest_0286.py")
r = importlib.util.module_from_spec(spec)
spec.loader.exec_module(r)

BAK_1M = "reingest_0286_bak_price_ohlcv_1m"
RESTRICT = " ".join(r.MAP_RESTRICT.split())


class Halt(Exception):
    """Raised by the fake to end run_month after the step under test."""


class FakeCH:
    """Answers each statement with the first rule whose needle it contains."""

    def __init__(self, rules, dry=False):
        self.db, self.dry, self.url = "prices", dry, "https://ch.test"
        self.rules, self.calls = list(rules), []

    def _answer(self, role, sql):
        sql = " ".join(sql.split())
        self.calls.append((role, sql))
        for needle, ans in self.rules:
            if needle in sql:
                return ans(sql) if callable(ans) else ans
        raise AssertionError(f"unexpected SQL ({role}): {sql[:240]}")

    def rows(self, role, sql, **kw):
        return self._answer(role, sql)

    def one(self, role, sql, **kw):
        rows = self.rows(role, sql, **kw)
        return rows[0][0] if rows else None

    def q(self, role, sql, **kw):
        self._answer(role, sql)
        return ""

    def write(self, role, sql, **kw):
        return "" if self.dry else self.q(role, sql, **kw)

    def writes(self):
        return [s for _, s in self.calls if s.startswith(("CREATE", "ALTER", "DROP", "TRUNCATE", "INSERT"))]


def id_types(widths):
    """The id-columns rule: {table: 'UInt32' | 'UInt64'}; an absent table has no columns."""
    def answer(sql):
        t = re.search(r"table = '([^']+)'", sql).group(1)
        return [["asset_id", widths[t]], ["quote_asset_id", widths[t]]] if t in widths else []
    return ("name IN ('asset_id', 'quote_asset_id') ORDER BY name", answer)


def parts(rows_by_table):
    def answer(sql):
        t = re.search(r"table = '([^']+)'", sql).group(1)
        return [[str(rows_by_table.get(t, 0))]]
    return ("FROM system.parts", answer)


def args(*extra):
    return r.build_parser().parse_args(list(extra))


def state(months, tmp):
    st = r.State(tmp, False)
    st.d["plan"] = {str(m): {"start": 1000 * i + 1, "end": 1000 * (i + 1)} for i, m in enumerate(months)}
    return st


TIER_TABLES = [f"price_ohlcv_{t}" for t in ("1m", "15m", "1h", "4h", "1d", "1w", "1M")]


def pf_columns(sql):
    """Three pf_* columns on every tier and on its __pre0139 copy; count what the SQL selects."""
    tables = TIER_TABLES + [f"{t}__pre0139" for t in TIER_TABLES]
    if "table LIKE 'price_ohlcv_%'" in sql:
        return [[str(3 * len(tables))]]
    return [[str(3 * sum(f"'{t}'" in sql for t in tables))]]


def preflight_rules(width, old_baks=(), map_rows=5):
    return [
        ("startsWith(table, 'reingest_0286_bak_')", [[t] for t in old_baks]),
        id_types({"price_ohlcv_1m": width}),
        ("name = 'asset_id_map_0139'", [[str(map_rows)]]),
        ("SELECT version()", [["26.3.10.60", "UTC", "UTC"]]),
        ("pf_trade_count", pf_columns),
        ("CREATE TABLE IF NOT EXISTS", []),
        ("PARTITION 190001", []),
        ("free_space", [[str(2 ** 50)]]),
        ("ingest_cursor", [["99999999"]]),
    ]


def preflight(ch, a, st):
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        try:
            r.cmd_preflight(a, ch, st)
            return None, out.getvalue()
        except r.Stop as e:
            return e, out.getvalue()


PF = ["preflight", "--skip-aws-check", "--ack-phase1-measured", "--repo", str(REPO),
      "--sdex-backfill", sys.executable]


class Detection(unittest.TestCase):
    def test_uint64_1m_is_post0139(self):
        self.assertTrue(r.post0139_mode(FakeCH([id_types({"price_ohlcv_1m": "UInt64"})])))

    def test_uint32_or_missing_1m_is_not(self):
        self.assertFalse(r.post0139_mode(FakeCH([id_types({"price_ohlcv_1m": "UInt32"})])))
        self.assertFalse(r.post0139_mode(FakeCH([id_types({})])))

    def test_flags_exist(self):
        out = subprocess.run([sys.executable, str(HERE / "reingest_0286.py"), "--help"],
                             capture_output=True, text=True).stdout
        for flag in ("--ack-0139-binaries", "--second-pass", "--min-excluded-rows"):
            self.assertIn(flag, out)


class Preflight(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.st = state([202201], self.tmp.name)

    def tearDown(self):
        self.tmp.cleanup()

    def test_post0139_refuses_without_the_binaries_ack(self):
        err, out = preflight(FakeCH(preflight_rules("UInt64")), args(*PF), self.st)
        self.assertIsNotNone(err)
        self.assertRegex(out, r"STOP 0139 binaries")

    def test_the_pf_gate_counts_the_live_tiers_not_their_pre0139_copies(self):
        err, out = preflight(FakeCH(preflight_rules("UInt64")), args(*PF, "--ack-0139-binaries"), self.st)
        self.assertIn("ok   phase 1 schema: 21/21 pf columns", out)

    def test_post0139_passes_with_the_ack(self):
        err, out = preflight(FakeCH(preflight_rules("UInt64")), args(*PF, "--ack-0139-binaries"), self.st)
        self.assertIsNone(err, out)
        self.assertRegex(out, r"ok   0139 snapshots")

    def test_post0139_refuses_old_shape_snapshots_and_skips_the_grants_probe(self):
        ch = FakeCH(preflight_rules("UInt64", old_baks=[BAK_1M, "reingest_0286_bak_price_ohlcv_1d"]))
        err, out = preflight(ch, args(*PF, "--ack-0139-binaries"), self.st)
        self.assertIsNotNone(err)
        self.assertRegex(out, r"STOP 0139 snapshots: .*reingest_0286_bak_price_ohlcv_1m.*_pre0139")
        self.assertEqual(ch.writes(), [], "no CREATE/ATTACH next to an old-shape snapshot")
        old_q = next(s for _, s in ch.calls if "startsWith(table, 'reingest_0286_bak_')" in s)
        self.assertIn("NOT endsWith(table, '_pre0139')", old_q)
        self.assertIn("'UInt64'", old_q)

    def test_post0139_refuses_without_the_map(self):
        err, out = preflight(FakeCH(preflight_rules("UInt64", map_rows=0)),
                             args(*PF, "--ack-0139-binaries"), self.st)
        self.assertIsNotNone(err)
        self.assertRegex(out, r"STOP 0139 map")

    def test_pre0139_needs_no_ack_and_reads_no_0139_tables(self):
        ch = FakeCH(preflight_rules("UInt32", old_baks=[BAK_1M], map_rows=0))
        err, out = preflight(ch, args(*PF), self.st)
        self.assertIsNone(err, out)
        self.assertNotRegex(out, r"note 0139|0139 (binaries|map|snapshots)")
        self.assertFalse(any("asset_id_map_0139" in s or "startsWith(table" in s for _, s in ch.calls))


class Snapshot(unittest.TestCase):
    def test_refuses_an_old_shape_backup_before_kept_or_attach(self):
        ch = FakeCH([("CREATE TABLE IF NOT EXISTS", []),
                     id_types({"price_ohlcv_1m": "UInt64", BAK_1M: "UInt32"}),
                     parts({BAK_1M: 7, "price_ohlcv_1m": 9})])
        with self.assertRaisesRegex(r.Stop, r"_pre0139"):
            r.snapshot(ch, "price_ohlcv_1m", 202201)
        self.assertFalse(any("ATTACH" in s for s in ch.writes()))

    def test_same_shape_backup_snapshots(self):
        ch = FakeCH([("CREATE TABLE IF NOT EXISTS", []), ("ATTACH PARTITION", []),
                     id_types({"price_ohlcv_1m": "UInt64", BAK_1M: "UInt64"}),
                     parts({"price_ohlcv_1m": 9})])
        rows = iter([0, 9, 9])  # bak empty -> source 9 -> bak 9 after ATTACH
        ch.rules[-1] = ("FROM system.parts", lambda sql: [[str(next(rows))]])
        self.assertEqual(r.snapshot(ch, "price_ohlcv_1m", 202201), "9 rows")


class Rollback(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.st = state([202201], self.tmp.name)
        self.st.month(202201)["snap"] = {"1m": "9 rows", "15m": "empty"}

    def tearDown(self):
        self.tmp.cleanup()

    def rollback(self, ch):
        with contextlib.redirect_stdout(io.StringIO()):
            r.cmd_rollback(args("rollback", "202201"), ch, self.st)

    def test_old_shape_snapshot_refuses_and_points_to_pre0139(self):
        ch = FakeCH([id_types({"price_ohlcv_1m": "UInt64", BAK_1M: "UInt32"}), parts({BAK_1M: 9})])
        with self.assertRaises(r.Stop) as e:
            self.rollback(ch)
        self.assertIn(f"{BAK_1M}_pre0139", str(e.exception))
        self.assertIn("price_ohlcv_1m__pre0139", str(e.exception))
        self.assertEqual(ch.writes(), [])

    def test_a_backup_without_the_month_refuses_instead_of_emptying_it(self):
        ch = FakeCH([id_types({"price_ohlcv_1m": "UInt64", BAK_1M: "UInt64"}), parts({BAK_1M: 0})])
        with self.assertRaisesRegex(r.Stop, r"no 202201 rows.*_pre0139"):
            self.rollback(ch)
        self.assertEqual(ch.writes(), [])

    def test_same_shape_snapshot_restores(self):
        ch = FakeCH([id_types({"price_ohlcv_1m": "UInt64", BAK_1M: "UInt64"}), parts({BAK_1M: 9}),
                     ("ALTER TABLE", [])])
        self.rollback(ch)
        self.assertEqual(len(ch.writes()), 2)
        self.assertNotIn("202201", self.st.d["months"])


SUMS_ALL = [["sdex", "10", "1100", "50", "60"]]       # 100 trades restored under colliding ids
SUMS_COPIED = [["sdex", "9", "1000", "40", "50"]]


class Reconcile(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        r.DASH["notes"] = []

    def tearDown(self):
        self.tmp.cleanup()

    def run_reconcile(self, after_copied, ids="u64", width="UInt64", accept=()):
        st = state([202201], self.tmp.name)
        ms = st.month(202201)
        ms.update(step=r.STEPS.index("reconcile"), ref="1m", ids=ids,
                  before={"sdex": {"candles": 9, "trades": 1000, "vb": "40", "vq": "50"}},
                  outside_before={})

        def sums(sql):
            return after_copied if RESTRICT in sql else SUMS_ALL

        def drop(sql):
            raise Halt()

        ch = FakeCH([id_types({"price_ohlcv_1m": width}),
                     ("AS class", [["sdex", "colliding", "100"]]),
                     ("GROUP BY source ORDER BY source", sums),
                     ("DROP PARTITION", drop)])
        a = args("run", *[f"--accept={m}" for m in accept])
        err = None
        with contextlib.redirect_stdout(io.StringIO()):
            try:
                r.run_month(a, ch, st, 202201, None)
            except Halt:
                pass
            except r.Stop as e:
                err = e
        return ms, ch, err

    def test_restricts_both_ids_to_copied_map_rows(self):
        sql = " ".join(r.SUMS.format(tier="1m", m=202201, restrict=r.MAP_RESTRICT).split())
        self.assertIn("asset_id IN (SELECT new_id FROM prices.asset_id_map_0139", sql)
        self.assertIn("quote_asset_id IN (SELECT new_id FROM prices.asset_id_map_0139", sql)
        self.assertIn("status IN ('mapped', 'sentinel')", sql)
        self.assertIn("countIf(status = 'colliding') = 0", sql)

    def test_restored_colliding_trades_are_information_not_a_defect(self):
        ms, ch, err = self.run_reconcile(SUMS_COPIED)
        self.assertIsNone(err)
        self.assertEqual(ms["result"]["verdict"], "OK")
        self.assertEqual(ms["outside_after"], {"sdex colliding": 100})
        self.assertTrue(any("0 -> 100" in n and "information" in n for n in r.DASH["notes"]))
        self.assertTrue(all(RESTRICT in s for _, s in ch.calls if "GROUP BY source ORDER BY" in s))

    def test_a_gain_on_copied_ids_is_still_a_defect(self):
        ms, _, err = self.run_reconcile([["sdex", "9", "1010", "40", "50"]])
        self.assertEqual(ms["result"]["verdict"], "DEFECT")
        self.assertRegex(str(err), "reconciliation failed")

    def test_before_on_the_old_width_stops_unless_accepted(self):
        ms, _, err = self.run_reconcile(SUMS_COPIED, ids="u32")
        self.assertRegex(str(err), r"0139 window")
        self.assertNotIn("result", ms)
        ms, _, err = self.run_reconcile(SUMS_COPIED, ids="u32", accept=[202201])
        self.assertIsNone(err)
        self.assertEqual(ms["result"]["verdict"], "OK")

    def test_pre0139_reconciles_unrestricted(self):
        ms, ch, _ = self.run_reconcile(SUMS_COPIED, ids="u32", width="UInt32")
        self.assertEqual(ms["result"]["verdict"], "DEFECT")  # +10 % on the unrestricted sums
        self.assertFalse(any("asset_id_map_0139" in s for _, s in ch.calls))


class SecondPass(unittest.TestCase):
    LISTED = [["201608", "3", "0"], ["201901", "1", "1"], ["202105", "500", "0"], ["202203", "40", "0"]]

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.st = state([201608, 201609, 201901, 202105, 202112, 202201, 202203], self.tmp.name)
        r.LOG = Path(self.tmp.name) / "run.log"

    def tearDown(self):
        r.LOG = None
        self.tmp.cleanup()

    def months(self, *extra, width="UInt64", command="run"):
        ch = FakeCH([id_types({"price_ohlcv_1m": width}), ("rekey_0139_reingest_months", self.LISTED)])
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            got = r.planned_months(args(command, "--second-pass", *extra), self.st, ch)
        return got, out.getvalue()

    def test_status_prints_the_skips_without_logging_them(self):
        got, out = self.months("--to-month", "202112", "--min-excluded-rows", "3", command="status")
        self.assertEqual(got, [202105])
        self.assertRegex(out, r"SKIPS 201608")
        self.assertFalse(r.LOG.exists())

    def test_every_listed_month_up_to_the_pause(self):
        got, _ = self.months("--to-month", "202112")
        self.assertEqual(got, [201608, 201901, 202105])

    def test_threshold_skips_small_months_and_records_them(self):
        got, out = self.months("--to-month", "202112", "--min-excluded-rows", "3")
        self.assertEqual(got, [202105])
        self.assertRegex(out, r"SKIPS 201608: 3 excluded")
        self.assertRegex(out, r"SKIPS 201901: 2 excluded")
        self.assertEqual(self.st.d["second_pass"]["skipped"], {"201608": 3, "201901": 2})
        self.assertIn("SKIPS 201608", r.LOG.read_text())

    def test_refuses_without_the_pause_month(self):
        with self.assertRaisesRegex(r.Stop, r"--to-month"):
            self.months()

    def test_refuses_before_the_swap(self):
        with self.assertRaisesRegex(r.Stop, r"0139"):
            self.months("--to-month", "202112", width="UInt32")

    def test_refuses_a_listed_month_the_plan_lacks(self):
        self.st.d["plan"].pop("201901")
        with self.assertRaisesRegex(r.Stop, r"201901"):
            self.months("--to-month", "202112")


class NoLiteralIds(unittest.TestCase):
    def test_operator_text_names_reference_assets_by_identity(self):
        src = (HERE / "reingest_0286.py").read_text()
        self.assertNotRegex(src, r"asset_id\s*(=|IN)\s*\(?\s*\d")


if __name__ == "__main__":
    unittest.main()
