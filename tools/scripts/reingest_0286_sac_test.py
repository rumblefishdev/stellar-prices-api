"""reingest_0286.py reads events-backfill's task-0242 `unproven sac swaps:` line.

python3 -m unittest discover -s tools/scripts -p 'reingest_0286_sac_test.py'
"""

import importlib.util
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
spec = importlib.util.spec_from_file_location("reingest_0286", HERE / "reingest_0286.py")
r = importlib.util.module_from_spec(spec)
spec.loader.exec_module(r)

LOG = Path("/m/202609/events-backfill.log")


def summary(unproven=None, fallbacks=True):
    """events-backfill's closing summary; `unproven=None` is a pre-0242 binary."""
    lines = ["=== events-backfill complete ===",
             "swaps dropped (unresolved):0",
             "swaps failed dispatch:     0"]
    if fallbacks:
        lines.append("negative apply order:      0")
    if unproven is not None:
        lines.append(f"unproven sac swaps:        {unproven}")
    return "\n".join(lines) + "\n"


class UnprovenSacLine(unittest.TestCase):
    def test_the_line_is_read_and_only_non_zero_stops(self):
        for unproven, expected in ((0, 0), (None, None)):
            with self.subTest(unproven=unproven):
                found = r.amm_summary(summary(unproven), LOG, dry_run=True)
                self.assertEqual(found["unproven_sacs"], expected)
                self.assertEqual((found["fallbacks"], found["dropped"]), (0, 0))

    def test_a_non_zero_count_stops_naming_it_and_what_the_month_holds(self):
        # The dry run comes after drop_1m and sdex: never "nothing was written".
        for dry_run, says in ((True, ("--dry-run pass", "no AMM candle was written",
                                      "202609's 1m partition is already dropped", "SDEX-only",
                                      "Fix it today", "`rollback 202609`")),
                              (False, ("write pass", "AMM candles are in without those swaps",
                                       "`rollback 202609`"))):
            with self.subTest(dry_run=dry_run), self.assertRaises(r.Stop) as cm:
                r.amm_summary(summary(3), LOG, dry_run=dry_run)
            msg = str(cm.exception)
            for part in ("3 swaps", "task 0242 D2", str(LOG),
                         "docs/runbooks/0242-sac-identity-heal.md §4c") + says:
                self.assertIn(part, msg)
            self.assertNotIn("nothing was written", msg)

    def test_a_missing_required_line_still_stops(self):
        with self.assertRaises(r.Stop) as cm:
            r.amm_summary(summary(0, fallbacks=False), LOG, dry_run=True)
        self.assertIn("negative apply order:", str(cm.exception))

    def test_the_label_is_the_one_events_backfill_prints(self):
        run_rs = (REPO / "packages/events-backfill/src/run.rs").read_text()
        self.assertIn(f'UNPROVEN_SAC_LABEL: &str = "{r.UNPROVEN_SAC_LABEL}"', run_rs)


if __name__ == "__main__":
    unittest.main()
