//! Submission validation (spec §6). Deterministic; the first failure rejects
//! with its reason. Rules run in order across the whole submission.
use crate::protocol::summary::{
    ItemBody, ItemStatus, JobBundle, NewStatus, Submission, UserIntent, valid_provenance_label,
};
use crate::summary::fold::{evidence_allowed, status_allowed};
use std::collections::HashMap;

/// What a `target` resolves to: where the item was introduced and its kind.
struct Target<'a> {
    seq: u64,
    body: TargetBody<'a>,
}
enum TargetBody<'a> {
    /// An open entry of the bundle's fold.
    Fold(&'a ItemBody),
    /// A decision proposed by this submission.
    Decision,
    /// An open item proposed by this submission.
    OpenItem,
}
impl Target<'_> {
    fn allows(&self, status: NewStatus) -> bool {
        match &self.body {
            TargetBody::Fold(body) => status_allowed(body, status),
            TargetBody::Decision => status == NewStatus::Superseded,
            TargetBody::OpenItem => matches!(status, NewStatus::Resolved | NewStatus::Superseded),
        }
    }
}

/// Validate a raw submission against its job bundle. `priority_at(seq)` reads
/// ordinary canonical human/relayed priority at `seq`; `text_at(seq)` is that
/// message's text. The worker judges meaning; these checks bound its evidence.
pub fn validate(
    bundle: &JobBundle,
    submission: &serde_json::Value,
    priority_at: &dyn Fn(u64) -> bool,
    text_at: &dyn Fn(u64) -> Option<String>,
) -> Result<Submission, String> {
    // Rule 1: schema, level-allowed fields, budgets, labels, unique refs.
    let parsed = Submission::parse(submission)?;
    if bundle.level > 0
        && (!parsed.new_decisions.is_empty()
            || !parsed.new_open_items.is_empty()
            || !parsed.transitions.is_empty())
    {
        return Err(
            "rollup submissions carry only narrative, prompt_version and model".to_string(),
        );
    }
    let encoded =
        serde_json::to_vec(submission).map_err(|e| format!("unencodable submission: {e}"))?;
    if encoded.len() > bundle.budget_bytes as usize {
        return Err(format!(
            "submission is {} bytes, over the {} byte budget",
            encoded.len(),
            bundle.budget_bytes
        ));
    }
    if parsed.narrative.len() > bundle.narrative_bytes as usize {
        return Err(format!(
            "narrative is {} bytes, over the {} byte limit",
            parsed.narrative.len(),
            bundle.narrative_bytes
        ));
    }
    if !valid_provenance_label(&parsed.prompt_version) {
        return Err("prompt_version must be non-empty and at most 64 bytes".to_string());
    }
    if !valid_provenance_label(&parsed.model) {
        return Err("model must be non-empty and at most 64 bytes".to_string());
    }
    let mut refs: HashMap<&str, Target<'_>> = HashMap::new();
    let proposed = parsed
        .new_decisions
        .iter()
        .map(|d| (d.reference.as_str(), d.seq, TargetBody::Decision))
        .chain(
            parsed
                .new_open_items
                .iter()
                .map(|o| (o.reference.as_str(), o.seq, TargetBody::OpenItem)),
        );
    for (reference, seq, body) in proposed {
        if refs.insert(reference, Target { seq, body }).is_some() {
            return Err(format!("duplicate ref {reference}"));
        }
        if bundle.fold.entries.iter().any(|e| e.item.id == reference) {
            return Err(format!("ref {reference} collides with a fold item id"));
        }
    }

    // Rule 2: every seq and cite_seq inside the job's range.
    let in_range = |seq: u64| seq >= bundle.range.first_seq && seq <= bundle.range.last_seq;
    let out_of_range = |what: &str, seq: u64| {
        format!(
            "{what} {seq} is outside the job range {}..={}",
            bundle.range.first_seq, bundle.range.last_seq
        )
    };
    for d in &parsed.new_decisions {
        if !in_range(d.seq) {
            return Err(out_of_range(
                &format!("decision {} seq", d.reference),
                d.seq,
            ));
        }
    }
    for o in &parsed.new_open_items {
        if !in_range(o.seq) {
            return Err(out_of_range(
                &format!("open item {} seq", o.reference),
                o.seq,
            ));
        }
    }
    for o in &parsed.new_open_items {
        if bundle.fold.entries.iter().any(|e| {
            e.item.seq == o.seq
                && matches!(
                    e.item.body,
                    ItemBody::UserInstruction {
                        user_intent: Some(_),
                        ..
                    }
                )
        }) {
            return Err(format!(
                "open item {} duplicates classified source {}",
                o.reference, o.seq
            ));
        }
    }
    for t in &parsed.transitions {
        if !in_range(t.cite_seq) {
            return Err(out_of_range(
                &format!("transition on {} cite_seq", t.target),
                t.cite_seq,
            ));
        }
    }

    // Rule 3: every quote is an exact substring of the cited message.
    let quotes = parsed
        .new_decisions
        .iter()
        .map(|d| (format!("decision {}", d.reference), d.seq, &d.quote))
        .chain(
            parsed
                .new_open_items
                .iter()
                .map(|o| (format!("open item {}", o.reference), o.seq, &o.quote)),
        )
        .chain(
            parsed
                .transitions
                .iter()
                .map(|t| (format!("transition on {}", t.target), t.cite_seq, &t.quote)),
        );
    for (what, seq, quote) in quotes {
        let Some(quote) = quote else { continue };
        match text_at(seq) {
            Some(text) if text.contains(quote.as_str()) => {}
            Some(_) => return Err(format!("{what}: quote is not in message {seq}")),
            None => return Err(format!("{what}: no message at {seq} to quote")),
        }
    }

    // Rules 4 and 5 resolve each target the same way.
    let resolve = |target: &str| -> Result<(Target<'_>, bool), String> {
        if let Some(proposed) = refs.get(target) {
            return Ok((
                Target {
                    seq: proposed.seq,
                    body: match proposed.body {
                        TargetBody::Decision => TargetBody::Decision,
                        _ => TargetBody::OpenItem,
                    },
                },
                false,
            ));
        }
        match bundle.fold.entries.iter().find(|e| e.item.id == target) {
            Some(e) if matches!(e.status, ItemStatus::Open | ItemStatus::Active) => Ok((
                Target {
                    seq: e.item.seq,
                    body: TargetBody::Fold(&e.item.body),
                },
                matches!(e.item.body, ItemBody::UserInstruction { .. }),
            )),
            Some(_) => Err(format!("transition target {target} is not open")),
            None => Err(format!(
                "transition target {target} is neither an open fold id nor a ref of this submission"
            )),
        }
    };

    // Rule 4: target exists and is open (or a ref), was introduced below its
    // cite_seq, and a superseded instruction is cited by a priority message.
    for t in &parsed.transitions {
        let (target, is_instruction) = resolve(&t.target)?;
        if target.seq >= t.cite_seq {
            return Err(format!(
                "transition target {} was introduced at {}, not below cite_seq {}",
                t.target, target.seq, t.cite_seq
            ));
        }
        if is_instruction && t.new_status == NewStatus::Superseded && !priority_at(t.cite_seq) {
            return Err(format!(
                "instruction {} can be superseded only by a priority message; {} is not one",
                t.target, t.cite_seq
            ));
        }
    }

    // Rule 5: new_status is allowed for the target's kind.
    for t in &parsed.transitions {
        let (target, _) = resolve(&t.target)?;
        if !target.allows(t.new_status) {
            return Err(format!(
                "new_status {:?} is not allowed for target {}",
                t.new_status, t.target
            ));
        }
        match target.body {
            TargetBody::Fold(body) => {
                evidence_allowed(body, t.new_status, t.rule_change, priority_at(t.cite_seq))
                    .map_err(|reason| format!("transition on {}: {reason}", t.target))?;
                if matches!(
                    body,
                    ItemBody::UserInstruction {
                        user_intent: Some(UserIntent::Rule),
                        ..
                    }
                ) && t.quote.as_ref().is_none_or(|q| q.trim().is_empty())
                {
                    return Err(format!(
                        "transition on {}: rule supersession needs a nonempty quote",
                        t.target
                    ));
                }
            }
            _ if t.rule_change.is_some() => {
                return Err(format!(
                    "transition on {}: rule_change is allowed only for a rule",
                    t.target
                ));
            }
            _ => {}
        }
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        ids::{SeatId, SummaryJobId, ThreadId},
        summary::{
            AuthorRole, Fold, FoldDisplay, FoldEntry, LedgerItem, OpenItemKind, SUBMISSION_SCHEMA,
            SeqRange,
        },
    };
    use serde_json::{Value, json};

    fn entry(id: &str, seq: u64, status: ItemStatus, body: ItemBody) -> FoldEntry {
        FoldEntry {
            item: LedgerItem {
                id: id.into(),
                seq,
                body,
            },
            status,
            closed_at_seq: (!matches!(status, ItemStatus::Open | ItemStatus::Active))
                .then_some(seq + 1),
            display: FoldDisplay::Full,
        }
    }
    fn instruction() -> ItemBody {
        ItemBody::UserInstruction {
            author_seat: None,
            author_role: Some(AuthorRole::Human),
            relays_user: false,
            user_intent: None,
            text: Some("do it".into()),
            text_ref: None,
            message_id: None,
        }
    }
    fn open_item_body() -> ItemBody {
        ItemBody::OpenItem {
            kind: OpenItemKind::Question,
            from_seat: SeatId::new("sa"),
            to_seat: None,
            text: "why?".into(),
        }
    }
    fn decision_body() -> ItemBody {
        ItemBody::Decision {
            by_seat: SeatId::new("sa"),
            text: "use sqlite".into(),
        }
    }

    /// Level-0 job over 11..=20 with fold items: open instruction i.3, open
    /// item cv.0.1 (4), active decision cv.0.2 (5), this chunk's prefill i.12,
    /// and a resolved item cv.0.9 (6).
    fn bundle(level: u32) -> JobBundle {
        JobBundle {
            job_id: SummaryJobId::new("j1"),
            thread: ThreadId::new("t1"),
            chunking_version: "cv".into(),
            level,
            index: 1,
            range: SeqRange {
                first_seq: 11,
                last_seq: 20,
            },
            submission_schema: SUBMISSION_SCHEMA,
            budget_bytes: 3072 + if level == 0 { 8192 } else { 1024 },
            narrative_bytes: 3072,
            messages: vec![],
            children: vec![],
            fold: Fold {
                entries: vec![
                    entry("i.3", 3, ItemStatus::Open, instruction()),
                    entry("cv.0.1", 4, ItemStatus::Open, open_item_body()),
                    entry("cv.0.2", 5, ItemStatus::Active, decision_body()),
                    entry("cv.0.9", 6, ItemStatus::Resolved, open_item_body()),
                    entry("i.12", 12, ItemStatus::Open, instruction()),
                ],
                identifiers: vec![],
                rendered_bytes: 0,
            },
            pinned: vec![],
            size_bytes: 0,
            oversized: false,
        }
    }

    fn good() -> Value {
        json!({
            "submission_schema": SUBMISSION_SCHEMA,
            "narrative": "sa decided things",
            "new_decisions": [{"ref": "d1", "seq": 13, "by_seat": "sa", "text": "go", "quote": "hello 13"}],
            "new_open_items": [{"ref": "o1", "seq": 14, "kind": "ask", "from_seat": "sa", "text": "who?"}],
            "transitions": [
                {"target": "i.3", "new_status": "done", "cite_seq": 15, "quote": "hello 15"},
                {"target": "cv.0.1", "new_status": "resolved", "cite_seq": 16},
                {"target": "d1", "new_status": "superseded", "cite_seq": 17},
                {"target": "o1", "new_status": "resolved", "cite_seq": 18},
                {"target": "i.12", "new_status": "done", "cite_seq": 19},
            ],
            "prompt_version": "p1",
            "model": "m1",
        })
    }
    fn text_at(seq: u64) -> Option<String> {
        (11..=20)
            .contains(&seq)
            .then(|| format!("well hello {seq} there"))
    }
    fn priority_at(seq: u64) -> bool {
        seq == 16 || seq == 17
    }
    fn check(bundle: &JobBundle, value: &Value) -> Result<Submission, String> {
        validate(bundle, value, &priority_at, &text_at)
    }
    fn reject(bundle: &JobBundle, value: &Value) -> String {
        check(bundle, value).expect_err("must be rejected")
    }
    fn edit(mutate: impl FnOnce(&mut Value)) -> Value {
        let mut v = good();
        mutate(&mut v);
        v
    }

    #[test]
    fn a_fully_valid_level0_submission_passes() {
        let parsed = check(&bundle(0), &good()).unwrap();
        assert_eq!(parsed.new_decisions.len(), 1);
        assert_eq!(parsed.transitions.len(), 5);
    }

    #[test]
    fn rule1_schema_and_shape() {
        let b = bundle(0);
        assert!(
            reject(
                &b,
                &edit(|v| v["submission_schema"] = json!(SUBMISSION_SCHEMA + 1))
            )
            .contains(&format!(
                "unknown submission_schema {}",
                SUBMISSION_SCHEMA + 1
            ))
        );
        assert!(reject(&b, &json!([1])).contains("not a JSON object"));
        assert!(reject(&b, &edit(|v| v["surprise"] = json!(1))).contains("invalid submission"));
        // Duplicate refs.
        let dup = edit(|v| v["new_open_items"][0]["ref"] = json!("d1"));
        assert_eq!(reject(&b, &dup), "duplicate ref d1");
        // A ref may not shadow a fold id.
        let shadow = edit(|v| v["new_open_items"][0]["ref"] = json!("i.3"));
        assert!(reject(&b, &shadow).contains("collides with a fold item id"));
        // Positive: a minimal submission.
        let minimal = json!({"submission_schema": SUBMISSION_SCHEMA, "narrative": "n", "prompt_version": "p", "model": "m"});
        assert!(check(&b, &minimal).is_ok());
    }

    #[test]
    fn rule1_rollups_carry_only_narrative() {
        let b = bundle(1);
        let narrative_only = json!({"submission_schema": SUBMISSION_SCHEMA, "narrative": "n", "prompt_version": "p", "model": "m"});
        assert!(check(&b, &narrative_only).is_ok());
        let expected = "rollup submissions carry only narrative, prompt_version and model";
        for field in [
            ("new_decisions", good()["new_decisions"].clone()),
            ("new_open_items", good()["new_open_items"].clone()),
            ("transitions", good()["transitions"].clone()),
        ] {
            let mut v = narrative_only.clone();
            v[field.0] = field.1;
            assert_eq!(reject(&b, &v), expected, "{}", field.0);
        }
    }

    #[test]
    fn rule1_budgets() {
        let mut b = bundle(0);
        // Narrative at the limit passes; one byte over fails.
        let at_limit = edit(|v| v["narrative"] = json!("x".repeat(3072)));
        assert!(check(&b, &at_limit).is_ok());
        let over = edit(|v| v["narrative"] = json!("x".repeat(3073)));
        assert!(reject(&b, &over).contains("narrative is 3073 bytes"));
        // Whole encoded submission over budget_bytes.
        b.budget_bytes = serde_json::to_vec(&good()).unwrap().len() as u32;
        assert!(check(&b, &good()).is_ok());
        b.budget_bytes -= 1;
        assert!(reject(&b, &good()).contains("byte budget"));
        // Rollup budget is narrow: a 1 KiB over-the-narrative payload is refused.
        let mut r = bundle(1);
        r.narrative_bytes = 100;
        r.budget_bytes = 200;
        let big_labels = json!({"submission_schema": SUBMISSION_SCHEMA, "narrative": "n", "prompt_version": "p".repeat(64), "model": "m".repeat(64)});
        assert!(check(&r, &big_labels).is_ok());
        r.budget_bytes = 150;
        assert!(reject(&r, &big_labels).contains("byte budget"));
    }

    #[test]
    fn rule1_provenance_labels() {
        let b = bundle(0);
        for field in ["prompt_version", "model"] {
            assert!(check(&b, &edit(|v| v[field] = json!("x".repeat(64)))).is_ok());
            let empty = reject(&b, &edit(|v| v[field] = json!("")));
            assert_eq!(
                empty,
                format!("{field} must be non-empty and at most 64 bytes")
            );
            let long = reject(&b, &edit(|v| v[field] = json!("x".repeat(65))));
            assert_eq!(
                long,
                format!("{field} must be non-empty and at most 64 bytes")
            );
        }
    }

    #[test]
    fn rule2_seqs_inside_the_job_range() {
        let b = bundle(0);
        for bad in [10u64, 21] {
            let v = edit(|v| v["new_decisions"][0]["seq"] = json!(bad));
            assert!(
                reject(&b, &v).contains("outside the job range 11..=20"),
                "{bad}"
            );
        }
        let v = edit(|v| v["new_open_items"][0]["seq"] = json!(25));
        assert!(reject(&b, &v).contains("open item o1 seq 25 is outside"));
        let v = edit(|v| v["transitions"][1]["cite_seq"] = json!(10));
        assert!(reject(&b, &v).contains("cite_seq 10 is outside"));
        // Boundaries are inclusive.
        let v = edit(|v| {
            v["new_decisions"][0]["seq"] = json!(11);
            v["new_decisions"][0]["quote"] = json!("hello 11");
            v["transitions"][4]["cite_seq"] = json!(20);
        });
        assert!(check(&b, &v).is_ok());
    }

    #[test]
    fn rule3_quotes_are_exact_substrings() {
        let b = bundle(0);
        assert!(check(&b, &good()).is_ok());
        let v = edit(|v| v["new_decisions"][0]["quote"] = json!("hello 14"));
        assert!(reject(&b, &v).contains("decision d1: quote is not in message 13"));
        let v = edit(|v| v["transitions"][0]["quote"] = json!("Hello 15"));
        assert!(reject(&b, &v).contains("transition on i.3: quote is not in message 15"));
        let v = edit(|v| v["new_open_items"][0]["quote"] = json!("hello 15"));
        assert!(reject(&b, &v).contains("open item o1: quote is not in message 14"));
        // A cited message the daemon cannot read cannot be quoted.
        let none = validate(&b, &good(), &priority_at, &|_| None).unwrap_err();
        assert!(none.contains("no message at 13 to quote"));
    }

    #[test]
    fn rule4_targets_and_supersession() {
        let b = bundle(0);
        // Unknown target.
        let v = edit(|v| v["transitions"][1]["target"] = json!("nope"));
        assert!(reject(&b, &v).contains("neither an open fold id nor a ref"));
        // A closed fold item is not open.
        let v = edit(|v| v["transitions"][1]["target"] = json!("cv.0.9"));
        assert!(reject(&b, &v).contains("cv.0.9 is not open"));
        // Introduced at or above cite_seq: fold item (this chunk's prefill i.12
        // cited at 12) and a same-submission ref (d1 at 13 cited at 13).
        let v = edit(|v| v["transitions"][4]["cite_seq"] = json!(12));
        assert!(reject(&b, &v).contains("i.12 was introduced at 12, not below cite_seq 12"));
        let v = edit(|v| v["transitions"][2]["cite_seq"] = json!(13));
        assert!(reject(&b, &v).contains("d1 was introduced at 13, not below cite_seq 13"));
        // Same-submission ref introduced one below the cite is fine (good() does it).
        assert!(check(&b, &good()).is_ok());
        // Superseding an instruction needs a priority citing message.
        let v = edit(|v| {
            v["transitions"][0] =
                json!({"target": "i.3", "new_status": "superseded", "cite_seq": 15});
        });
        assert!(reject(&b, &v).contains("only by a priority message; 15 is not one"));
        let v = edit(|v| {
            v["transitions"][0] =
                json!({"target": "i.3", "new_status": "superseded", "cite_seq": 16});
        });
        assert!(check(&b, &v).is_ok());
        // Non-instructions may be superseded by any message.
        let v = edit(|v| {
            v["transitions"][1] =
                json!({"target": "cv.0.1", "new_status": "superseded", "cite_seq": 15});
        });
        assert!(check(&b, &v).is_ok());
    }

    #[test]
    fn rule5_status_allowed_for_kind() {
        let b = bundle(0);
        let transition = |target: &str, status: &str| {
            json!({"submission_schema": SUBMISSION_SCHEMA, "narrative": "n", "prompt_version": "p", "model": "m",
                   "new_open_items": [{"ref": "o1", "seq": 12, "kind": "ask", "from_seat": "sa", "text": "q"}],
                   "new_decisions": [{"ref": "d1", "seq": 12, "by_seat": "sa", "text": "t"}],
                   "transitions": [{"target": target, "new_status": status, "cite_seq": 19}]})
        };
        let cases = [
            // (target, status, allowed)
            ("i.3", "done", true),
            ("i.3", "resolved", false),
            ("cv.0.1", "resolved", true),
            ("cv.0.1", "done", false),
            ("cv.0.2", "superseded", true),
            ("cv.0.2", "done", false),
            ("cv.0.2", "resolved", false),
            ("d1", "superseded", true),
            ("d1", "resolved", false),
            ("o1", "resolved", true),
            ("o1", "done", false),
        ];
        for (target, status, allowed) in cases {
            let result = check(&b, &transition(target, status));
            match (allowed, result) {
                (true, Ok(_)) => {}
                (false, Err(reason)) => {
                    assert!(
                        reason.contains("is not allowed for target"),
                        "{target} {status}: {reason}"
                    )
                }
                (true, Err(reason)) => panic!("{target} {status} refused: {reason}"),
                (false, Ok(_)) => panic!("{target} {status} accepted"),
            }
        }
    }

    #[test]
    fn the_first_failing_rule_is_reported() {
        // Rule 2 (range) and rule 4 (unknown target) both fail; rule 2 wins.
        let v = edit(|v| {
            v["new_decisions"][0]["seq"] = json!(99);
            v["transitions"][1]["target"] = json!("nope");
        });
        assert!(reject(&bundle(0), &v).contains("outside the job range"));
    }
    fn intent_bundle(intent: crate::protocol::summary::UserIntent) -> JobBundle {
        let mut b = bundle(0);
        let e = b
            .fold
            .entries
            .iter_mut()
            .find(|e| e.item.id == "i.12")
            .unwrap();
        if let ItemBody::UserInstruction { user_intent, .. } = &mut e.item.body {
            *user_intent = Some(intent);
        }
        e.status = if intent == crate::protocol::summary::UserIntent::Rule {
            ItemStatus::Active
        } else {
            ItemStatus::Open
        };
        b
    }
    fn intent_transition() -> Value {
        json!({"submission_schema": SUBMISSION_SCHEMA, "narrative": "n", "prompt_version": "p", "model": "m",
            "transitions": [{"target": "i.12", "new_status": "superseded", "cite_seq": 16,
                "rule_change": "withdrawn", "quote": "hello 16"}]})
    }

    #[test]
    fn user_intent_rule_evidence_guards() {
        use crate::protocol::summary::UserIntent::{Query, Request, Rule};
        let b = intent_bundle(Rule);
        let valid = intent_transition();
        for change in ["withdrawn", "replaced"] {
            let mut v = valid.clone();
            v["transitions"][0]["rule_change"] = json!(change);
            assert!(check(&b, &v).is_ok(), "{change}");
        }
        // Quotes must exist, be nonblank and exactly match the cited source.
        for quote in [Value::Null, json!(""), json!(" "), json!("Hello 16")] {
            let mut v = valid.clone();
            v["transitions"][0]["quote"] = quote.clone();
            assert!(check(&b, &v).is_err(), "quote {quote}");
        }
        let mut v = valid.clone();
        v["transitions"][0].as_object_mut().unwrap().remove("quote");
        assert!(check(&b, &v).is_err());
        v = valid.clone();
        v["transitions"][0]
            .as_object_mut()
            .unwrap()
            .remove("rule_change");
        assert!(check(&b, &v).is_err());
        for status in ["resolved", "done"] {
            let mut v = valid.clone();
            v["transitions"][0]["new_status"] = json!(status);
            assert!(check(&b, &v).is_err(), "{status}");
        }
        for cite in [10, 11, 12, 21] {
            let mut v = valid.clone();
            v["transitions"][0]["cite_seq"] = json!(cite);
            v["transitions"][0]["quote"] = json!(format!("hello {cite}"));
            assert!(check(&b, &v).is_err(), "cite {cite}");
        }
        // The callback contract excludes system events independently of role
        // and relay. All sources have the same exact quote, so only recorded
        // source kind and priority determine acceptance.
        use crate::protocol::{results::MessageKind, summary::is_priority};
        for (role, relay, kind, accepted) in [
            (AuthorRole::Human, false, MessageKind::Ordinary, true),
            (AuthorRole::Agent, true, MessageKind::Ordinary, true),
            (AuthorRole::Agent, false, MessageKind::Ordinary, false),
            (AuthorRole::Human, false, MessageKind::Info, false),
            (AuthorRole::Agent, true, MessageKind::Warn, false),
        ] {
            let priority =
                |seq| seq == 16 && kind == MessageKind::Ordinary && is_priority(Some(role), relay);
            assert_eq!(
                validate(&b, &valid, &priority, &text_at).is_ok(),
                accepted,
                "{role:?} relay={relay} {kind:?}"
            );
        }
        for intent in [Query, Request] {
            let b = intent_bundle(intent);
            let mut v = valid.clone();
            v["transitions"][0] =
                json!({"target": "i.12", "new_status": "resolved", "cite_seq": 15});
            assert!(check(&b, &v).is_ok(), "agent answer/completion {intent:?}");
            v["transitions"][0]["rule_change"] = json!("replaced");
            assert!(check(&b, &v).is_err(), "non-rule {intent:?}");
        }
        for (target, status) in [
            ("i.3", "done"),
            ("cv.0.1", "resolved"),
            ("cv.0.2", "superseded"),
        ] {
            let mut v = valid.clone();
            v["transitions"][0] = json!({"target": target, "new_status": status, "cite_seq": 16, "rule_change": "withdrawn"});
            assert!(check(&bundle(0), &v).is_err(), "non-rule {target}");
        }
        for (field, item, status) in [
            (
                "new_decisions",
                json!({"ref": "d", "seq": 13, "by_seat": "sa", "text": "d"}),
                "superseded",
            ),
            (
                "new_open_items",
                json!({"ref": "d", "seq": 13, "kind": "ask", "from_seat": "sa", "text": "q"}),
                "resolved",
            ),
        ] {
            let mut v = valid.clone();
            v[field] = json!([item]);
            v["transitions"][0] = json!({"target": "d", "new_status": status, "cite_seq": 16, "rule_change": "withdrawn"});
            assert!(check(&bundle(0), &v).is_err(), "proposed non-rule {field}");
        }
    }

    #[test]
    fn user_intent_duplicate_classified_open_item_rejected() {
        use crate::protocol::summary::UserIntent::{Query, Request, Rule};
        for intent in [Query, Request, Rule] {
            let b = intent_bundle(intent);
            for kind in ["ask", "question", "commitment", "blocker"] {
                let v = json!({"submission_schema": SUBMISSION_SCHEMA, "narrative": "n", "prompt_version": "p", "model": "m",
                    "new_open_items": [{"ref": "other-id", "seq": 12, "kind": kind, "from_seat": "sa", "text": "duplicate"}]});
                assert!(check(&b, &v).is_err(), "{intent:?} {kind}");
                assert!(check(&bundle(0), &v).is_ok(), "unclassified {kind}");
                let mut agent = v.clone();
                agent["new_open_items"][0]["seq"] = json!(14);
                assert!(check(&b, &agent).is_ok(), "agent discovered {kind}");
            }
        }
    }
}
