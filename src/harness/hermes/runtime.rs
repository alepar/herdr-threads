//! Read-only launcher metadata and strict actual-interpreter helper results.
//! Machine argv alone never admits an installed runtime or profile.
use crate::harness::runtime::{RuntimeDescriptor, RuntimeIdentity};
use serde::Deserialize;
use std::{
    io::Read,
    os::fd::AsRawFd,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

pub const QUALIFICATION_GAP: &str = "native_read_only_boundary_unavailable";

#[derive(Debug, PartialEq, Eq)]
pub enum ProbeFailure {
    Malformed,
    UnsupportedShape,
    Unavailable(String),
    Deadline,
    Cancelled,
    OutputLimit,
    ChildFailed,
}

#[derive(Debug)]
pub struct UnsupportedInstalledRuntime {
    /// Positive machine metadata is not effective-config/API qualification.
    pub machine_metadata: MachineCommand,
    pub reason: &'static str,
}

/// Legacy native admission remains unavailable. The separate profile observer
/// can inspect assets, but supplies no callback/native acceptance qualification.
pub fn qualify_installed() -> Result<RuntimeMetadata, ProbeFailure> {
    Err(ProbeFailure::Unavailable(QUALIFICATION_GAP.into()))
}

/// Capture only the producer's machine argv. It is never executed, parsed as
/// shell/bootstrap source, or upgraded to native runtime/config qualification.
pub fn capture_machine_metadata(
    launcher: &Path,
    helper: &Path,
    profile: &str,
    env: &crate::harness::adapter::SetupEnvironment,
    budget: &crate::protocol::time::CallBudget,
) -> Result<UnsupportedInstalledRuntime, ProbeFailure> {
    let started = Instant::now();
    check_budget(env.clock.as_ref(), budget, started)?;
    if !absolute(launcher)
        || !absolute(helper)
        || !safe(profile, 256)
        || !env.cwd.is_absolute()
        || env
            .home
            .as_ref()
            .is_none_or(|v| !Path::new(v).is_absolute())
    {
        return Err(ProbeFailure::Malformed);
    }
    let mut command = Command::new(launcher);
    command
        .args([
            "--print-runtime-command",
            "--module",
            "trace",
            "--",
            "--count",
            "--no-report",
        ])
        .arg(helper)
        .args(["--profile", profile])
        .env_clear()
        .envs(&env.declared)
        .env("HOME", env.home.as_ref().unwrap())
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .current_dir(&env.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if let Some(path) = &env.path {
        command.env("PATH", path);
    }
    #[cfg(feature = "test-support")]
    crate::test_support::spawn::tag(&mut command);
    let bytes = run_capture(command, env.clock.as_ref(), budget, started)?;
    let machine_metadata = decode_machine_command(&bytes, helper, profile)?;
    check_budget(env.clock.as_ref(), budget, started)?;
    Ok(UnsupportedInstalledRuntime {
        machine_metadata,
        reason: "native_bootstrap_read_only_unqualified",
    })
}

struct OwnedProbe(Child);
impl Drop for OwnedProbe {
    fn drop(&mut self) {
        // This process group was created exclusively for our child. Reap the
        // direct child and stop descendants retaining either captured pipe.
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGKILL);
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn check_budget(
    clock: &dyn crate::protocol::time::Clock,
    budget: &crate::protocol::time::CallBudget,
    started: Instant,
) -> Result<(), ProbeFailure> {
    if budget.cancellation.is_cancelled() {
        return Err(ProbeFailure::Cancelled);
    }
    if budget.deadline_passed(clock) || started.elapsed() >= Duration::from_secs(2) {
        return Err(ProbeFailure::Deadline);
    }
    Ok(())
}
fn nonblocking(pipe: &impl AsRawFd) -> Result<(), ProbeFailure> {
    let fd = pipe.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(ProbeFailure::ChildFailed);
    }
    Ok(())
}
fn drain(pipe: &mut impl Read, bytes: &mut Vec<u8>) -> Result<bool, ProbeFailure> {
    let mut buf = [0_u8; 4096];
    // One chunk per turn: a continuously flooding stream cannot starve the
    // other pipe or the shared cancellation/deadline check.
    match pipe.read(&mut buf) {
        Ok(0) => Ok(true),
        Ok(n) => {
            if bytes.len() + n > 16384 {
                return Err(ProbeFailure::OutputLimit);
            }
            bytes.extend_from_slice(&buf[..n]);
            Ok(false)
        }
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
            ) =>
        {
            Ok(false)
        }
        Err(_) => Err(ProbeFailure::ChildFailed),
    }
}
fn run_capture(
    mut command: Command,
    clock: &dyn crate::protocol::time::Clock,
    budget: &crate::protocol::time::CallBudget,
    started: Instant,
) -> Result<Vec<u8>, ProbeFailure> {
    check_budget(clock, budget, started)?;
    let mut child = OwnedProbe(command.spawn().map_err(|_| ProbeFailure::ChildFailed)?);
    let mut stdout = child.0.stdout.take().ok_or(ProbeFailure::ChildFailed)?;
    let mut stderr = child.0.stderr.take().ok_or(ProbeFailure::ChildFailed)?;
    nonblocking(&stdout)?;
    nonblocking(&stderr)?;
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let (mut out_eof, mut err_eof) = (false, false);
    loop {
        check_budget(clock, budget, started)?;
        if !out_eof {
            out_eof = drain(&mut stdout, &mut out)?;
        }
        if !err_eof {
            err_eof = drain(&mut stderr, &mut err)?;
        }
        let status = child.0.try_wait().map_err(|_| ProbeFailure::ChildFailed)?;
        if out_eof
            && err_eof
            && let Some(status) = status
        {
            check_budget(clock, budget, started)?;
            return if status.success() {
                Ok(out)
            } else {
                Err(ProbeFailure::ChildFailed)
            };
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[derive(Debug)]
pub struct MachineCommand {
    pub interpreter: PathBuf,
    pub argv: Vec<String>,
}

#[derive(Debug)]
pub struct MetadataScope {
    pub interpreter: PathBuf,
    pub source_root: PathBuf,
    pub profile: String,
    pub home: PathBuf,
}

#[derive(Debug)]
pub struct RuntimeMetadata {
    pub identity: RuntimeIdentity,
    pub profile: String,
    pub home: PathBuf,
    pub physical_home: PathBuf,
    pub callback_timeout_ms: Option<u64>,
    pub enabled: Vec<String>,
    pub disabled: Vec<String>,
}

impl RuntimeMetadata {
    /// Legacy metadata cannot grant native callback admission.
    pub fn qualification_gap(&self) -> &'static str {
        QUALIFICATION_GAP
    }
    /// Declared metadata only; not effective native-config qualification.
    pub fn declared_plugin_enabled(&self) -> bool {
        self.enabled.iter().any(|v| v == "herdr-threads")
            && !self.disabled.iter().any(|v| v == "herdr-threads")
    }
}

fn safe(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}
fn absolute(value: &Path) -> bool {
    value.is_absolute() && value.to_str().is_some_and(|v| safe(v, 4096))
}
pub fn decode_machine_command(
    bytes: &[u8],
    helper: &Path,
    profile: &str,
) -> Result<MachineCommand, ProbeFailure> {
    if bytes.len() > 16384 || !absolute(helper) || !safe(profile, 256) {
        return Err(ProbeFailure::Malformed);
    }
    let argv: Vec<String> = serde_json::from_slice(bytes).map_err(|_| ProbeFailure::Malformed)?;
    if argv.len() != 9
        || !argv.iter().all(|v| safe(v, 8192))
        || argv[1] != "-I"
        || argv[2] != "-c"
        || argv[4] != "--count"
        || argv[5] != "--no-report"
        || Path::new(&argv[6]) != helper
        || argv[7] != "--profile"
        || argv[8] != profile
        || !absolute(Path::new(&argv[0]))
    {
        return Err(ProbeFailure::UnsupportedShape);
    }
    let interpreter = PathBuf::from(&argv[0]);
    Ok(MachineCommand { interpreter, argv })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApiFacts {
    register_hook: bool,
    invoke_hook: bool,
    callbacks: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HelperResult {
    schema_version: u32,
    status: String,
    reason: Option<String>,
    runtime_descriptor: Option<RuntimeDescriptor>,
    profile: Option<String>,
    home: Option<PathBuf>,
    physical_home: Option<PathBuf>,
    interpreter: Option<PathBuf>,
    source_root: Option<PathBuf>,
    callback_timeout_ms: Option<u64>,
    enabled: Option<Vec<String>>,
    disabled: Option<Vec<String>>,
    api: Option<ApiFacts>,
    evidence_stage: String,
}
/// Parse bounded metadata against an explicit source/profile scope. Neither
/// parser success nor declared API facts qualify a native runtime or timeout.
pub fn decode_runtime_helper(
    bytes: &[u8],
    expected: &MetadataScope,
) -> Result<RuntimeMetadata, ProbeFailure> {
    if bytes.len() > 16384 {
        return Err(ProbeFailure::Malformed);
    }
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| ProbeFailure::Malformed)?;
    let fields = [
        "schema_version",
        "status",
        "reason",
        "runtime_descriptor",
        "profile",
        "home",
        "physical_home",
        "interpreter",
        "source_root",
        "enabled",
        "disabled",
        "callback_timeout_ms",
        "api",
        "evidence_stage",
    ];
    if !value.as_object().is_some_and(|obj| {
        obj.len() == fields.len() && fields.iter().all(|name| obj.contains_key(*name))
    }) {
        return Err(ProbeFailure::Malformed);
    }
    let result: HelperResult =
        serde_json::from_slice(bytes).map_err(|_| ProbeFailure::Malformed)?;
    if result.schema_version != 1
        || !matches!(
            result.evidence_stage.as_str(),
            "model_free_plumbing" | "unavailable"
        )
    {
        return Err(ProbeFailure::Malformed);
    }
    if result.status == "unsupported" {
        if result.evidence_stage != "unavailable" {
            return Err(ProbeFailure::Malformed);
        }
        let reason = result
            .reason
            .filter(|s| safe(s, 128))
            .ok_or(ProbeFailure::Malformed)?;
        if fields[3..13].iter().any(|name| !value[*name].is_null()) {
            return Err(ProbeFailure::Malformed);
        }
        return Err(ProbeFailure::Unavailable(reason));
    }
    if result.status != "metadata_only"
        || result.evidence_stage != "model_free_plumbing"
        || result.reason.is_some()
    {
        return Err(ProbeFailure::Malformed);
    }
    let malformed = || ProbeFailure::Malformed;
    let descriptor = result.runtime_descriptor.ok_or_else(malformed)?;
    let descriptor_fields = [
        "release_version",
        "source",
        "base_version",
        "derived_version",
        "commit",
        "dirty",
        "distance",
    ];
    if !value["runtime_descriptor"].as_object().is_some_and(|obj| {
        obj.len() == descriptor_fields.len()
            && descriptor_fields.iter().all(|name| obj.contains_key(*name))
    }) || (descriptor.source == "git"
        && (descriptor.commit.is_none() || descriptor.distance.is_none()))
    {
        return Err(ProbeFailure::Malformed);
    }
    let profile = result.profile.ok_or_else(malformed)?;
    let home = result.home.ok_or_else(malformed)?;
    let physical_home = result.physical_home.ok_or_else(malformed)?;
    let interpreter = result.interpreter.ok_or_else(malformed)?;
    let source_root = result.source_root.ok_or_else(malformed)?;
    let callback_timeout_ms = result.callback_timeout_ms;
    let enabled = result.enabled.ok_or_else(malformed)?;
    let disabled = result.disabled.ok_or_else(malformed)?;
    let api = result.api.ok_or_else(malformed)?;
    if interpreter != expected.interpreter
        || source_root != expected.source_root
        || profile != expected.profile
        || home != expected.home
        || [&home, &physical_home, &interpreter, &source_root]
            .iter()
            .any(|v| !absolute(v))
        || !safe(&profile, 256)
        || descriptor.release_version.is_some()
        || descriptor.source == "unknown"
        || descriptor.base_version.as_deref() == Some("unknown")
        || descriptor.base_version.is_none()
        || descriptor.derived_version.is_none()
        || descriptor.dirty.is_none()
        || callback_timeout_ms.is_some()
        || !api.register_hook
        || !api.invoke_hook
        || api.callbacks
            != [
                "pre_llm_call",
                "post_tool_call",
                "on_session_start",
                "on_session_reset",
            ]
        || [&enabled, &disabled]
            .iter()
            .any(|names| names.len() > 128 || names.iter().any(|v| !safe(v, 256)))
    {
        return Err(ProbeFailure::Malformed);
    }
    Ok(RuntimeMetadata {
        identity: RuntimeIdentity::build(descriptor).map_err(|_| ProbeFailure::Malformed)?,
        profile,
        home,
        physical_home,
        callback_timeout_ms,
        enabled,
        disabled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn context() -> MetadataScope {
        MetadataScope {
            interpreter: "/fixture/store python/bin/python3".into(),
            source_root: "/fixture/native source".into(),
            profile: "default".into(),
            home: "/fixture/custom home".into(),
        }
    }

    #[test]
    fn hermes_runtime_helper_uses_captured_launcher_profile_and_exact_source() {
        // Removing exact source/build attribution must not credit bare release.
        let got = decode_runtime_helper(
            include_bytes!("../../../tests/fixtures/hermes/runtime-api.json"),
            &context(),
        )
        .unwrap();
        assert!(got.identity.key.starts_with("build:"));
        assert_eq!(
            got.identity.derived_version.as_deref(),
            Some("0.21.5+3962.g37daf85")
        );
        assert_eq!(
            got.identity.commit.as_deref(),
            Some("37daf85b2ad0ee50ed45d7234dc47b7fa24cec09")
        );
        assert_eq!(got.home, Path::new("/fixture/custom home"));
        assert_eq!(got.callback_timeout_ms, None);
        assert!(qualify_installed().is_err());
        assert!(got.declared_plugin_enabled());
    }

    #[test]
    fn captured_argv_retains_spaces_without_shell_or_bootstrap_parsing() {
        let got = decode_machine_command(
            include_bytes!("../../../tests/fixtures/hermes/runtime-command.json"),
            Path::new("/fixture/owned helper/runtime_helper.py"),
            "default",
        )
        .unwrap();
        assert_eq!(
            got.interpreter,
            Path::new("/fixture/store python/bin/python3")
        );
        assert_eq!(got.argv[6], "/fixture/owned helper/runtime_helper.py");
    }

    #[test]
    fn helper_rejects_other_running_interpreter_source_scope_and_incomplete_api() {
        let baseline: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../tests/fixtures/hermes/runtime-api.json"
        ))
        .unwrap();
        for (field, value) in [
            ("interpreter", serde_json::json!("/other/python")),
            ("source_root", serde_json::json!("/other/native")),
            ("profile", serde_json::json!("sticky")),
            ("home", serde_json::json!("/other/home")),
            ("schema_version", serde_json::json!(2)),
            ("evidence_stage", serde_json::json!("live_model")),
            ("callback_timeout_ms", serde_json::json!(100)),
            (
                "api",
                serde_json::json!({"register_hook":false,"invoke_hook":true,"callbacks":[]}),
            ),
        ] {
            let mut changed = baseline.clone();
            changed[field] = value;
            assert!(
                decode_runtime_helper(&serde_json::to_vec(&changed).unwrap(), &context()).is_err(),
                "accepted {field}"
            );
        }
    }

    #[test]
    fn helper_requires_complete_schema_and_byte_caps() {
        let baseline: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../tests/fixtures/hermes/runtime-api.json"
        ))
        .unwrap();
        for field in ["reason", "api", "evidence_stage", "interpreter"] {
            let mut changed = baseline.clone();
            changed.as_object_mut().unwrap().remove(field);
            assert!(
                decode_runtime_helper(&serde_json::to_vec(&changed).unwrap(), &context()).is_err(),
                "accepted missing {field}"
            );
        }
        let mut changed = baseline;
        changed["body"] = serde_json::json!("private conversation");
        assert!(decode_runtime_helper(&serde_json::to_vec(&changed).unwrap(), &context()).is_err());
        assert!(decode_runtime_helper(&vec![b' '; 16385], &context()).is_err());
    }

    #[test]
    fn machine_capture_refuses_shell_substitution_and_wrong_helper_tail() {
        let baseline: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../tests/fixtures/hermes/runtime-command.json"
        ))
        .unwrap();
        for (index, value) in [
            (0, "python3"),
            (1, "-E"),
            (2, "-m"),
            (6, "/foreign/helper.py"),
            (8, "sticky"),
        ] {
            let mut changed = baseline.clone();
            changed[index] = serde_json::json!(value);
            assert!(
                decode_machine_command(
                    &serde_json::to_vec(&changed).unwrap(),
                    Path::new("/fixture/owned helper/runtime_helper.py"),
                    "default"
                )
                .is_err(),
                "accepted {index}"
            );
        }
    }

    #[cfg(feature = "test-support")]
    struct TestDir(PathBuf);
    #[cfg(feature = "test-support")]
    impl TestDir {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    #[cfg(feature = "test-support")]
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(feature = "test-support")]
    fn launcher(
        body: &str,
    ) -> (
        TestDir,
        PathBuf,
        crate::harness::adapter::SetupEnvironment,
        crate::protocol::time::CallBudget,
    ) {
        use std::os::unix::fs::PermissionsExt;
        let tmp =
            TestDir(std::env::temp_dir().join(format!("hermes-runtime-{}", uuid::Uuid::new_v4())));
        std::fs::create_dir(&tmp.0).unwrap();
        let path = tmp.path().join("launcher with spaces");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let env = crate::harness::adapter::SetupEnvironment {
            home: Some(tmp.path().as_os_str().to_owned()),
            cwd: tmp.path().to_owned(),
            ..Default::default()
        };
        let budget = crate::protocol::time::CallBudget {
            deadline: crate::protocol::time::MonoInstant(5000),
            cancellation: Default::default(),
        };
        (tmp, path, env, budget)
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn capture_child_is_metadata_only_and_never_executes_returned_bootstrap() {
        let fixture = include_str!("../../../tests/fixtures/hermes/runtime-command.json");
        let (tmp, launch, env, budget) = launcher(&format!(
            "[ \"$1\" = '--print-runtime-command' ] || exit 71\n[ \"$3\" = 'trace' ] || exit 72\n[ \"$7\" = '/fixture/owned helper/runtime_helper.py' ] || exit 73\n[ \"$9\" = 'default' ] || exit 74\n/bin/cat <<'JSON'\n{fixture}JSON"
        ));
        let result = capture_machine_metadata(
            &launch,
            Path::new("/fixture/owned helper/runtime_helper.py"),
            "default",
            &env,
            &budget,
        )
        .unwrap();
        assert_eq!(result.reason, "native_bootstrap_read_only_unqualified");
        assert_eq!(result.machine_metadata.argv[8], "default");
        // Executing the returned nonexisting fixture interpreter would fail.
        assert!(!tmp.path().join("config.yaml").exists());
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn capture_budget_exhaustion_prevents_spawn() {
        let (tmp, launch, env, mut budget) = launcher("touch spawned");
        budget.deadline = crate::protocol::time::MonoInstant(0);
        assert_eq!(
            capture_machine_metadata(
                &launch,
                Path::new("/fixture/helper.py"),
                "default",
                &env,
                &budget
            )
            .unwrap_err(),
            ProbeFailure::Deadline
        );
        assert!(!tmp.path().join("spawned").exists());
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn capture_kills_and_reaps_owned_hung_child() {
        let (tmp, launch, env, budget) = launcher("printf '%s' $$ > owned-pid\nexec /bin/sleep 20");
        let pidfile = tmp.path().join("owned-pid");
        let cancellation = budget.cancellation.clone();
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let watcher_done = done.clone();
        let watcher = std::thread::spawn(move || {
            let started = Instant::now();
            while started.elapsed() < Duration::from_secs(3)
                && !watcher_done.load(std::sync::atomic::Ordering::SeqCst)
            {
                if std::fs::read_to_string(&pidfile)
                    .ok()
                    .and_then(|v| v.parse::<i32>().ok())
                    .is_some()
                {
                    cancellation.cancel();
                    return;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        });
        let result = capture_machine_metadata(
            &launch,
            Path::new("/fixture/helper.py"),
            "default",
            &env,
            &budget,
        );
        done.store(true, std::sync::atomic::Ordering::SeqCst);
        watcher.join().unwrap();
        assert_eq!(result.unwrap_err(), ProbeFailure::Cancelled);
        let pid: i32 = std::fs::read_to_string(tmp.path().join("owned-pid"))
            .unwrap()
            .parse()
            .unwrap();
        // Missing kill/reap leaves this owned child visible after the return.
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }

    #[test]
    fn current_native_qualification_is_refused_not_promoted_from_metadata() {
        assert_eq!(
            qualify_installed().unwrap_err(),
            ProbeFailure::Unavailable("native_read_only_boundary_unavailable".into())
        );
        let data: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../tests/fixtures/hermes/runtime-api.json"
        ))
        .unwrap();
        let mut wrong = data.clone();
        wrong["status"] = serde_json::json!("qualified");
        assert!(decode_runtime_helper(&serde_json::to_vec(&wrong).unwrap(), &context()).is_err());
        wrong = data;
        wrong["evidence_stage"] = serde_json::json!("unavailable");
        assert!(decode_runtime_helper(&serde_json::to_vec(&wrong).unwrap(), &context()).is_err());
    }

    fn metadata_fixture() -> serde_json::Value {
        serde_json::from_slice(include_bytes!(
            "../../../tests/fixtures/hermes/runtime-api.json"
        ))
        .unwrap()
    }

    #[test]
    fn git_metadata_without_commit_is_not_exact_source_identity() {
        let mut data = metadata_fixture();
        data["runtime_descriptor"]["commit"] = serde_json::Value::Null;
        assert!(decode_runtime_helper(&serde_json::to_vec(&data).unwrap(), &context()).is_err());
    }

    #[test]
    fn git_metadata_without_distance_is_not_exact_source_identity() {
        let mut data = metadata_fixture();
        data["runtime_descriptor"]["distance"] = serde_json::Value::Null;
        assert!(decode_runtime_helper(&serde_json::to_vec(&data).unwrap(), &context()).is_err());
    }

    #[test]
    fn descriptor_requires_explicit_nullable_fields() {
        let mut data = metadata_fixture();
        data["runtime_descriptor"]
            .as_object_mut()
            .unwrap()
            .remove("release_version");
        assert!(decode_runtime_helper(&serde_json::to_vec(&data).unwrap(), &context()).is_err());
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn capture_bounds_both_streams_and_handles_nonzero_exit() {
        for direction in ["", " >&2"] {
            let (_tmp, launch, env, budget) =
                launcher(&format!("printf '%s' '{}'{}", "x".repeat(16385), direction));
            assert_eq!(
                capture_machine_metadata(
                    &launch,
                    Path::new("/fixture/helper.py"),
                    "default",
                    &env,
                    &budget
                )
                .unwrap_err(),
                ProbeFailure::OutputLimit
            );
        }
        let (_tmp, launch, env, budget) = launcher("exit 7");
        assert_eq!(
            capture_machine_metadata(
                &launch,
                Path::new("/fixture/helper.py"),
                "default",
                &env,
                &budget
            )
            .unwrap_err(),
            ProbeFailure::ChildFailed
        );
    }
}

/// Diagnostic configuration quality; fallback subtype is never inferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigQuality {
    Successful,
    Unknown,
    FailedConfigRead,
}

/// Validated selected-home observation for assets only. This is not a
/// QualifiedRuntime and cannot supply callback admission or Working status.
#[derive(Debug)]
pub struct ProfileObservation {
    pub identity: RuntimeDescriptor,
    pub profile: String,
    pub home: PathBuf,
    pub physical_home: PathBuf,
    pub config_quality: ConfigQuality,
    pub enabled: Option<Vec<String>>,
    pub disabled: Option<Vec<String>>,
}
impl ProfileObservation {
    pub fn configured_enabled(&self) -> Option<bool> {
        let enabled = self.enabled.as_ref()?;
        let disabled = self.disabled.as_ref()?;
        (self.config_quality == ConfigQuality::Successful).then(|| {
            enabled.iter().any(|s| s == "herdr-threads")
                && !disabled.iter().any(|s| s == "herdr-threads")
        })
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeOrigins {
    hermes_bootstrap: PathBuf,
    hermes_constants: PathBuf,
    #[serde(rename = "hermes_cli.profiles")]
    profiles: PathBuf,
    #[serde(rename = "hermes_cli.version_info")]
    version: PathBuf,
    #[serde(rename = "hermes_cli.config")]
    config: PathBuf,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileResult {
    schema_version: u32,
    status: String,
    reason: Option<String>,
    runtime_descriptor: Option<RuntimeDescriptor>,
    profile: Option<String>,
    home: Option<PathBuf>,
    physical_home: Option<PathBuf>,
    interpreter: Option<PathBuf>,
    source_root: Option<PathBuf>,
    module_origins: Option<NativeOrigins>,
    dependency_paths: Option<Vec<PathBuf>>,
    enabled: Option<Vec<String>>,
    disabled: Option<Vec<String>>,
    config_quality: ConfigQuality,
    fallback_kind: Option<String>,
    environment_scope: String,
    cli_dotenv_loaded: bool,
    cli_scratch_rehomed: bool,
    identity_provenance: String,
    evidence_stage: String,
}
/// Strict complete schema2. Legacy API facts are intentionally absent.
pub fn decode_profile_observation(
    bytes: &[u8],
    expected: &MetadataScope,
) -> Result<ProfileObservation, ProbeFailure> {
    if bytes.len() > 16384 {
        return Err(ProbeFailure::Malformed);
    }
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| ProbeFailure::Malformed)?;
    let fields = [
        "schema_version",
        "status",
        "reason",
        "runtime_descriptor",
        "profile",
        "home",
        "physical_home",
        "interpreter",
        "source_root",
        "module_origins",
        "dependency_paths",
        "enabled",
        "disabled",
        "config_quality",
        "fallback_kind",
        "environment_scope",
        "cli_dotenv_loaded",
        "cli_scratch_rehomed",
        "identity_provenance",
        "evidence_stage",
    ];
    if !value
        .as_object()
        .is_some_and(|v| v.len() == fields.len() && fields.iter().all(|k| v.contains_key(*k)))
    {
        return Err(ProbeFailure::Malformed);
    }
    let r: ProfileResult = serde_json::from_slice(bytes).map_err(|_| ProbeFailure::Malformed)?;
    if r.schema_version != 2
        || r.fallback_kind.is_some()
        || r.cli_dotenv_loaded
        || r.cli_scratch_rehomed
        || r.environment_scope != "declared_child_input_plus_native_bootstrap_profile_effects"
        || r.identity_provenance != "startup_captured"
    {
        return Err(ProbeFailure::Malformed);
    }
    if r.status == "unavailable" {
        if r.reason.as_deref() != Some("inspection_unavailable")
            || r.evidence_stage != "unavailable"
            || r.config_quality != ConfigQuality::Unknown
            || fields[3..13].iter().any(|k| !value[*k].is_null())
        {
            return Err(ProbeFailure::Malformed);
        }
        return Err(ProbeFailure::Unavailable("inspection_unavailable".into()));
    }
    if r.status != "observed"
        || r.reason.is_some()
        || r.evidence_stage != "startup_profile_observation"
    {
        return Err(ProbeFailure::Malformed);
    }
    let descriptor = r.runtime_descriptor.ok_or(ProbeFailure::Malformed)?;
    let descriptor_fields = [
        "release_version",
        "source",
        "base_version",
        "derived_version",
        "commit",
        "dirty",
        "distance",
    ];
    if !value["runtime_descriptor"]
        .as_object()
        .is_some_and(|v| v.len() == 7 && descriptor_fields.iter().all(|k| v.contains_key(*k)))
        || descriptor.release_version.is_some()
        || !matches!(
            descriptor.source.as_str(),
            "build" | "commit-build" | "ci" | "docker" | "fallback" | "git" | "local" | "nix"
        )
        || descriptor.base_version.is_none()
        || descriptor.derived_version.is_none()
        || descriptor.dirty.is_none()
        || (descriptor.source == "git"
            && (descriptor.commit.is_none() || descriptor.distance.is_none()))
    {
        return Err(ProbeFailure::Malformed);
    }
    // This diagnostic retains the native source spelling, including
    // commit-build; it does not construct an admission RuntimeIdentity key.
    if descriptor
        .base_version
        .as_deref()
        .is_none_or(|s| !safe(s, 128))
        || descriptor
            .derived_version
            .as_deref()
            .is_none_or(|s| !safe(s, 128))
        || descriptor.commit.as_ref().is_some_and(|s| {
            s.len() != 40
                || !s
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        })
        || descriptor.distance.is_some_and(|d| d > u32::MAX as u64)
    {
        return Err(ProbeFailure::Malformed);
    }
    let home = r.home.ok_or(ProbeFailure::Malformed)?;
    let physical_home = r.physical_home.ok_or(ProbeFailure::Malformed)?;
    if r.interpreter.as_ref() != Some(&expected.interpreter)
        || r.source_root.as_ref() != Some(&expected.source_root)
        || r.profile.as_ref() != Some(&expected.profile)
        || home != expected.home
        || !safe(&expected.profile, 256)
        || [
            &home,
            &physical_home,
            &expected.interpreter,
            &expected.source_root,
        ]
        .iter()
        .any(|p| !absolute(p))
    {
        return Err(ProbeFailure::Malformed);
    }
    let origins = r.module_origins.ok_or(ProbeFailure::Malformed)?;
    let names = [
        "hermes_bootstrap",
        "hermes_constants",
        "hermes_cli.profiles",
        "hermes_cli.version_info",
        "hermes_cli.config",
    ];
    let paths = [
        origins.hermes_bootstrap,
        origins.hermes_constants,
        origins.profiles,
        origins.version,
        origins.config,
    ];
    if names.iter().zip(paths).any(|(name, path)| {
        path != expected
            .source_root
            .join(format!("{}.py", name.replace('.', "/")))
    }) || r.dependency_paths.as_ref().is_none_or(|v| {
        v.len() != 1 || !absolute(&v[0]) || v[0].file_name().is_none_or(|n| n != "site-packages")
    }) {
        return Err(ProbeFailure::Malformed);
    }
    if r.config_quality == ConfigQuality::Successful {
        if [&r.enabled, &r.disabled].iter().any(|v| {
            v.as_ref()
                .is_none_or(|names| names.len() > 128 || names.iter().any(|s| !safe(s, 256)))
        }) {
            return Err(ProbeFailure::Malformed);
        }
    } else if r.enabled.is_some() || r.disabled.is_some() {
        return Err(ProbeFailure::Malformed);
    }
    Ok(ProfileObservation {
        identity: descriptor,
        profile: expected.profile.clone(),
        home,
        physical_home,
        config_quality: r.config_quality,
        enabled: r.enabled,
        disabled: r.disabled,
    })
}

/// Explicit source/profile invocation context; callers never substitute PATH Python.
pub struct ProfileInspection<'a> {
    pub launcher: &'a Path,
    pub helper: &'a Path,
    pub scope: &'a MetadataScope,
    pub environment: &'a crate::harness::adapter::SetupEnvironment,
    pub budget: &'a crate::protocol::time::CallBudget,
}
impl ProfileInspection<'_> {
    pub fn observe(&self) -> Result<ProfileObservation, ProbeFailure> {
        observe_selected_profile(
            self.launcher,
            self.helper,
            self.scope,
            self.environment,
            self.budget,
        )
    }
}

/// Capture official argv, then execute it unchanged under ONE caller budget.
/// No native call is performed by decoding, and no result grants admission.
pub fn observe_selected_profile(
    launcher: &Path,
    helper: &Path,
    expected: &MetadataScope,
    env: &crate::harness::adapter::SetupEnvironment,
    budget: &crate::protocol::time::CallBudget,
) -> Result<ProfileObservation, ProbeFailure> {
    let started = Instant::now();
    let machine = capture_machine_metadata(launcher, helper, &expected.profile, env, budget)?
        .machine_metadata;
    check_budget(env.clock.as_ref(), budget, started)?;
    if machine.interpreter.canonicalize().ok().as_ref() != Some(&expected.interpreter) {
        return Err(ProbeFailure::UnsupportedShape);
    }
    let scope = serde_json::json!({"interpreter":expected.interpreter,"source_root":expected.source_root,"profile":expected.profile,"home":expected.home});
    let mut command = Command::new(&machine.interpreter);
    command
        .args(&machine.argv[1..])
        .env_clear()
        .envs(&env.declared)
        .env("HOME", env.home.as_ref().ok_or(ProbeFailure::Malformed)?)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("HERDR_HERMES_INSPECTION_SCOPE", scope.to_string())
        .current_dir(&env.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if let Some(path) = &env.path {
        command.env("PATH", path);
    }
    #[cfg(feature = "test-support")]
    crate::test_support::spawn::tag(&mut command);
    let bytes = run_capture(command, env.clock.as_ref(), budget, started)?;
    let observation = decode_profile_observation(&bytes, expected)?;
    check_budget(env.clock.as_ref(), budget, started)?;
    Ok(observation)
}

#[cfg(test)]
mod profile_observation_tests {
    use super::*;
    #[test]
    fn startup_profile_schema_is_separate_from_legacy_qualification() {
        let bytes = include_bytes!("../../../tests/fixtures/hermes/plugin-assets.json");
        assert!(
            decode_profile_observation(
                bytes,
                &MetadataScope {
                    interpreter: "/fixture/store python/bin/python3".into(),
                    source_root: "/fixture/native source".into(),
                    profile: "default".into(),
                    home: "/fixture/custom home".into()
                }
            )
            .is_ok(),
            "separate observation producer is missing"
        );
    }
    fn fixture_scope() -> MetadataScope {
        MetadataScope {
            interpreter: "/fixture/store python/bin/python3".into(),
            source_root: "/fixture/native source".into(),
            profile: "default".into(),
            home: "/fixture/custom home".into(),
        }
    }
    #[test]
    fn duplicate_native_origin_is_not_a_complete_profile_result() {
        let raw = include_str!("../../../tests/fixtures/hermes/plugin-assets.json");
        let changed=raw.replace("\"hermes_bootstrap\": \"/fixture/native source/hermes_bootstrap.py\"", "\"hermes_bootstrap\": \"/fixture/native source/hermes_bootstrap.py\",\"hermes_bootstrap\": \"/fixture/native source/hermes_bootstrap.py\"");
        assert_eq!(
            decode_profile_observation(changed.as_bytes(), &fixture_scope()).unwrap_err(),
            ProbeFailure::Malformed
        );
    }
    #[test]
    fn profile_config_quality_scope_and_bounds_never_become_admission_facts() {
        let data: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../tests/fixtures/hermes/plugin-assets.json"
        ))
        .unwrap();
        for (field, value) in [
            ("schema_version", serde_json::json!(1)),
            ("evidence_stage", serde_json::json!("live_model")),
            ("fallback_kind", serde_json::json!("last_known_good")),
            ("cli_dotenv_loaded", serde_json::json!(true)),
            ("environment_scope", serde_json::json!("full_cli")),
            ("config_quality", serde_json::json!("failed_config_read")),
            ("profile", serde_json::json!("work")),
            ("enabled", serde_json::json!(["secret\nvalue"])),
        ] {
            let mut changed = data.clone();
            changed[field] = value;
            assert!(
                decode_profile_observation(
                    &serde_json::to_vec(&changed).unwrap(),
                    &fixture_scope()
                )
                .is_err(),
                "accepted {field}"
            );
        }
        for field in [
            "fallback_kind",
            "reason",
            "physical_home",
            "runtime_descriptor",
        ] {
            let mut changed = data.clone();
            changed.as_object_mut().unwrap().remove(field);
            assert!(
                decode_profile_observation(
                    &serde_json::to_vec(&changed).unwrap(),
                    &fixture_scope()
                )
                .is_err(),
                "accepted missing {field}"
            );
        }
        let mut failed = data;
        failed["config_quality"] = serde_json::json!("failed_config_read");
        failed["enabled"] = serde_json::Value::Null;
        failed["disabled"] = serde_json::Value::Null;
        let got =
            decode_profile_observation(&serde_json::to_vec(&failed).unwrap(), &fixture_scope())
                .unwrap();
        assert_eq!(got.configured_enabled(), None);
        assert!(decode_profile_observation(&vec![b' '; 16385], &fixture_scope()).is_err());
        assert!(qualify_installed().is_err());
    }
    #[test]
    fn native_stamp_source_spelling_is_retained_without_admission_identity() {
        let mut value: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../tests/fixtures/hermes/plugin-assets.json"
        ))
        .unwrap();
        value["runtime_descriptor"]["source"] = serde_json::json!("commit-build");
        let got =
            decode_profile_observation(&serde_json::to_vec(&value).unwrap(), &fixture_scope())
                .unwrap();
        assert_eq!(got.identity.source, "commit-build");
        assert_eq!(
            got.identity.commit.as_deref(),
            Some("1234567890abcdef1234567890abcdef12345678")
        );
        assert_eq!(got.identity.distance, Some(1));
        assert_eq!(got.identity.release_version, None);
        value["runtime_descriptor"]["source"] = serde_json::json!("unknown");
        assert!(
            decode_profile_observation(&serde_json::to_vec(&value).unwrap(), &fixture_scope())
                .is_err()
        );
    }
}
