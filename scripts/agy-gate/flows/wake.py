"""``wake``: idle wake into the composer (test-infra spec D7, row ``wake`` (a)-(e), plus two roast rows).

One world, one enrolled agy seat on one thread. A wake is a ``<USER_REQUEST>`` the gate did not type
(``_l7.Typed`` tracks every prompt it did).

a. ACK-required N1 to an idle agy: within 60 s an untyped request arrives and the same request's
   system messages hold N1's frame (the woken turn's first ``PreInvocation`` injected it).
b. Lazy-only Z1: no new main request within 15 s.
c. N2 sent during a ``delay_ms`` busy step: no untyped request; N2 is delivered in-turn at the next
   ``PreInvocation``.
d. A draft is typed, N3 sent: no wake in 15 s and the draft is intact in the detection text.
e. The model runs ``herdr-threads wake-on all`` through ``run_command``; lazy Z2 then wakes.
f. A subagent generating (child ``delay_ms``): N4 sent, no wake while the ``Agent(<name>)`` row is live.
g. A subagent running a tool (child ``run_command`` that blocks): N5 sent, no wake likewise.
"""
from __future__ import annotations

import sys
import time
from pathlib import Path

GATE_DIR = Path(__file__).resolve().parent.parent
for _dir in (GATE_DIR, GATE_DIR / "flows", GATE_DIR.parent / "agy-sandbox"):
    if str(_dir) not in sys.path:
        sys.path.insert(0, str(_dir))

import _l5  # noqa: E402
import _l6  # noqa: E402
import _l7  # noqa: E402
import observe  # noqa: E402
from gate import Flow  # noqa: E402

DRAFT = "WAKE-DRAFT keep me"
# Requests that follow a child's end are the parent's business and may not come (agy decides).
OPTIONAL_SUB = ("g-child-final", "f-parent-after", "g-parent-after")


def settle(world, pane, label):
    world.check("%s: the agent returned to idle" % label, pane.wait("idle", 40))
    if _l6.dismiss_survey(pane):
        world.check("%s: the feedback survey was dismissed" % label, _l6.SURVEY not in _l6.screen_tail(pane, 20))
    time.sleep(2)


def phase_a(world, pane, sender, thread, typed):
    base = _l7.untyped_count(world, typed)
    n1 = _l6.send(world, sender, thread, "WAKE-N1 ack me", "--require-ack-pane", pane.pane_id)
    world.check("a: N1 was sent", bool(n1))
    wake = _l7.await_wake(world, typed, base)
    world.check("a: within 60 s an untyped user request arrives (the wake)", wake is not None,
                "untyped=%s" % _l7.untyped_count(world, typed))
    world.check("a: the same request's system messages hold N1's frame", n1 in _l7.frames_of(wake),
                "frames=%s n1=%s" % (_l7.frames_of(wake), n1))
    world.artifact("a-wake-user-requests.json", (wake or {}).get("user_requests"))
    observe.wait_until(lambda: _l6.receipt(world, n1).get("state") == "acked", 30)
    world.check("a: the woken turn ACKed N1", _l6.receipt(world, n1).get("state") == "acked",
                "receipt=%s" % _l6.receipt(world, n1))
    settle(world, pane, "a")


def phase_b(world, pane, sender, thread, typed):
    z1 = _l6.send(world, sender, thread, "WAKE-Z1 lazy only", "--lazy")
    world.check("b: Z1 was sent", bool(z1))
    requests, untyped = _l7.quiet_window(world, typed)
    world.check("b: no new main request within 15 s", requests == 0, "added=%s untyped=%s" % (requests, untyped))
    world.check("b: Z1 is still pending", _l6.lazy_state(world, z1) == "pending", "state=%s" % _l6.lazy_state(world, z1))


def phase_c(world, pane, sender, thread, typed):
    base = _l7.untyped_count(world, typed)
    typed.prompt(pane, "WAKE-BUSY-C go")
    served = _l5.await_step(world, "c-busy", 30)
    world.check("c: the busy turn was served", served is not None)
    n2 = _l6.send(world, sender, thread, "WAKE-N2 mid turn", "--nudge")
    world.check("c: N2 was sent during the busy step", bool(n2))
    after = _l5.await_step(world, "c-after", 40)
    world.check("c: N2 is delivered in-turn at the next PreInvocation",
                after is not None and n2 in _l7.body_frames(world, after),
                "frames=%s n2=%s" % (_l7.body_frames(world, after), n2))
    settle(world, pane, "c")
    follow_up = [r for r in observe.main_requests(world) if r.get("seq", 0) > (served or {}).get("seq", 0)][:1]
    world.artifact("c-follow-up-body.json", _l7.body_json(world, follow_up[0]) if follow_up else None)
    world.artifact("c-receipts.json", {"N2": _l6.receipt(world, n2), "tables": _l6.tables(world, "receipt_state")})
    lines, seen = [], set()
    for path in pane.transcripts():
        try:
            text = path.read_text(errors="replace")
        except OSError:
            continue
        for line in text.splitlines():
            if n2 and n2 in line and line not in seen:
                seen.add(line)
                lines.append(line)
    world.artifact("c-transcript-n2.jsonl", "\n".join(lines) + "\n")
    settled = observe.wait_until(lambda: _l6.receipt(world, n2).get("provenance") == "cooperative_hook_delivery", 20)
    world.check("c: N2 settles (cooperative_hook_delivery) after the post-tool-call injection", bool(settled),
                "N2=%s" % _l6.receipt(world, n2))
    record = _l6.downgrade_record(world)
    world.check("c: no injection miss is recorded and the identity is not downgraded",
                record is None or (not record.get("misses") and record.get("downgraded_at") is None),
                "downgrade.json=%s" % record)
    world.check("c: no untyped user request during or after the busy turn", _l7.untyped_count(world, typed) == base,
                "untyped %s -> %s" % (base, _l7.untyped_count(world, typed)))


def phase_d(world, pane, sender, thread, typed):
    base, requests = _l7.untyped_count(world, typed), len(observe.main_requests(world))
    pane.draft(DRAFT)
    observe.wait_until(lambda: DRAFT in pane.read_detection(), 10)
    n3 = _l6.send(world, sender, thread, "WAKE-N3 while drafting", "--nudge")
    world.check("d: N3 was sent", bool(n3))
    added, untyped = _l7.quiet_window(world, typed)
    detection = pane.read_detection()
    world.artifact("d-detection.txt", detection)
    world.check("d: no wake while a draft is in the composer", added == 0 and untyped == 0,
                "added=%s untyped=%s" % (added, untyped))
    world.check("d: the draft is intact in the detection text", DRAFT in detection)
    world.check("d: no request was served while drafting", len(observe.main_requests(world)) == requests)
    typed.submit_draft(pane, DRAFT)
    served = _l5.await_step(world, "d-draft", 30)
    world.check("d: the submitted draft is the turn that delivers N3",
                served is not None and n3 in _l7.frames_of(served), "frames=%s" % _l7.frames_of(served))
    settle(world, pane, "d")
    world.check("d: still no untyped user request", _l7.untyped_count(world, typed) == base)


def phase_e(world, pane, sender, thread, typed):
    typed.prompt(pane, "WAKE-SETON go")
    called = _l5.await_step(world, "e-set", 30)
    world.check("e: the model's wake-on call was served", called is not None)
    done = _l5.await_step(world, "e-done", 30)
    world.check("e: the request after the call carries a run_command functionResponse",
                done is not None and any(r.get("name") == "run_command" for r in done.get("function_responses") or []),
                "responses=%s" % (done or {}).get("function_responses"))
    settle(world, pane, "e")
    # Read from the daemon database: `wake-on` run from the gate process is an unattributable caller (agent seats
    # have no pane-context journal); only the agy session itself, through run_command, may act as the seat.
    seats = _l5.query(world, "SELECT DISTINCT s.id, s.wake_on FROM seats s JOIN occupant_bindings b ON b.seat_id=s.id "
                             "WHERE b.harness='agy' AND b.role='top_level' AND b.liveness='connected'")
    world.artifact("e-wake-on.json", seats)
    world.check("e: the seat's wake preference is now all", len(seats) == 1 and seats[0]["wake_on"] == "all",
                "seats=%s" % seats)
    base = _l7.untyped_count(world, typed)
    z2 = _l6.send(world, sender, thread, "WAKE-Z2 lazy", "--lazy")
    world.check("e: Z2 was sent", bool(z2))
    wake = _l7.await_wake(world, typed, base)
    world.check("e: lazy Z2 wakes the idle agent", wake is not None, "untyped=%s" % _l7.untyped_count(world, typed))
    world.check("e: the woken request holds Z2's frame", z2 in _l7.frames_of(wake), "frames=%s" % _l7.frames_of(wake))
    settle(world, pane, "e")


def subagent_phase(world, pane, sender, thread, typed, tag, prompt, child_step, final_step, body, window,
                   after=True):
    """The parent finishes at once while the child is held (research B), so Herdr reports the pane idle
    with an ``Agent(<name>)`` row live: no wake may type into it. Once the row is gone the pending
    note is owed (a wake, or an in-turn delivery)."""
    base = _l7.untyped_count(world, typed)
    typed.prompt(pane, prompt)
    child = _l5.await_step(world, child_step, 40)
    world.check("%s: the child request was served" % tag, child is not None)
    note = _l6.send(world, sender, thread, body, "--nudge")
    world.check("%s: the note was sent" % tag, bool(note))
    time.sleep(2)
    live = _l7.agent_row_live(pane)
    world.artifact("%s-detection.txt" % tag, pane.read_detection())
    world.check("%s: the Agent(<name>) row is live" % tag, live, "detection tail=%s" % pane.read_detection()[-300:])
    requests, untyped = _l7.quiet_window(world, typed, window)
    world.check("%s: no wake while the Agent row is live" % tag, untyped == 0,
                "main requests added=%s untyped=%s" % (requests, untyped))
    world.check("%s: the parent finished its turn" % tag, _l5.await_step(world, final_step, 60) is not None)
    if not after:
        # A blocked tool keeps the row until agy exits: the row stays live and still nothing wakes.
        requests, untyped = _l7.quiet_window(world, typed, window)
        world.check("%s: the row is still live and still no wake" % tag, untyped == 0 and _l7.agent_row_live(pane),
                    "main requests added=%s untyped=%s" % (requests, untyped))
        return
    gone = observe.wait_until(lambda: not _l7.agent_row_live(pane), 90)
    world.check("%s: the Agent row cleared" % tag, bool(gone))
    wake = _l7.await_wake(world, typed, base)
    delivered = [r for r in observe.main_requests(world) if note in _l7.frames_of(r)]
    world.artifact("%s-after.json" % tag, {"note": note, "wake": bool(wake), "delivered_in": [r.get("seq") for r in delivered]})
    world.check("%s: once the row is gone the pending note is delivered" % tag, bool(delivered),
                "wake=%s untyped=%s" % (bool(wake), _l7.untyped_count(world, typed)))
    settle(world, pane, tag)


def run(world):
    setup = _l6.setup(world)
    world.check("setup agy succeeds", setup.returncode == 0, (setup.stderr or setup.stdout).strip()[:200])
    sender = world.sender()
    # The thread exists before agy starts: the scripted accept (wake-join) names it.
    thread = _l6.create_thread(world, sender, "gate-wake")
    typed = _l7.Typed()
    pane = _l6.start_agy(world)
    scenario = _l7.load(world, "wake", thread=thread)
    typed.prompt(pane, "WAKE-WARM hello")
    _l5.await_step(world, "warm", 30)
    pane.wait("idle", 30)
    binding = _l5.await_binding(world, lambda b: True)
    world.check("the agy seat is enrolled", binding is not None)
    if binding is None:
        return
    invite, accept = _l6.join(world, sender, pane, thread, binding["seat_id"])
    world.check("the sender invited the agy seat and the agy session accepted (run_command, joined)",
                invite.returncode == 0 and accept.ok, "invite=%s accept=%s" % (invite.stderr.strip(), accept.detail))
    _l5.await_step(world, "wake-join", 10)
    settle(world, pane, "join")
    phase_a(world, pane, sender, thread, typed)
    phase_b(world, pane, sender, thread, typed)
    phase_c(world, pane, sender, thread, typed)
    phase_d(world, pane, sender, thread, typed)
    phase_e(world, pane, sender, thread, typed)
    world.check("wake: every step consumed", not _l6.unconsumed(world, scenario, ("wake-join",)),
                "unconsumed=%s" % _l6.unconsumed(world, scenario, ("wake-join",)))
    sub = _l7.load(world, "wake-subagent", thread=thread)
    subagent_phase(world, pane, sender, thread, typed, "f", "WAKE-PGEN go", "f-child", "f-parent-final",
                   "WAKE-N4 during a generating subagent", 12)
    subagent_phase(world, pane, sender, thread, typed, "g", "WAKE-PTOOL go", "g-child-call", "g-parent-final",
                   "WAKE-N5 during a subagent tool", 12, after=False)
    world.check("subagent: every step consumed", not _l6.unconsumed(world, sub, OPTIONAL_SUB),
                "unconsumed=%s" % _l6.unconsumed(world, sub, OPTIONAL_SUB))
    clean, detail = observe.backend_clean(world)
    world.check("no unmatched or template-error request", clean, detail)
    world.artifact("requests.json", world.backend.requests())
    pane.exit()


FLOW = Flow(name="wake", requires=("ht-poh.4.5", "ht-poh.5", "ht-poh.3.4", "ht-poh.3.6"), run=run)
