"""``downgrade``: the forced downgrade to ``userMessage`` and its recovery (test-infra spec D7, D3).

``step_mutator.py`` sits in front of the ``PreInvocation`` hook in the isolated HOME's owned
``hooks.json``. Armed, it turns the hook's ``systemMessage`` step into a step agy rejects (``abort``)
or silently ignores (``ignore``). The flow walks, on one enrolled seat:

1. abort: the turn that injects M1 dies (no model request, an error in the pane), ``Stop`` records the
   downgrade and ``doctor`` reports it for the executable identity;
2. downgraded: M2 arrives as a ``user_requests`` entry (in no ``system_messages``) and settles;
3. ``restage_agy()`` (a new executable identity) and ``--conversation <id>``: M3 is a
   ``system_messages`` entry again;
4. ignored step: one turn whose frame never reaches the transcript is one miss and no downgrade; a
   miss on a second, distinct turn downgrades;
5. ``hooks.json`` is restored byte-identically and ``setup-status`` reports ``current``.
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


def busy_then(world, pane, sender, thread, label, busy_prompt, busy_step, body, go_prompt, arm=None, arm_late=None):
    """A scripted busy turn during which ``body`` is sent (``arm`` arms the mutator once the busy
    request is served, so the busy turn's own empty hook output cannot consume it; ``arm_late`` arms it
    after the busy turn has ended and before ``go_prompt`` is typed, so the go turn's first hook
    invocation is guaranteed to find it armed, ht-poh.46); then the turn
    ``go_prompt`` starts, whose first invocation finds the item pending. Returns the message id."""
    if _l6.dismiss_survey(pane):
        world.check("%s: the feedback survey was dismissed" % label, _l6.SURVEY not in _l6.screen_tail(pane, 20))
    pane.prompt(busy_prompt)
    served = _l5.await_step(world, busy_step, 30)
    world.check("%s: the busy turn was served" % label, served is not None)
    if arm:
        _l6.arm(world, arm)
    message = _l6.send(world, sender, thread, body, "--nudge")
    world.check("%s: the send succeeded" % label, bool(message))
    world.check("%s: the busy turn ended" % label, pane.wait("idle", 40))
    if arm_late:
        _l6.arm(world, arm_late)
    pane.prompt(go_prompt)
    return message


def delivery_downgrade(world):
    """The ``downgrade`` object of ``doctor --json``'s delivery section (``{}`` when none)."""
    return _l6.doctor_delivery(world).get("downgrade") or {}


def abort_phase(world, pane, sender, thread, scenario):
    m1 = busy_then(world, pane, sender, thread, "abort", "DOWN-BUSY1 start", "busy1",
                   "DOWN-M1 abort me", "DOWN-GO1 now", arm="abort")
    fired = observe.wait_until(lambda: _l6.fired(world), 20)
    world.check("abort: the mutator fired", fired is not None)
    world.artifact("abort-fired.json", fired or {})
    world.check("abort: it recorded the original hook output with M1's frame",
                bool(fired) and m1 in fired.get("frame_ids", []) and "systemMessage" in fired.get("original_stdout", ""),
                "fired=%s" % str(fired)[:300])
    record = observe.wait_until(lambda: (_l6.downgrade_record(world) or {}).get("downgraded_at"), 25)
    world.check("abort: Stop recorded a downgrade", bool(record), "downgrade.json=%s" % _l6.downgrade_record(world))
    time.sleep(3)
    asked = [r.get("seq") for r in observe.main_requests(world) if (r.get("user_requests") or [""])[-1].startswith("DOWN-GO1")]
    world.check("abort: no main request was made for the aborted turn", not asked, "requests=%s" % asked)
    screen = pane.read_pane(60)
    world.artifact("abort-pane.txt", screen)
    world.check("abort: the pane shows the injection error",
                any(word in screen.lower() for word in ("inject", "hook", "error")), screen[-300:])
    down = observe.wait_until(lambda: delivery_downgrade(world), 10)
    world.artifact("doctor-delivery-abort.json", _l6.doctor_delivery(world))
    world.check("abort: doctor reports the downgrade (abort) for the identity",
                (down or {}).get("reason") == "abort" and (down or {}).get("identity_current") is True, "downgrade=%s" % down)
    return m1


def user_message_phase(world, pane, sender, thread, scenario):
    m2 = busy_then(world, pane, sender, thread, "downgraded", "DOWN-BUSY2 start", "busy2",
                   "DOWN-M2 user message", "DOWN-GO2 now")
    request = _l5.await_step(world, "m2-inv0", 40)
    world.check("downgraded: M2's turn request was served", request is not None)
    users = (request or {}).get("user_requests", [])
    systems = _l6.system_texts(request)
    world.check("downgraded: M2's frame is inside a user_requests entry",
                any(m2 in u and "[herdr-threads]" in u for u in users), "user_requests=%s" % [u[:80] for u in users])
    world.check("downgraded: M2's frame is in no system_messages entry", not any(m2 in s for s in systems))
    world.check("downgraded: the agent returned to idle", pane.wait("idle", 30))
    settled = observe.wait_until(lambda: _l6.receipt(world, m2).get("provenance") == HOOK_PROVENANCE, 15)
    world.check("downgraded: M2 settles (cooperative_hook_delivery)", bool(settled), "M2=%s" % _l6.receipt(world, m2))
    return m2


def restage_phase(world, pane, sender, thread, conversation):
    pane.exit()
    world.restage_agy()
    pane = _l6.start_agy(world, args=["--conversation", conversation], pane=pane.pane_id)
    down = delivery_downgrade(world)
    world.artifact("doctor-delivery-restaged.json", _l6.doctor_delivery(world))
    world.check("restaged: the recorded downgrade no longer applies to the new executable identity",
                not down or down.get("identity_current") is False, "downgrade=%s" % down)
    m3 = busy_then(world, pane, sender, thread, "restaged", "DOWN-BUSY3 start", "busy3",
                   "DOWN-M3 system message", "DOWN-GO3 now")
    request = _l5.await_step(world, "m3-inv0", 40)
    world.check("restaged: M3's turn request was served", request is not None)
    world.check("restaged: M3 is in system_messages again",
                any(m3 in s for s in _l6.system_texts(request)), "system=%s" % [s[:60] for s in _l6.system_texts(request)])
    world.check("restaged: M3's frame is in no user_requests entry",
                not any(m3 in u for u in (request or {}).get("user_requests", [])))
    world.check("restaged: the agent returned to idle", pane.wait("idle", 30))
    settled = observe.wait_until(lambda: _l6.receipt(world, m3).get("provenance") == HOOK_PROVENANCE, 15)
    world.check("restaged: M3 settles (cooperative_hook_delivery)", bool(settled), "M3=%s" % _l6.receipt(world, m3))
    return pane, m3


def ignore_phase(world, pane, sender, thread):
    scenario = load(world, "downgrade-ignore")
    m4 = busy_then(world, pane, sender, thread, "ignored-1", "DOWN-IBUSY1 start", "ign-busy1",
                   "DOWN-M4 ignored once", "DOWN-IGO1 now", arm="ignore")
    fired = observe.wait_until(lambda: _l6.fired(world), 20)
    world.check("ignored-1: the mutator fired in ignore mode", bool(fired) and fired.get("mode") == "ignore"
                and m4 in fired.get("frame_ids", []), "fired=%s" % str(fired)[:300])
    request = _l5.await_step(world, "ign-go1", 30)
    world.check("ignored-1: the turn was served normally (agy ignored the step, no Stop error)", request is not None)
    world.check("ignored-1: M4 is in no system_messages entry", not any(m4 in s for s in _l6.system_texts(request)))
    world.check("ignored-1: the agent returned to idle", pane.wait("idle", 30))
    time.sleep(3)
    record = _l6.downgrade_record(world) or {}
    world.check("ignored-1: one miss is recorded and nothing is downgraded",
                len(record.get("misses", [])) == 1 and record.get("downgraded_at") is None, "downgrade.json=%s" % record)
    world.check("ignored-1: M4's frame is absent from the transcript",
                not _l6.frame_in_transcript(_l6.transcript_text(pane), m4))
    world.check("ignored-1: doctor reports no downgrade", not delivery_downgrade(world))
    # The second busy turn runs unarmed; the mutator is armed between that turn's end and IGO2, so the miss comes from IGO2.
    m5 = busy_then(world, pane, sender, thread, "ignored-2", "DOWN-IBUSY2 start", "ign-busy2",
                   "DOWN-M5 ignored twice", "DOWN-IGO2 now", arm_late="ignore")
    request = _l5.await_step(world, "ign-go2", 40)
    world.check("ignored-2: the second turn was served", request is not None)
    world.check("ignored-2: the agent returned to idle", pane.wait("idle", 30))
    downgraded = observe.wait_until(lambda: (_l6.downgrade_record(world) or {}).get("downgraded_at"), 15)
    record = _l6.downgrade_record(world) or {}
    world.check("ignored-2: misses on two distinct turns downgrade (misses)",
                bool(downgraded) and record.get("reason") == "misses", "downgrade.json=%s" % record)
    down = delivery_downgrade(world)
    world.check("ignored-2: doctor reports the downgrade (misses)", down.get("reason") == "misses", "downgrade=%s" % down)
    world.check("ignored-2: M5's frame is absent from the transcript",
                not _l6.frame_in_transcript(_l6.transcript_text(pane), m5))
    world.check("ignored: every step consumed", not _l6.unconsumed(world, scenario, ("ign-tail",)),
                "unconsumed=%s" % _l6.unconsumed(world, scenario, ("ign-tail",)))


def run(world):
    scenario = load(world, "downgrade")
    setup = _l6.setup(world)
    world.check("setup agy succeeds", setup.returncode == 0, (setup.stderr or setup.stdout).strip()[:200])
    sender = world.sender()
    original = _l6.install_mutator(world)
    world.check("the mutator prefixes the PreInvocation command",
                "step_mutator.py" in _l6.hooks_path(world).read_text() and _l6.hooks_path(world).read_bytes() != original)
    pane = _l6.start_agy(world)
    _l5.run_turn(world, pane, "DOWN-WARM hello", "warm")
    binding = _l5.await_binding(world, lambda b: True)
    world.check("the agy seat is enrolled", binding is not None)
    if binding is None:
        return
    seat, conversation = binding["seat_id"], binding["native_session"]
    thread = _l6.create_thread(world, sender, "gate-downgrade")
    # The idle wake exists only while the invitation is pending: an accept that lands first removes the
    # reason to wake (ht-poh.46), so the wake is awaited between the invite and the accept.
    invite = world.ht(["invite", thread, "--seat", seat], pane=sender.pane_id, check=False, human=True)
    joined = _l5.await_step(world, "wake-join", 45)
    world.check("the join's idle wake was served (wake-join on 'herdr-threads: wake up')", joined is not None)
    pane.wait("idle", 30)
    accept = world.ht(["accept", thread], pane=pane.pane_id, check=False)
    world.check("the sender invited the agy seat and it accepted", invite.returncode == 0 and accept.returncode == 0,
                "invite=%s accept=%s" % (invite.stderr.strip(), accept.stderr.strip()))

    abort_phase(world, pane, sender, thread, scenario)
    user_message_phase(world, pane, sender, thread, scenario)
    pane, _ = restage_phase(world, pane, sender, thread, conversation)
    world.check("downgrade: every step consumed", not _l6.unconsumed(world, scenario, ("tail",)),
                "unconsumed=%s" % _l6.unconsumed(world, scenario, ("tail",)))
    ignore_phase(world, pane, sender, thread)

    pane.exit()
    status = _l4.setup_status(world)
    world.artifact("setup-status-mutated.json", status)
    _l6.restore_hooks(world, original)
    world.check("hooks.json is restored byte-identically", _l6.hooks_path(world).read_bytes() == original)
    status = _l4.setup_status(world)
    world.artifact("setup-status-restored.json", status)
    world.check("setup-status reports current after the restore", status.get("plugin", {}).get("current") is True,
                status.get("plugin"))
    clean, detail = observe.backend_clean(world)
    world.check("no unmatched or template-error request", clean, detail)
    world.artifact("requests.json", world.backend.requests())
    world.artifact("state-files.json", _l6.state_files(world))


FLOW = Flow(name="downgrade", requires=("ht-poh.5", "ht-poh.4.7"), run=run)
