# Native send grammar handoff

Proposed guidance for the active installer/hand-off owner, routed through the parent.

```text
ht send THREAD (--body TEXT | --file PATH | --stdin)
  [--lazy | --nudge] [--require-ack SEAT...]
  [--require-ack-pane PANE] [--space SPACE] [--tab TAB]
  [--deadline SECONDS] [--relays-user] [--user-intent query|request|rule]
```

Native `send` defaults to lazy delivery. `--lazy` explicitly selects that same mode.
`--nudge` selects ordinary delivery with its existing attention and wake behavior.
Explicit ACK seats or ACK pane recipients automatically select ordinary/nudge mode.
`--lazy` conflicts with `--nudge`, any explicit ACK selector, and any supplied
deadline. A deadline without `--nudge` or an explicit ACK recipient is rejected;
it cannot silently enable attention. Lazy delivery requires daemon capability
`send.lazy_v1`; unsupported daemons are refused before intent recording, without
fallback. Relay and human-intent flags remain independent recorded claims.

Exact new native option help:

```text
--lazy   Passive delivery at the next explicit inbox check (the default); no wake or ACK obligation
--nudge  Enable ordinary attention and wake behavior; explicit ACK recipients imply this mode
--deadline <DEADLINE>  ACK deadline in positive seconds; requires --nudge or explicit ACK recipients
```

Proposed concise guide paragraph:

> Send important, nonurgent announcements with `ht send THREAD --body TEXT`
> (lazy by default; `--lazy` is also accepted). They wait for an existing
> participant's next natural explicit inbox check and create no wake, poke,
> notification, model turn or ACK obligation. Use `--nudge` for ordinary
> attention, or select explicit ACK recipients, which enable nudge automatically.
> Lazy sends cannot specify ACK recipients or deadlines. JSON, machine,
> explicit-seat inbox and history/body/search discovery are read-only; discovering
> a lazy message does not ACK it or adopt peer instructions. Own default text inbox
> completion tracks displayed delivery only, not agreement, authority or attention.

Service-owner sends and compound handoffs remain ordinary and accept neither
new native mode flag. Historical omitted wire/journal modes remain Ordinary;
retry preserves stored mode, complete caller claim, harness and intent scope.

The installer-owned `cli::actor_route::InvocationActor` / `split_actor_argv` and
`journal::classify_original_actor` interfaces are absent from this task's supplied
base. This task introduces no human namespace or guessed classifier. The active
owner must reconcile its real reviewed seam at integration; no shared skill or
installer namespace file is edited here.
