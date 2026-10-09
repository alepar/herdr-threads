"""Unit tests of the live stress driver's pure helpers (ht-j16.9). No Claude needed."""

import json
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import stress  # noqa: E402


def row(at, kind, ids=(), via=None, turn=None, reason=None):
    return {"at": at, "kind": kind, "ids": list(ids), "via": via, "turn": turn, "reason": reason}


class LedgerParse(unittest.TestCase):
    def test_parses_lines_and_skips_blank_and_torn_tail(self):
        text = json.dumps(row(1, "received", ["m1"])) + "\n\n" + '{"at": 2, "kind": "deli'
        self.assertEqual([r["kind"] for r in stress.parse_ledger(text)], ["received"])

    def test_rejects_a_complete_line_that_is_not_an_object(self):
        with self.assertRaises(ValueError):
            stress.parse_ledger("[1]\n")


class LedgerInvariants(unittest.TestCase):
    def test_clean_ledger_has_no_violations(self):
        led = [
            row(1, "received", ["m1"]),
            row(2, "delivered", ["m1"], via="context", turn="t1"),
            row(3, "acked", ["m1"], via="context", reason="settled"),
            row(4, "received", ["m2"]),
            row(5, "submit", ["m2"], via="submit"),
            row(6, "delivered", ["m2"], via="submit"),
            row(7, "acked", ["m2"], via="submit", reason="settled"),
        ]
        self.assertEqual(stress.check_ledger(led), [])

    def test_duplicate_delivery_is_flagged(self):
        led = [row(1, "delivered", ["m1"], via="context"), row(2, "delivered", ["m1"], via="submit")]
        self.assertEqual(len(stress.check_ledger(led)), 1)
        self.assertIn("duplicate delivered m1", stress.check_ledger(led)[0])

    def test_duplicate_settling_ack_is_flagged_but_already_settled_is_not(self):
        ok = [row(1, "acked", ["m1"], reason="settled"), row(2, "acked", ["m1"], reason="already_settled")]
        self.assertEqual(stress.check_ledger(ok), [])
        bad = [row(1, "acked", ["m1"], reason="settled"), row(2, "acked", ["m1"], reason="settled")]
        self.assertIn("duplicate settling ack m1", stress.check_ledger(bad)[0])

    def test_submit_with_open_turn_is_flagged(self):
        led = [row(1, "submit", ["m1"], via="submit", turn="t7")]
        self.assertIn("submit while turn t7 open", stress.check_ledger(led)[0])

    def test_submit_with_no_turn_is_clean(self):
        self.assertEqual(stress.check_ledger([row(1, "submit", ["m1"], via="submit")]), [])

    def test_clear_reset_allows_redelivery_after_it(self):
        led = [row(1, "delivered", ["m1"]), row(10, "delivered", ["m1"])]
        self.assertEqual(stress.check_ledger(led, resets=[5]), [])
        self.assertEqual(len(stress.check_ledger(led, resets=[0])), 1)  # reset before both: still one segment

    def test_stale_generation_refusal_allows_one_redelivery(self):
        led = [
            row(1, "delivered", ["m1"]),
            row(2, "refused", ["m1"], via="context", reason="ack:stale_generation"),
            row(3, "delivered", ["m1"]),
        ]
        self.assertEqual(stress.check_ledger(led), [])
        led.append(row(4, "delivered", ["m1"]))
        self.assertEqual(len(stress.check_ledger(led)), 1)

    def test_ack_of_a_truncated_id_is_flagged(self):
        led = [row(1, "acked", ["m9"], reason="settled")]
        self.assertIn("ack for truncated m9", stress.check_ledger(led, truncated={"m9"})[0])
        self.assertEqual(stress.check_ledger(led, truncated={"m1"}), [])

    def test_submit_inside_post_abort_hold_is_flagged(self):
        led = [row(50, "submit", ["m1"], via="submit")]
        self.assertIn("submit inside hold", stress.check_ledger(led, holds=[(40, 60)])[0])
        self.assertEqual(stress.check_ledger(led, holds=[(40, 50)]), [])  # end is exclusive
        self.assertEqual(stress.check_ledger(led, holds=[(60, 70)]), [])


class Settlement(unittest.TestCase):
    def test_every_sent_id_acked_or_pending(self):
        self.assertEqual(stress.check_settled(["a", "b"], {"a": "acked", "b": "pending"}, []), [])

    def test_missing_receipt_is_a_lost_message(self):
        self.assertIn("lost message b", stress.check_settled(["a", "b"], {"a": "acked"}, [])[0])

    def test_unexpected_state_is_flagged(self):
        self.assertIn("state recipient_retired", stress.check_settled(["a"], {"a": "recipient_retired"}, [])[0])

    def test_ledger_settled_but_daemon_pending_is_flagged(self):
        led = [row(1, "acked", ["a"], reason="settled")]
        self.assertIn("ledger settled a but daemon pending", stress.check_settled(["a"], {"a": "pending"}, led)[0])

    def test_ledger_already_settled_with_pending_daemon_is_not_flagged_here(self):
        led = [row(1, "acked", ["a"], reason="already_settled")]
        self.assertEqual(stress.check_settled(["a"], {"a": "pending"}, led), [])

    def test_ack_without_a_mod_ledger_ack_is_flagged(self):
        led = [row(1, "acked", ["a"], reason="settled"), row(2, "acked", ["b"], reason="already_settled")]
        rc = {"a": "acked", "b": "acked", "c": "acked", "d": "pending"}
        self.assertEqual(stress.check_mod_settled(["a", "b", "c", "d"], rc, led), ["c acked outside the mod (no ledger ack)"])


class Args(unittest.TestCase):
    def test_defaults(self):
        a = stress.parse_args([])
        self.assertEqual(a.iterations, 20)
        self.assertEqual(a.scenarios, stress.SCENARIO_NAMES)

    def test_scenario_list_and_unknown(self):
        self.assertEqual(stress.parse_args(["--scenarios", "reload,busy_context"]).scenarios, ["reload", "busy_context"])
        with self.assertRaises(SystemExit):
            stress.parse_args(["--scenarios", "nope"])

    def test_iterations_must_be_positive(self):
        with self.assertRaises(SystemExit):
            stress.parse_args(["--iterations", "0"])


class Versions(unittest.TestCase):
    def test_version_at_least(self):
        self.assertTrue(stress.version_at_least("2.1.295", "2.1.287"))
        self.assertFalse(stress.version_at_least("2.1.286", "2.1.287"))
        self.assertTrue(stress.version_at_least("2.2", "2.1.287"))

    def test_signed_out_output_is_not_usable(self):
        self.assertFalse(stress.auth_output_usable("Not logged in · Please run /login"))
        self.assertTrue(stress.auth_output_usable("ok"))
        self.assertFalse(stress.auth_output_usable(""))


def hook(at, event, blocked=False):
    return {"at": at, "event": event, "blocked": blocked}


class TurnOverlap(unittest.TestCase):
    def test_intervals_from_prompt_to_stop_with_continuation_and_abort(self):
        evs = [hook(10, "UserPromptSubmit"), hook(20, "Stop", blocked=True), hook(30, "Stop"),
               hook(40, "UserPromptSubmit"), hook(45, "UserPromptSubmit"),  # a queued prompt folds in
               hook(60, "UserPromptSubmit", blocked=True), hook(70, "UserPromptSubmit")]
        self.assertEqual(stress.turn_intervals(evs, aborts=[50]),
                         [(10, 30), (40, 50), (70, float("inf"))])

    def test_submit_inside_an_open_turn_is_flagged_and_its_own_turn_is_not(self):
        evs = [hook(10, "UserPromptSubmit"), hook(30, "Stop"), hook(41, "UserPromptSubmit"), hook(50, "Stop")]
        self.assertEqual(stress.check_turn_overlap([row(40, "submit", ["m1"])], evs), [])
        self.assertIn("submit m2 at 20 inside a turn", stress.check_turn_overlap([row(20, "submit", ["m2"])], evs)[0])

    def test_blocked_prompt_opens_no_turn(self):
        evs = [hook(10, "UserPromptSubmit", blocked=True)]
        self.assertEqual(stress.check_turn_overlap([row(20, "submit", ["m1"])], evs), [])


class MergeLines(unittest.TestCase):
    def test_rewrites_and_a_reload_reset_merge_without_duplicates(self):
        rows, seen = [], set()
        a, b, c = (json.dumps(row(t, "received", ["m%d" % t])) for t in (1, 2, 3))
        self.assertEqual(stress.merge_lines(rows, seen, a + "\n"), 1)
        self.assertEqual(stress.merge_lines(rows, seen, a + "\n" + b + "\n"), 1)
        self.assertEqual(stress.merge_lines(rows, seen, c + "\n" + '{"at": 4, "ki'), 1)  # reload: new file, torn tail
        self.assertEqual([r["at"] for r in rows], [1, 2, 3])


if __name__ == "__main__":
    unittest.main()
