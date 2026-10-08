//! Cooperative invocation routing, independent of output presentation.
use crate::protocol::results::ApiError;
use std::ffi::{OsStr, OsString};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InvocationActor {
    #[default]
    Agent,
    Human,
}

pub fn split_actor_argv(argv: &[String]) -> Result<(InvocationActor, Vec<String>), ApiError> {
    let (actor, retained) = split_actor_os_argv(argv.iter().map(OsString::from).collect());
    Ok((
        actor,
        retained
            .into_iter()
            .map(|s| s.into_string().expect("UTF-8 input"))
            .collect(),
    ))
}

pub(crate) fn split_actor_os_argv(mut argv: Vec<OsString>) -> (InvocationActor, Vec<OsString>) {
    if argv
        .get(1)
        .is_some_and(|value| value == OsStr::new("human"))
    {
        argv.remove(1);
        (InvocationActor::Human, argv)
    } else {
        (InvocationActor::Agent, argv)
    }
}

/// Only walk recognized leading globals. Values (including `human`) are data.
pub(crate) fn command_index(argv: &[OsString]) -> Option<usize> {
    let mut index = 1;
    while let Some(token) = argv.get(index).and_then(|s| s.to_str()) {
        match token {
            "--json" | "--machine" | "--human" => index += 1,
            "--state-dir"
            | "--host-endpoint"
            | "--cooperative-seat"
            | "--cooperative-target"
            | "--cooperative-harness"
            | "--cooperative-role" => index += 2,
            _ if [
                "--state-dir=",
                "--host-endpoint=",
                "--cooperative-seat=",
                "--cooperative-target=",
                "--cooperative-harness=",
                "--cooperative-role=",
            ]
            .iter()
            .any(|flag| token.starts_with(flag)) =>
            {
                index += 1
            }
            _ => return Some(index),
        }
    }
    None
}

pub(crate) fn guidance(argv: &[OsString], displaced: Option<usize>) -> String {
    let mut replacement = argv.to_vec();
    if let Some(index) = displaced {
        replacement.remove(index);
    }
    replacement.insert(1.min(replacement.len()), OsString::from("human"));
    let tokens: Option<Vec<_>> = replacement.iter().map(|s| s.to_str()).collect();
    match tokens {
        Some(tokens) => format!("person/operator actions require immediate human namespace; use {}", tokens.into_iter().map(|s| shlex::try_quote(s).map(|s| s.into_owned()).unwrap_or_else(|_| "<unrepresentable token>".into())).collect::<Vec<_>>().join(" ")),
        None => "person/operator actions require human immediately after the original executable; retain all other original arguments (opaque argv cannot be rendered as UTF-8)".into(),
    }
}
