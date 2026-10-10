"""``delivery``: hook-pull delivery with transcript confirmation (test-infra spec D7, row ``delivery``).

One world, four scenarios in turn, each on the same enrolled agy seat and thread. A phase keeps the
agent busy on a scripted turn while the sender sends, so nothing wakes the idle agent early; the next
turn's first invocation then finds every item pending and the ``PreInvocation`` hook injects them.

1. ``delivery-stop``: an item injected on a turn's only model call is settled at ``Stop``
   (``cooperative_hook_delivery``) and no idle wake follows.
2. ``delivery-cancel``: an Esc-cancelled turn that injected an item records no injection miss.
3. ``delivery-error``: a turn whose model call the fake server fails records no injection miss.
4. ``delivery``: lazy L1, ordinary O1, ACK-required A1 and an oversize T1 arrive in one page; inv0
   holds, so nothing is settled before the confirm; the model ACKs A1 through ``run_command`` (the
   allow entry, no permission prompt); O1 settles at the next ``PreInvocation``, L1 completes, T1
   stays pending, and the frame headers are in the transcript.
"""
from __future__ import annotations

import sys
import time
from pathlib import Path

GATE_DIR = Path(__file__).resolve().parent.parent
for _dir in (GATE_DIR, GATE_DIR / "flows", GATE_DIR.parent / "agy-sandbox"):
    if str(_dir) not in sys.path:
        sys.path.insert(0, str(_dir))

import _l4  # noqa: E402
import _l5  # noqa: E402
import _l6  # noqa: E402
import observe  # noqa: E402
from gate import Flow  # noqa: E402

SCEN = GATE_DIR / "scenarios"
HOOK_PROVENANCE = "cooperative_hook_delivery"


def load(world, name):
    scenario = _l6.load_scenario(SCEN / (name + ".json"))
    scenario["vars"] = dict(scenario.get("vars", {}), cwd=str(world.ws))
    world.backend.load(scenario)
    return scenario


def busy_turn_send(world, pane, sender, thread, prompt, busy_label, sends):
    """Start a scripted busy turn, send ``sends`` (a list of ``(body, flags)``) while it runs, wait for
    its end. Returns the sent message ids, in order."""
    if _l6.dismiss_survey(pane):
        world.check("%s: the feedback survey was dismissed" % busy_label, _l6.SURVEY not in _l6.screen_tail(pane, 20))
    pane.prompt(prompt)
    served = _l5.await_step(world, busy_label, 30)
    world.check("%s: the busy turn was served" % busy_label, served is not None)
    ids = [_l6.send(world, sender, thread, body, *flags) for body, flags in sends]
    world.check("%s: every send succeeded" % busy_label, all(ids), "ids=%s" % ids)
    world.check("%s: the busy turn ended" % busy_label, pane.wait("idle", 40))
    return ids


def no_miss(world, label):
    record = _l6.downgrade_record(world)
    world.check("%s: no injection miss is recorded and the identity is not downgraded" % label,
                record is None or (not record.get("misses") and record.get("downgraded_at") is None),
                "downgrade.json=%s" % record)


def frames_in(request):
    """Message ids framed in the request's ``PreInvocation`` system messages."""
    return _l6.frame_ids_in([m for m in _l6.system_texts(request) if _l6.HEADER.search(m)])


def stop_phase(world, pane, sender, thread, scenario):
    [o2] = busy_turn_send(world, pane, sender, thread, "STOP-BUSY start", "stop-busy",
                          [("STOP-NOTE ordinary", ("--nudge",))])
    pane.prompt("STOP-GO now")
    request = _l5.await_step(world, "stop-inv0", 30)
    world.check("stop: the injecting turn's request was served", request is not None)
    world.check("stop: the item's frame is in that request", o2 in frames_in(request), "ids=%s" % frames_in(request))
    world.check("stop: the agent returned to idle", pane.wait("idle", 30))
    settled = observe.wait_until(lambda: _l6.receipt(world, o2).get("provenance") == HOOK_PROVENANCE, 8)
    world.check("stop: the item injected on the turn's last model call is settled at Stop "
                "(cooperative_hook_delivery)", bool(settled), "receipt=%s" % _l6.receipt(world, o2))
    count = len(observe.main_requests(world))
    time.sleep(10)
    world.check("stop: no idle wake followed", len(observe.main_requests(world)) == count,
                "requests %s -> %s" % (count, len(observe.main_requests(world))))
    world.check("stop: frame header is in the transcript", _l6.frame_in_transcript(_l6.transcript_text(pane), o2))
    world.check("stop: every step consumed", not _l6.unconsumed(world, scenario, ("stop-tail",)),
                "unconsumed=%s" % _l6.unconsumed(world, scenario, ("stop-tail",)))


def cancel_phase(world, pane, sender, thread):
    scenario = load(world, "delivery-cancel")
    busy_turn_send(world, pane, sender, thread, "CANCEL-BUSY start", "cancel-busy",
                   [("CANCEL-NOTE ordinary", ("--nudge",))])
    pane.prompt("CANCEL-GO now")
    request = _l5.await_step(world, "cancel-inv0", 30)
    world.check("cancel: the injecting turn's request was served", request is not None)
    started = time.monotonic()
    pane.keys("Escape")
    world.check("cancel: Esc ended the held turn", pane.wait("idle", 15))
    world.check("cancel: the turn ended well before the 20 s hold", time.monotonic() - started < 15,
                "took %.1fs" % (time.monotonic() - started))
    time.sleep(3)
    no_miss(world, "cancel")
    world.check("cancel: every step consumed", not _l6.unconsumed(world, scenario, ("cancel-tail",)))


def error_phase(world, pane, sender, thread):
    scenario = load(world, "delivery-error")
    busy_turn_send(world, pane, sender, thread, "ERROR-BUSY start", "error-busy",
                   [("ERROR-NOTE ordinary", ("--nudge",))])
    pane.prompt("ERROR-GO now")
    request = _l5.await_step(world, "error-inv0", 30)
    world.check("error: the failing request was served", request is not None)
    world.check("error: the agent returned to idle", pane.wait("idle", 40))
    time.sleep(3)
    world.artifact("error-pane.txt", pane.read_pane(40))
    no_miss(world, "error")
    world.check("error: every step consumed", not _l6.unconsumed(world, scenario, ("error-tail",)))


def main_phase(world, pane, sender, thread, seat):
    scenario = load(world, "delivery")
    ids = busy_turn_send(world, pane, sender, thread, "DELIV-BUSY start", "busy", [
        ("MAIN-L1 lazy note", ("--lazy",)),
        ("MAIN-O1 ordinary note", ("--nudge",)),
        ("MAIN-A1 ack me", ("--require-ack-pane", pane.pane_id)),
        ("MAIN-T1 " + "T" * _l6.OVERSIZE, ("--require-ack-pane", pane.pane_id)),
    ])
    l1, o1, a1, t1 = ids
    pane.prompt("MAIN-GO now")
    request = _l5.await_step(world, "inv0", 30)
    snapshot = {i: _l6.receipt(world, i) for i in (o1, a1, t1)}
    lazy_mid = _l6.lazy_state(world, l1)
    world.check("delivery: inv0 was served", request is not None)
    framed = frames_in(request)
    world.check("delivery: inv0's system messages hold frames for L1, O1, A1 and T1",
                all(i in framed for i in ids), "framed=%s ids=%s" % (framed, ids))
    texts = " ".join(_l6.system_texts(request))
    world.check("delivery: T1's frame is truncated", "truncated; run herdr-threads body %s" % t1 in texts)
    world.check("delivery: nothing is settled during the hold",
                all(r.get("state") == "pending" for r in snapshot.values()) and lazy_mid == "pending",
                "receipts=%s lazy=%s" % (snapshot, lazy_mid))
    blocked = []
    final = observe.wait_until(lambda: (blocked.append(pane.agent_status() == "blocked"), _l5.request_for(world, "inv1"))[1], 40)
    world.check("delivery: the ACK ran under the allow entry with no permission prompt",
                final is not None and not any(blocked), "inv1=%s blocked=%s" % (final is not None, any(blocked)))
    responses = (final or {}).get("function_responses") or []
    world.check("delivery: the request after the ACK carries a run_command functionResponse",
                any(r.get("name") == "run_command" for r in responses), "responses=%s" % responses)
    world.check("delivery: the agent returned to idle", pane.wait("idle", 40))
    observe.wait_until(lambda: _l6.receipt(world, o1).get("state") == "acked", 10)
    r_o1, r_a1, r_t1 = (_l6.receipt(world, i) for i in (o1, a1, t1))
    world.artifact("receipts.json", {"O1": r_o1, "A1": r_a1, "T1": r_t1})
    world.check("delivery: O1 is settled with provenance cooperative_hook_delivery",
                r_o1.get("state") == "acked" and r_o1.get("provenance") == HOOK_PROVENANCE, "O1=%s" % r_o1)
    world.check("delivery: L1 is completed", _l6.lazy_state(world, l1) == "displayed")
    world.check("delivery: A1 is ACKed by the agy seat", r_a1.get("state") == "acked" and r_a1.get("actor") == seat,
                "A1=%s seat=%s" % (r_a1, seat))
    world.check("delivery: T1 is still pending", r_t1.get("state") == "pending", "T1=%s" % r_t1)
    transcript = _l6.transcript_text(pane)
    world.check("delivery: frame headers for L1, O1, A1 are in the transcript",
                all(_l6.frame_in_transcript(transcript, i) for i in (l1, o1, a1)))
    world.check("delivery: every step consumed", not _l6.unconsumed(world, scenario, ("main-tail",)),
                "unconsumed=%s" % _l6.unconsumed(world, scenario, ("main-tail",)))


def run(world):
    setup = _l6.setup(world)
    world.check("setup agy succeeds", setup.returncode == 0, (setup.stderr or setup.stdout).strip()[:200])
    sender = world.sender()
    pane = _l6.start_agy(world)
    stop_scenario = load(world, "delivery-stop")
    _l5.run_turn(world, pane, "DELIV-WARM hello", "warm")
    binding = _l5.await_binding(world, lambda b: True)
    world.check("the agy seat is enrolled", binding is not None)
    if binding is None:
        return
    seat = binding["seat_id"]
    thread = _l6.create_thread(world, sender, "gate-delivery")
    invite, accept = _l6.join(world, sender, pane, thread, seat)
    world.check("the sender invited the agy seat and it accepted", invite.returncode == 0 and accept.returncode == 0,
                "invite=%s accept=%s" % (invite.stderr.strip(), accept.stderr.strip()))
    joined = _l5.await_step(world, "wake-join", 45)
    world.check("the join's idle wake was served (wake-join on 'herdr-threads: wake up')", joined is not None)
    pane.wait("idle", 30)
    stop_phase(world, pane, sender, thread, stop_scenario)
    cancel_phase(world, pane, sender, thread)
    error_phase(world, pane, sender, thread)
    main_phase(world, pane, sender, thread, seat)
    clean, detail = observe.backend_clean(world)
    world.check("no unmatched or template-error request", clean, detail)
    world.artifact("requests.json", world.backend.requests())
    world.artifact("state-files.json", _l6.state_files(world))
    pane.exit()


FLOW = Flow(name="delivery", requires=("ht-poh.5", "ht-poh.3.4"), run=run)
