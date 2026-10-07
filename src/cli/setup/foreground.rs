//! Read-only user foreground choices. Configured settings are hints, never execution proof.
use super::{Harness, SetupEnv, Value, json};
use std::{fs, io::Read, path::Path};

const LIMIT: u64 = 1 << 20;
const EXPLANATION: &str = "herdr-threads requires each managed agent to remain in its pane: a shared harness server may run hooks/tools with its own pane environment instead of the TUI environment used for seat attribution. Setup inspects user settings only and never changes foreground settings. User settings and launch arguments are not proof of execution; project/managed settings, environment, and wrapper behavior may override them.";
const CLAUDE_ADVICE: &str = "Set disableAgentView to true in Claude's user settings.json (merge {\"disableAgentView\":true} into the existing object), or set env.CLAUDE_CODE_DISABLE_AGENT_VIEW to \"1\" there. This disables agent view, --bg/background and on-demand daemon mode. CLAUDE_CODE_DISABLE_BACKGROUND_TASKS is a different setting and does not replace this choice.";
const CODEX_ADVICE: &str = "Codex 0.160.1 has no persistent user config/environment opt-out for daemon attachment. features.daemon_auto_start = false prevents automatic startup only: Codex still attaches to an existing daemon. Use native codex --no-daemon, or set HERDR_THREADS_CODEX_OPTS='--no-daemon' for herdr-threads launch when the selected binary/wrapper supports and forwards that argument. These configured arguments do not prove that a wrapper runs in the foreground; verify its behavior.";

/// Bound reads and reject symlinks/special files before reading; O_NONBLOCK also covers a
/// file replaced with a FIFO between metadata inspection and open.
fn read(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let meta = match path.symlink_metadata() {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot inspect user config: {error}")),
    };
    if !meta.file_type().is_file() {
        return Err("user config must be a regular file (symlinks are not followed)".into());
    }
    if meta.len() > LIMIT {
        return Err("user config exceeds 1 MiB read limit".into());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|e| format!("cannot read user config: {e}"))?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("user config must be a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > LIMIT {
        return Err("user config exceeds 1 MiB read limit".into());
    }
    Ok(Some(bytes))
}

fn truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

pub(super) fn inspect(harness: Harness, env: &SetupEnv) -> Value {
    let (path, advice) = match harness {
        Harness::Claude => (
            env.claude_config_dir
                .as_ref()
                .map(|p| p.join("settings.json")),
            CLAUDE_ADVICE,
        ),
        Harness::Codex => (
            env.codex_home.as_ref().map(|p| p.join("config.toml")),
            CODEX_ADVICE,
        ),
        _ => return Value::Null,
    };
    let mut report = json!({
        "status": "required", "scope": "user", "execution": "unknown",
        "settings_file": path.as_ref().map(|p| p.display().to_string()),
        "explanation": EXPLANATION, "advice": advice, "modified_by_setup": false,
    });
    if harness == Harness::Codex {
        report["persistent_opt_out"] = json!("unavailable (Codex 0.160.1)");
    }
    let inspected = path
        .as_ref()
        .ok_or_else(|| "user config directory is unknown".to_owned())
        .and_then(|p| read(p));
    let result = inspected.and_then(|bytes| match (harness, bytes) {
        (_, None) => {
            report["user_config"] = json!("missing");
            Ok(())
        }
        (Harness::Claude, Some(bytes)) => {
            let settings: Value = serde_json::from_slice(&bytes)
                .map_err(|_| "invalid user settings JSON".to_owned())?;
            if !settings.is_object() {
                return Err("user settings JSON must be an object".into());
            }
            let flag = settings.get("disableAgentView");
            let env_flag = settings
                .get("env")
                .and_then(|v| v.get("CLAUDE_CODE_DISABLE_AGENT_VIEW"));
            report["disableAgentView"] = flag.cloned().unwrap_or(Value::Null);
            report["CLAUDE_CODE_DISABLE_AGENT_VIEW"] = env_flag.cloned().unwrap_or(Value::Null);
            let env_enabled = env_flag.and_then(Value::as_str).is_some_and(truthy);
            if flag == Some(&Value::Bool(true)) || env_enabled {
                report["status"] = json!("configured");
            } else if flag.is_some_and(|v| !v.is_boolean())
                || env_flag.is_some_and(|v| !v.is_string())
            {
                return Err("foreground user setting has an unrecognized value".into());
            }
            report["user_config"] = json!("read");
            Ok(())
        }
        (Harness::Codex, Some(bytes)) => {
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| "user config TOML is not UTF-8".to_owned())?;
            let config = text
                .parse::<toml_edit::DocumentMut>()
                .map_err(|_| "invalid user config TOML".to_owned())?;
            report["daemon_auto_start"] = config
                .get("features")
                .and_then(|v| v.get("daemon_auto_start"))
                .and_then(toml_edit::Item::as_bool)
                .map_or(Value::Null, Value::Bool);
            report["user_config"] = json!("read");
            Ok(())
        }
        _ => unreachable!(),
    });
    if let Err(error) = result {
        report["status"] = json!("unknown");
        report["inspection_error"] = json!(error);
    }
    report
}

pub(crate) fn attach(report: &mut Value, harness: Harness, env: &SetupEnv) {
    let foreground = inspect(harness, env);
    let warning = format!(
        "Foreground user settings {}. {} {}",
        foreground["status"].as_str().unwrap_or("unknown"),
        EXPLANATION,
        foreground["advice"].as_str().unwrap_or_default()
    );
    if !report["warnings"].is_array() {
        report["warnings"] = json!([]);
    }
    let warnings = report["warnings"].as_array_mut().unwrap();
    if foreground["status"] != "configured"
        && !warnings.iter().any(|v| v.as_str() == Some(&warning))
    {
        warnings.push(json!(warning));
    }
    report["foreground"] = foreground;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreground_inspection_bounds_reads_and_rejects_symlinks_and_special_files() {
        let root = std::env::temp_dir().join(format!("foreground-read-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let target = root.join("target");
        fs::write(&target, b"{}").unwrap();
        let link = root.join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(read(&link).unwrap_err().contains("regular file"));
        assert!(read(&root).unwrap_err().contains("regular file"));
        let large = root.join("large");
        fs::File::create(&large)
            .unwrap()
            .set_len(LIMIT + 1)
            .unwrap();
        assert!(read(&large).unwrap_err().contains("1 MiB"));
        let fifo = root.join("fifo");
        let name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: the NUL-terminated pathname remains alive for this call.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(read(&fifo).unwrap_err().contains("regular file"));
        assert_eq!(fs::read(&target).unwrap(), b"{}");
        fs::remove_dir_all(root).unwrap();
    }
}
