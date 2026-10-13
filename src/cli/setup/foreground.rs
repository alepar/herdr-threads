//! Read-only user foreground choices. Configured settings are hints, never execution proof.
use super::{Harness, SetupEnv, Value, json};
use std::{fs, io::Read, path::Path};

const LIMIT: u64 = 1 << 20;
const FOREGROUND_WARNING_PREFIX: &str = "Foreground user settings ";
const EXPLANATION: &str = "herdr-threads requires each managed agent to remain in its pane: a shared harness server may run hooks/tools with its own pane environment instead of the TUI environment used for seat attribution. Setup inspects user settings and changes a foreground setting only through an owned component the user explicitly consents to, where the advice names one. User settings, environment and launch arguments are not proof of execution; project/managed settings, other shells, and wrapper behavior may override them.";

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

pub(crate) fn inspect(harness: Harness, env: &SetupEnv) -> Value {
    inspect_in(crate::harness::registry::builtins(), harness.as_str(), env)
}

/// The foreground report of adapter `id` in `registry`; `null` for an adapter that declares no
/// foreground advice.
pub(crate) fn inspect_in(
    registry: &crate::harness::registry::Registry,
    id: &str,
    env: &SetupEnv,
) -> Value {
    let Some(registration) = registry
        .agent(id)
        .ok()
        .and_then(|id| registry.by_id(id).ok())
    else {
        return Value::Null;
    };
    let Some(spec) = registration.foreground_advice() else {
        return Value::Null;
    };
    let path = env
        .config_roots
        .get(registration.metadata().id)
        .map(|root| root.join(spec.settings_file));
    let mut report = json!({
        "status": "required", "scope": "user", "execution": "unknown",
        "settings_file": path.as_ref().map(|p| p.display().to_string()),
        "explanation": EXPLANATION, "advice": spec.advice, "modified_by_setup": false,
    });
    for (key, value) in spec.fixed_fields {
        report[*key] = json!(value);
    }
    let result = path
        .as_ref()
        .ok_or_else(|| "user config directory is unknown".to_owned())
        .and_then(|p| read(p))
        .and_then(|bytes| match bytes {
            None => {
                report["user_config"] = json!("missing");
                Ok(())
            }
            Some(bytes) => {
                (spec.inspect)(&bytes, &mut report)?;
                report["user_config"] = json!("read");
                Ok(())
            }
        });
    if let Err(error) = result {
        report["status"] = json!("unknown");
        report["inspection_error"] = json!(error);
    }
    if let Some(inspect) = spec.inspect_environment {
        inspect(&env.snapshot(), &mut report);
    }
    report
}

pub(crate) fn attach(report: &mut Value, harness: Harness, env: &SetupEnv) {
    let foreground = inspect(harness, env);
    let warning = format!(
        "{FOREGROUND_WARNING_PREFIX}{}. {} {}",
        foreground["status"].as_str().unwrap_or("unknown"),
        EXPLANATION,
        foreground["advice"].as_str().unwrap_or_default()
    );
    if !report["warnings"].is_array() {
        report["warnings"] = json!([]);
    }
    let warnings = report["warnings"].as_array_mut().unwrap();
    // Attaching again (after a consent answer) replaces the earlier reading.
    warnings.retain(|v| {
        !v.as_str()
            .is_some_and(|w| w.starts_with(FOREGROUND_WARNING_PREFIX))
    });
    if foreground["status"] != "configured" {
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
