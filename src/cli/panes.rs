//! `--pane` accepts a Herdr pane ID (`w4:p1`) or a unique pane name.
//!
//! A name is the pane's own label (`herdr pane rename`) or, for a tab that
//! holds exactly one pane, that tab's label. The CLI resolves a name to the
//! pane ID once, from one host snapshot, before it builds any request: the
//! service, journals and seat mappings only ever see pane IDs. Names are a
//! lookup aid, never identity evidence.
//!
//! Precedence ([`NAME_PRECEDENCE`]): an exact pane ID in the host's list, then
//! a unique pane label, then the label of a single-pane tab. The first rule
//! that matches wins, so a pane whose label looks like another pane's ID is
//! still reached by its ID, never by the label.
//!
//! Compatibility: a value shaped like a Herdr pane ID is used as given with no
//! host call. Any other value is first matched as an exact pane ID in the
//! host's pane list; when that list cannot be read the value is passed on
//! unchanged, exactly as before names were accepted, and the service reports
//! what it finds.

use super::commands::{CliAction, MutationSpec, ParsedCli};
use crate::{
    host::observation::PaneName,
    protocol::{ids::HostTargetId, results::ApiError},
};

/// How a person finds a pane ID; part of every name-resolution error.
pub const FIND_PANE_ID_HINT: &str =
    "run `herdr pane current` inside that pane (or `herdr pane list`) to get its pane ID";

/// How a `--pane` name is matched, in order; named by the `not_found` error so
/// a person can see why their name did not resolve. Documented in
/// `docs/agent-usage.md`.
pub const NAME_PRECEDENCE: &str = "names match in this order: an exact pane ID, then a unique pane \
     label (`herdr pane rename`), then the label of a tab that holds exactly one pane";

/// The Herdr 0.9.1 pane ID shape `w<workspace>:p<pane>`. Such a value is used
/// as given, exactly as before names were accepted.
pub fn looks_like_pane_id(value: &str) -> bool {
    let Some((workspace, pane)) = value.split_once(':') else {
        return false;
    };
    let part = |text: &str, prefix: char| {
        text.len() > 1
            && text.starts_with(prefix)
            && text[1..].bytes().all(|b| b.is_ascii_alphanumeric())
    };
    part(workspace, 'w') && part(pane, 'p')
}

fn invalid(detail: String) -> ApiError {
    ApiError::invalid_request(detail)
}

fn ambiguous(value: &str, kind: &str, matches: &[&HostTargetId]) -> ApiError {
    let ids = matches
        .iter()
        .take(8)
        .map(|id| id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    invalid(format!(
        "pane name `{value}` is ambiguous: {count} panes match by {kind} ({ids}); pass the pane \
         ID instead; {FIND_PANE_ID_HINT}",
        count = matches.len(),
    ))
}

/// Resolve one `--pane` value against the host's panes: an exact pane ID
/// wins, then a unique pane label, then the label of a single-pane tab.
/// Ambiguity and absence are refused with how to find the ID.
pub fn resolve_pane_name(value: &str, panes: &[PaneName]) -> Result<HostTargetId, ApiError> {
    if let Some(pane) = panes.iter().find(|pane| pane.target.as_str() == value) {
        return Ok(pane.target.clone());
    }
    let by_label: Vec<&HostTargetId> = panes
        .iter()
        .filter(|pane| pane.label.as_deref() == Some(value))
        .map(|pane| &pane.target)
        .collect();
    match by_label.as_slice() {
        [one] => return Ok((*one).clone()),
        [] => {}
        many => return Err(ambiguous(value, "pane label", many)),
    }
    let by_tab: Vec<&HostTargetId> = panes
        .iter()
        .filter(|pane| pane.tab_label.as_deref() == Some(value))
        .map(|pane| &pane.target)
        .collect();
    match by_tab.as_slice() {
        [one]
            if panes
                .iter()
                .any(|pane| &pane.target == *one && pane.tab_pane_count == 1) =>
        {
            Ok((*one).clone())
        }
        [] => Err(ApiError::not_found(format!(
            "no Herdr pane has the ID or name `{value}` ({NAME_PRECEDENCE}); {FIND_PANE_ID_HINT}"
        ))),
        many => Err(ambiguous(value, "tab label", many)),
    }
}

/// Every `--pane`-style argument of this invocation that is not already a
/// pane ID. Empty for most commands, so no host call is made.
fn pane_arguments(parsed: &mut ParsedCli) -> Vec<&mut HostTargetId> {
    let mut found = Vec::new();
    match &mut parsed.action {
        CliAction::Mutation(
            MutationSpec::Resolve(target)
            | MutationSpec::FreshSeat(target)
            | MutationSpec::Rebind { pane: target, .. }
            | MutationSpec::Replace { pane: target, .. },
        ) => found.push(target),
        CliAction::Launch(request) => found.push(&mut request.target),
        _ => {}
    }
    found.retain(|target| !looks_like_pane_id(target.as_str()));
    found
}

/// Replace pane names with pane IDs in place. `panes` is called at most once,
/// and only when some argument is not ID-shaped; if it fails, arguments are
/// left unchanged (the pre-name behavior).
pub fn resolve_pane_arguments(
    parsed: &mut ParsedCli,
    panes: impl FnOnce() -> Result<Vec<PaneName>, ApiError>,
) -> Result<(), ApiError> {
    let mut arguments = pane_arguments(parsed);
    if arguments.is_empty() {
        return Ok(());
    }
    let Ok(panes) = panes() else {
        return Ok(());
    };
    for target in arguments.iter_mut() {
        **target = resolve_pane_name(target.as_str(), &panes)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/cli/panes_hint.rs"]
mod panes_hint;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::results::ErrorCode;

    fn pane(id: &str, label: Option<&str>, tab: Option<&str>, tab_panes: usize) -> PaneName {
        PaneName {
            target: HostTargetId::new(id),
            label: label.map(str::to_owned),
            tab_label: tab.map(str::to_owned),
            tab_pane_count: tab_panes,
        }
    }

    fn panes() -> Vec<PaneName> {
        vec![
            pane("w1:p1", Some("try-target"), Some("main"), 2),
            pane("w1:p2", Some("dup"), Some("main"), 2),
            pane("w1:p3", Some("dup"), Some("solo"), 1),
            pane("w2:p1", None, Some("lonely"), 1),
        ]
    }

    #[test]
    fn snapshot_pane_and_tab_labels_are_read_from_herdr_0_9_1() {
        let pane = |id: &str, terminal: &str, tab: &str, label: Option<&str>| {
            let mut value = serde_json::json!({"pane_id":id,"terminal_id":terminal,
                "workspace_id":"w4","tab_id":tab,"focused":false,"agent_status":"idle","revision":1});
            if let Some(label) = label {
                value["label"] = serde_json::json!(label);
            }
            value
        };
        let raw = serde_json::json!({"id":"x","result":{"type":"session_snapshot","snapshot":{
            "version":"0.9.1","protocol":22,"agents":[],"workspaces":[],"layouts":[],
            "tabs":[{"tab_id":"w4:t1","label":"main","pane_count":2},
                    {"tab_id":"w4:t2","label":"try","pane_count":1}],
            "panes":[pane("w4:p1","term-1","w4:t1",Some("lead")),
                     pane("w4:p2","term-2","w4:t1",None),
                     pane("w4:p3","term-3","w4:t2",None)]}}})
        .to_string();
        let names = crate::host::observation::normalize_pane_names(&raw).unwrap();
        assert_eq!(resolve_pane_name("lead", &names).unwrap().as_str(), "w4:p1");
        assert_eq!(resolve_pane_name("try", &names).unwrap().as_str(), "w4:p3");
        assert!(
            resolve_pane_name("main", &names)
                .unwrap_err()
                .detail
                .contains("ambiguous")
        );
    }

    #[test]
    fn pane_id_shape_is_recognized_and_names_are_not() {
        for id in ["w4:p1", "w4:pAB", "wB:p1"] {
            assert!(looks_like_pane_id(id), "{id}");
        }
        for name in [
            "try-target",
            "w4",
            "w4:",
            "w4:x1",
            "w:p",
            "w4:p1:x",
            "try:p1",
        ] {
            assert!(!looks_like_pane_id(name), "{name}");
        }
    }

    #[test]
    fn unique_pane_label_resolves_to_its_pane_id() {
        assert_eq!(
            resolve_pane_name("try-target", &panes()).unwrap().as_str(),
            "w1:p1"
        );
        assert_eq!(
            resolve_pane_name("w1:p2", &panes()).unwrap().as_str(),
            "w1:p2"
        );
    }

    #[test]
    fn single_pane_tab_label_resolves_but_a_shared_tab_label_is_ambiguous() {
        assert_eq!(
            resolve_pane_name("lonely", &panes()).unwrap().as_str(),
            "w2:p1"
        );
        assert_eq!(
            resolve_pane_name("solo", &panes()).unwrap().as_str(),
            "w1:p3"
        );
        let error = resolve_pane_name("main", &panes()).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        assert!(error.detail.contains("ambiguous"), "{}", error.detail);
        assert!(
            error.detail.contains("herdr pane current"),
            "{}",
            error.detail
        );
    }

    #[test]
    fn ambiguous_pane_label_names_every_match_and_how_to_find_the_id() {
        let error = resolve_pane_name("dup", &panes()).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        assert!(error.detail.contains("ambiguous"), "{}", error.detail);
        assert!(error.detail.contains("w1:p2") && error.detail.contains("w1:p3"));
        assert!(
            error.detail.contains("herdr pane current"),
            "{}",
            error.detail
        );
    }

    #[test]
    fn missing_name_says_how_to_find_the_pane_id() {
        let error = resolve_pane_name("nope", &panes()).unwrap_err();
        assert_eq!(error.code, ErrorCode::NotFound);
        assert!(error.detail.contains("`nope`"), "{}", error.detail);
        assert!(
            error.detail.contains("herdr pane current"),
            "{}",
            error.detail
        );
    }

    #[test]
    fn only_name_arguments_consult_the_host_and_ids_pass_through() {
        let mut parsed = super::super::commands::parse_argv([
            "herdr-threads",
            "seat",
            "resolve",
            "--pane",
            "w9:p9",
        ])
        .unwrap();
        resolve_pane_arguments(&mut parsed, || panic!("an ID needs no host lookup")).unwrap();
        assert_eq!(
            parsed.action,
            CliAction::Mutation(MutationSpec::Resolve(HostTargetId::new("w9:p9")))
        );
        let mut parsed = super::super::commands::parse_argv([
            "herdr-threads",
            "seat",
            "rebind",
            "seat-1",
            "--pane",
            "try-target",
            "--operator",
        ])
        .unwrap();
        resolve_pane_arguments(&mut parsed, || Ok(panes())).unwrap();
        assert!(matches!(
            parsed.action,
            CliAction::Mutation(MutationSpec::Rebind { ref pane, .. }) if pane.as_str() == "w1:p1"
        ));
        // Without the host's pane list an opaque value passes on unchanged.
        let mut parsed = super::super::commands::parse_argv([
            "herdr-threads",
            "seat",
            "resolve",
            "--pane",
            "other",
        ])
        .unwrap();
        resolve_pane_arguments(&mut parsed, || Err(ApiError::host_unavailable("down"))).unwrap();
        assert_eq!(
            parsed.action,
            CliAction::Mutation(MutationSpec::Resolve(HostTargetId::new("other")))
        );
    }
}
