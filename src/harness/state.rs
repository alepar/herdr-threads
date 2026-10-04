//! Contract-first state derivation (ht-xoc.5): one pure function turns what
//! this machine and the manifest know about a (harness, version, contract id)
//! into `working`, `new` or `broken`, with the action that fixes a broken one.
//! Health and doctor both render from it; nothing else decides a version
//! verdict.
//!
//! First match wins:
//!
//! 1. a local violation: broken;
//! 2. the version is below the recipe floor: broken (upgrade the harness);
//! 3. the recipe tables report it broken: broken, even when verified here (the
//!    hook's B6 ladder refuses it, so "working" would be false; doctor notes it);
//! 4. local evidence verified it: working (a manifest `known_broken` is a
//!    doctor note only: it has worked here);
//! 5. the manifest reports it broken (same contract): broken;
//! 6. the manifest verified it (same contract): working;
//! 7. a recipe lists it: working;
//! 8. otherwise (optimistic, schema-matched, unlisted and not below the floor):
//!    new, which adds nothing to Health.
//!
//! [`roll_up`] applies the function to every evidence row under the harness's
//! contract id the hooks send now; Health shows the broken version seen most recently
//! within the last 24 hours.
use super::{
    admission::{self, ISSUES_URL, Refusal, Row},
    manifest::{Manifest, ManifestRow, ReleasePointers, RowEvidence, RowSource, RowStatus},
    recipe::{Recipe, Version},
};
use crate::store::harness_evidence::EvidenceRow;

/// Health's window: a version with no session for this long drops out.
pub const HEALTH_WINDOW_MS: u64 = 24 * 60 * 60 * 1000;

/// What the recipe tables say about a version, before any evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ladder {
    /// Unlisted (optimistic or schema-matched) and not below the floor.
    Admitted,
    Listed,
    /// Older than every recipe's minimum.
    BelowFloor {
        min: String,
    },
    /// Inside a recipe's `known_broken` range.
    RecipeKnownBroken {
        range: String,
        newest_working: Option<String>,
    },
}

pub struct StateInput<'a> {
    pub harness: &'static str,
    pub version: &'a str,
    pub contract_id: &'a str,
    pub ladder: Ladder,
    /// The evidence row for this (harness, version, contract id).
    pub local: Option<&'a EvidenceRow>,
    /// `Manifest::status_row`: the same contract only.
    pub manifest_status: Option<&'a ManifestRow>,
    /// `Manifest::other_contract_verified`: the version works under another
    /// contract id (another herdr-threads release).
    pub other_contract_verified: Option<&'a ManifestRow>,
    pub pointers: &'a ReleasePointers,
    /// The newest locally verified version below this one (same harness and
    /// contract id).
    pub newest_verified_here_below: Option<String>,
    /// This herdr-threads release (`env!("CARGO_PKG_VERSION")`).
    pub running_release: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Working(WorkingSource),
    New,
    Broken(Broken),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkingSource {
    Local,
    Manifest {
        source: RowSource,
        evidence: Option<RowEvidence>,
    },
    Recipe,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Broken {
    pub cause: BrokenCause,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrokenCause {
    LocalViolation {
        event: String,
        field: String,
    },
    BelowFloor {
        min: String,
    },
    ManifestKnownBroken {
        event: String,
        field: String,
        source: RowSource,
        issue_url: Option<String>,
    },
    RecipeKnownBroken {
        range: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    UpgradeHerdrThreads {
        to: String,
    },
    Pin {
        max: String,
    },
    Report {
        url: String,
    },
    /// Below the recipe floor: only the harness itself can move.
    UpgradeHarness,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Derived {
    pub state: State,
    /// Doctor-only lines (never Health).
    pub doctor_notes: Vec<String>,
}

pub(crate) fn table_ladder<P>(table: &[Recipe<P>], version: &str) -> Ladder {
    match admission::classify(table, version, || None) {
        Row::Refused(Refusal::OlderThanSupported(_)) => Ladder::BelowFloor {
            min: table
                .iter()
                .map(Recipe::min_version)
                .min()
                .map(|min| min.to_string())
                .unwrap_or_default(),
        },
        Row::Refused(Refusal::KnownBroken {
            range,
            newest_working,
        }) => Ladder::RecipeKnownBroken {
            range: range.to_string(),
            newest_working: newest_working.map(|version| version.to_string()),
        },
        Row::Listed(_) => Ladder::Listed,
        // An unparsable version never reaches here (evidence versions are
        // normalized); an unlisted or newer one is never refused.
        Row::Refused(Refusal::Unparsable) | Row::SchemaMatched(_) | Row::Optimistic { .. } => {
            Ladder::Admitted
        }
    }
}

/// The ladder row of `version` on the recipe tables (no schema fingerprint).
pub fn ladder_for(harness: &str, version: &str) -> Ladder {
    let registry = super::registry::builtins();
    let Some(registration) = registry
        .agent(harness)
        .ok()
        .and_then(|id| registry.by_id(id).ok())
    else {
        return Ladder::Admitted;
    };
    let Ok(identity) =
        super::runtime::RuntimeIdentity::stable_release(version, "native_transcript")
    else {
        return Ladder::Admitted;
    };
    registration.version_ladder(&identity)
}

fn source_word(source: RowSource) -> &'static str {
    match source {
        RowSource::Canary => "canary",
        RowSource::Manual => "manual",
    }
}

fn evidence_word(evidence: RowEvidence) -> &'static str {
    match evidence {
        RowEvidence::Live => "live",
        RowEvidence::NoModel => "no_model",
        RowEvidence::Schema => "schema",
        RowEvidence::Unproven => "none",
    }
}

fn manifest_source(row: &ManifestRow) -> RowSource {
    row.source.unwrap_or(RowSource::Manual)
}

fn select_action(input: &StateInput, recipe_newest_working: Option<&str>) -> Action {
    let running = Version::parse(input.running_release);
    if let Some(row) = input.other_contract_verified
        && let Some(since) = row
            .supported_since
            .as_deref()
            .map(|text| text.trim_start_matches('v'))
        && let (Some(parsed), Some(running)) = (Version::parse(since), running)
        && parsed > running
    {
        return Action::UpgradeHerdrThreads {
            to: parsed.to_string(),
        };
    }
    if let Some(max) = input
        .newest_verified_here_below
        .clone()
        .or_else(|| input.pointers.last_working.clone())
        .or_else(|| recipe_newest_working.map(str::to_owned))
    {
        return Action::Pin { max };
    }
    Action::Report {
        url: input
            .manifest_status
            .and_then(|row| row.issue_url.clone())
            .or_else(|| input.pointers.issue_url.clone())
            .unwrap_or_else(|| ISSUES_URL.to_owned()),
    }
}

/// The state of one (harness, version, contract id).
pub fn derive(input: &StateInput) -> Derived {
    let mut notes = Vec::new();
    let manifest_broken = input
        .manifest_status
        .filter(|row| row.status == Some(RowStatus::KnownBroken));
    let manifest_verified = input
        .manifest_status
        .filter(|row| row.status == Some(RowStatus::Verified));
    let recipe_newest_working = match &input.ladder {
        Ladder::RecipeKnownBroken { newest_working, .. } => newest_working.as_deref(),
        _ => None,
    };
    let broken = |cause| {
        State::Broken(Broken {
            cause,
            action: select_action(input, recipe_newest_working),
        })
    };
    let state = if let Some(local) = input.local.filter(|row| row.violation_at.is_some()) {
        broken(BrokenCause::LocalViolation {
            event: local.violation_event.clone().unwrap_or_default(),
            field: local.violation_field.clone().unwrap_or_default(),
        })
    } else if let Ladder::BelowFloor { min } = &input.ladder {
        State::Broken(Broken {
            cause: BrokenCause::BelowFloor { min: min.clone() },
            action: Action::UpgradeHarness,
        })
    } else if let Ladder::RecipeKnownBroken { range, .. } = &input.ladder {
        if input.local.is_some_and(EvidenceRow::verified) {
            notes.push(format!(
                "it has worked here, but the recipe tables mark {range} known broken and the hook refuses it"
            ));
        }
        broken(BrokenCause::RecipeKnownBroken {
            range: range.clone(),
        })
    } else if input.local.is_some_and(EvidenceRow::verified) {
        if let Some(row) = manifest_broken {
            notes.push(format!(
                "the {} reports a break in {}/{}; it has worked here",
                match manifest_source(row) {
                    RowSource::Canary => "canary",
                    RowSource::Manual => "manual manifest row",
                },
                row.broken_event.as_deref().unwrap_or("unknown"),
                row.broken_field.as_deref().unwrap_or("unknown"),
            ));
        }
        State::Working(WorkingSource::Local)
    } else if let Some(row) = manifest_broken {
        broken(BrokenCause::ManifestKnownBroken {
            event: row.broken_event.clone().unwrap_or_default(),
            field: row.broken_field.clone().unwrap_or_default(),
            source: manifest_source(row),
            issue_url: row.issue_url.clone(),
        })
    } else if let Some(row) = manifest_verified {
        State::Working(WorkingSource::Manifest {
            source: manifest_source(row),
            evidence: row.evidence,
        })
    } else if input.ladder == Ladder::Listed {
        State::Working(WorkingSource::Recipe)
    } else {
        State::New
    };
    Derived {
        state,
        doctor_notes: notes,
    }
}

/// `upgrade herdr-threads to X (supports <h> <v>)`, `pin <h> to <= Y`,
/// `report: <url>` or, below the floor, `upgrade <h>`.
pub fn action_text(harness: &str, version: &str, action: &Action) -> String {
    match action {
        Action::UpgradeHerdrThreads { to } => {
            format!("upgrade herdr-threads to {to} (supports {harness} {version})")
        }
        Action::Pin { max } => format!("pin {harness} to <= {max}"),
        Action::Report { url } => format!("report: {url}"),
        Action::UpgradeHarness => format!("upgrade {harness}"),
    }
}

/// The one line a broken verdict adds to Health and doctor (not yet bounded).
pub fn broken_line(harness: &str, version: &str, broken: &Broken) -> String {
    let action = action_text(harness, version, &broken.action);
    match &broken.cause {
        BrokenCause::LocalViolation { event, field } => format!(
            "harness {harness} {version} broken: {event} payload field {field} is missing or \
             has the wrong type; {action}"
        ),
        BrokenCause::ManifestKnownBroken {
            event,
            field,
            source,
            ..
        } => format!(
            "harness {harness} {version} broken: the {} manifest row reports {} payload field \
             {}; {action}",
            source_word(*source),
            if event.is_empty() { "unknown" } else { event },
            if field.is_empty() { "unknown" } else { field },
        ),
        BrokenCause::RecipeKnownBroken { range } => {
            format!("harness {harness} {version} broken: known broken in {range}; {action}")
        }
        BrokenCause::BelowFloor { min } => {
            format!("{harness} {version} is below the supported floor {min}; {action}")
        }
    }
}

/// `working`, `new` or `broken`.
pub fn state_word(state: &State) -> &'static str {
    match state {
        State::Working(_) => "working",
        State::New => "new",
        State::Broken(_) => "broken",
    }
}

/// Where the verdict came from, for doctor.
pub fn source_text(state: &State) -> String {
    match state {
        State::Working(WorkingSource::Local) => "local evidence (lifecycle + tool payloads)".into(),
        State::Working(WorkingSource::Manifest { source, evidence }) => match source {
            RowSource::Canary => match evidence {
                Some(evidence) => format!("canary manifest row ({})", evidence_word(*evidence)),
                None => "canary manifest row".into(),
            },
            RowSource::Manual => "manual manifest row".into(),
        },
        State::Working(WorkingSource::Recipe) => "recipe tables".into(),
        State::New => "no evidence yet".into(),
        State::Broken(Broken { cause, .. }) => match cause {
            BrokenCause::LocalViolation { event, field } => {
                format!("local evidence: violation in {event}/{field}")
            }
            BrokenCause::BelowFloor { .. } => "below the recipe floor".into(),
            BrokenCause::ManifestKnownBroken { source, .. } => {
                format!("{} manifest row", source_word(*source))
            }
            BrokenCause::RecipeKnownBroken { .. } => "recipe tables".into(),
        },
    }
}

/// The issue URL a verdict carries, when it has one.
pub fn issue_url(state: &State) -> Option<String> {
    match state {
        State::Broken(Broken {
            cause: BrokenCause::ManifestKnownBroken { issue_url, .. },
            action,
        }) => issue_url.clone().or(match action {
            Action::Report { url } => Some(url.clone()),
            _ => None,
        }),
        State::Broken(Broken {
            action: Action::Report { url },
            ..
        }) => Some(url.clone()),
        _ => None,
    }
}

/// One evidence row's verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionVerdict {
    pub version: String,
    pub derived: Derived,
    pub last_seen_at: u64,
    pub in_health_window: bool,
}

/// Every version seen under the contract id the harness's hooks send now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessRollup {
    pub harness: &'static str,
    /// The contract id the hooks send now: the one whose most recent row was
    /// seen last.
    pub contract_id: Option<String>,
    /// Newest `last_seen_at` first.
    pub versions: Vec<VersionVerdict>,
}

impl HarnessRollup {
    /// Health's one line for this harness: the broken version seen most
    /// recently within the window; `working` and `new` add nothing.
    pub fn health_line(&self) -> Option<String> {
        self.versions
            .iter()
            .filter(|verdict| verdict.in_health_window)
            .find_map(|verdict| match &verdict.derived.state {
                State::Broken(broken) => Some(broken_line(self.harness, &verdict.version, broken)),
                _ => None,
            })
    }
}

/// The contract id the hooks send now: the one whose most recent row was seen
/// last (ties: the greater id). After a herdr-threads downgrade the older
/// contract's rows are touched again, so it decides (ht-rlv.2).
pub fn newest_contract(rows: &[EvidenceRow]) -> Option<&str> {
    let mut last: std::collections::BTreeMap<&str, u64> = std::collections::BTreeMap::new();
    for row in rows {
        let entry = last.entry(row.contract_id.as_str()).or_insert(0);
        *entry = (*entry).max(row.last_seen_at);
    }
    last.into_iter()
        .max_by(|(id_a, at_a), (id_b, at_b)| at_a.cmp(at_b).then(id_a.cmp(id_b)))
        .map(|(id, _)| id)
}

fn derive_for(
    harness: &'static str,
    version: &str,
    contract_id: &str,
    local: Option<&EvidenceRow>,
    contract_rows: &[&EvidenceRow],
    manifest: &Manifest,
    running_release: &str,
) -> Derived {
    let pointers = manifest.release_pointers(harness, version, contract_id);
    let this = Version::parse(version);
    let newest_verified_here_below = contract_rows
        .iter()
        .filter(|row| row.verified() && row.violation_at.is_none())
        .filter_map(|row| Version::parse(&row.version))
        .filter(|candidate| this.is_some_and(|this| *candidate < this))
        .max()
        .map(|version| version.to_string());
    derive(&StateInput {
        harness,
        version,
        contract_id,
        ladder: ladder_for(harness, version),
        local,
        manifest_status: manifest.status_row(harness, version, contract_id),
        other_contract_verified: manifest.other_contract_verified(harness, version, contract_id),
        pointers: &pointers,
        newest_verified_here_below,
        running_release,
    })
}

/// Derives every row of `harness` under the contract id its hooks send now. `rows` are
/// all of the harness's evidence rows.
pub fn roll_up(
    harness: &'static str,
    rows: &[EvidenceRow],
    manifest: &Manifest,
    running_release: &str,
    now_ms: u64,
) -> HarnessRollup {
    let Some(contract_id) = newest_contract(rows) else {
        return HarnessRollup {
            harness,
            contract_id: None,
            versions: Vec::new(),
        };
    };
    let contract_rows: Vec<&EvidenceRow> = rows
        .iter()
        .filter(|row| row.harness == harness && row.contract_id == contract_id)
        .collect();
    let since = now_ms.saturating_sub(HEALTH_WINDOW_MS);
    let mut versions: Vec<VersionVerdict> = contract_rows
        .iter()
        .map(|row| VersionVerdict {
            version: row.version.clone(),
            derived: derive_for(
                harness,
                &row.version,
                contract_id,
                Some(row),
                &contract_rows,
                manifest,
                running_release,
            ),
            last_seen_at: row.last_seen_at,
            in_health_window: row.last_seen_at >= since,
        })
        .collect();
    versions.sort_by(|a, b| {
        b.last_seen_at
            .cmp(&a.last_seen_at)
            .then_with(|| a.version.cmp(&b.version))
    });
    HarnessRollup {
        harness,
        contract_id: Some(contract_id.to_owned()),
        versions,
    }
}

/// The verdict for the PATH-detected `version` with no attributed row (doctor
/// only, never Health): derived with no local evidence under the rollup's
/// contract id, or under the running binary's own contract when the harness
/// has no evidence yet.
pub fn derive_detected(
    harness: &'static str,
    version: &str,
    rollup_contract: Option<&str>,
    rows: &[EvidenceRow],
    manifest: &Manifest,
    running_release: &str,
) -> (Derived, String) {
    let own;
    let contract_id = match rollup_contract {
        Some(id) => id,
        None => {
            own = super::contract::contract_for(harness)
                .map(super::contract::contract_id)
                .unwrap_or_default();
            &own
        }
    };
    let contract_rows: Vec<&EvidenceRow> = rows
        .iter()
        .filter(|row| row.harness == harness && row.contract_id == contract_id)
        .collect();
    let derived = derive_for(
        harness,
        version,
        contract_id,
        None,
        &contract_rows,
        manifest,
        running_release,
    );
    (derived, contract_id.to_owned())
}

/// What doctor shows after a detected version's state: the broken line, the
/// new-version phrase, or the source of a working verdict.
pub fn detected_line(harness: &str, version: &str, derived: &Derived) -> String {
    match &derived.state {
        State::Broken(broken) => broken_line(harness, version, broken),
        State::New => "new version, not yet seen working; verified on first use".to_owned(),
        State::Working(_) => source_text(&derived.state),
    }
}

#[cfg(test)]
#[path = "../../tests/harness/state.rs"]
mod tests;

/// Exact-domain derivation shared by negotiated health and future local tooling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeDerived {
    pub state: crate::protocol::results::RuntimeEvidenceState,
    pub source: String,
    pub line: String,
    pub notes: Vec<String>,
    pub issue_url: Option<String>,
}
#[allow(clippy::too_many_arguments)]
pub fn derive_runtime(
    registration: &crate::harness::registry::Registration,
    identity: &super::runtime::RuntimeIdentity,
    domain: &str,
    origin: super::evidence::EvidenceOrigin,
    contract: &str,
    local: Option<&crate::store::harness_evidence::EvidenceRowV2>,
    manifest: &Manifest,
) -> RuntimeDerived {
    use crate::protocol::results::RuntimeEvidenceState as Verdict;
    let verdict = |state, source: String, explanation: String, notes: Vec<String>, issue_url| {
        RuntimeDerived {
            state,
            line: format!(
                "{} {} {domain}: {explanation}",
                registration.metadata().id,
                identity.key
            ),
            source,
            notes,
            issue_url,
        }
    };
    let unavailable = || {
        verdict(
            Verdict::Unavailable,
            "unavailable".into(),
            "current runtime contract descriptor unavailable".into(),
            vec![],
            None,
        )
    };
    if identity.validate().is_err() {
        return unavailable();
    }
    let Some(descriptor) = registration.contracts().iter().find(|d| {
        d.domain_id == domain
            && d.origin == origin
            && d.contract_id_v2().ok().as_deref() == Some(contract)
            && !d.required_milestones.is_empty()
    }) else {
        return unavailable();
    };
    let local = local.filter(|row| {
        row.harness == registration.metadata().id
            && &row.identity == identity
            && row.domain == domain
            && row.origin == origin
            && row.contract_id == contract
    });
    if let Some(row) = local.filter(|row| row.violation_at.is_some()) {
        return verdict(
            Verdict::Broken,
            "local callback violation (model stage unclassified)".into(),
            format!(
                "broken: {}/{} violated this exact contract",
                row.violation_event.as_deref().unwrap_or("unknown"),
                row.violation_field.as_deref().unwrap_or("unknown")
            ),
            vec![],
            Some(ISSUES_URL.into()),
        );
    }
    let ladder = if identity.release().is_some() {
        registration.version_ladder(identity)
    } else {
        Ladder::Admitted
    };
    match &ladder {
        Ladder::BelowFloor { min } => {
            return verdict(
                Verdict::Broken,
                "recipe floor".into(),
                format!(
                    "broken: below supported floor {min}; upgrade {}",
                    registration.metadata().id
                ),
                vec![],
                None,
            );
        }
        Ladder::RecipeKnownBroken {
            range,
            newest_working,
        } => {
            return verdict(
                Verdict::Broken,
                "recipe known broken".into(),
                format!(
                    "broken: recipe range {range}; newest working {}",
                    newest_working.as_deref().unwrap_or("unknown")
                ),
                vec![],
                Some(ISSUES_URL.into()),
            );
        }
        _ => {}
    }
    let known = manifest.runtime_row(registration.metadata().id, identity, descriptor);
    let manifest_source = |row: &super::manifest::RuntimeRow| {
        format!(
            "manifest {} ({})",
            match row.source {
                super::manifest::RuntimeSource::Canary => "canary",
                super::manifest::RuntimeSource::Manual => "manual",
            },
            match row.evidence_stage {
                super::manifest::RuntimeStage::SourceCaptured => "source_captured",
                super::manifest::RuntimeStage::NoModel => "no_model",
                super::manifest::RuntimeStage::Live => "live",
            }
        )
    };
    if local.is_some_and(|row| row.verified(descriptor)) {
        let notes = known
            .filter(|row| row.status == super::manifest::RuntimeStatus::KnownBroken)
            .map(|row| {
                vec![format!(
                    "{} reports a violation; this exact domain worked locally",
                    manifest_source(row)
                )]
            })
            .unwrap_or_default();
        return verdict(
            Verdict::Working,
            "local callback evidence (model stage unclassified)".into(),
            "working from all exact-domain milestones; receipt remains cooperative".into(),
            notes,
            None,
        );
    }
    if let Some(row) = known {
        let source = manifest_source(row);
        return match row.status {
            super::manifest::RuntimeStatus::KnownBroken => verdict(
                Verdict::Broken,
                source,
                format!(
                    "broken: manifest reports {}/{}",
                    row.broken_event.as_deref().unwrap_or("unknown"),
                    row.broken_field.as_deref().unwrap_or("unknown")
                ),
                vec![],
                row.issue_url.clone(),
            ),
            super::manifest::RuntimeStatus::Verified => verdict(
                Verdict::Working,
                source,
                "working under this exact domain; evidence stage does not prove native receipt"
                    .into(),
                vec![],
                row.issue_url.clone(),
            ),
        };
    }
    if ladder == Ladder::Listed {
        verdict(
            Verdict::Working,
            "recipe".into(),
            "working: listed stable release recipe".into(),
            vec![],
            None,
        )
    } else {
        verdict(
            Verdict::New,
            "unverified exact domain".into(),
            "new: exact-domain milestones not complete".into(),
            vec![],
            None,
        )
    }
}
