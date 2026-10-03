//! Codex hook-schema fingerprint admission. Fixtures are small synthetic
//! "binaries": shell scripts that print a `codex-cli` version line and exit,
//! followed by embedded schema text the shell never executes. The matching
//! fixture embeds the committed 0.158.0 schema extraction.
use crate::harness::codex::{
    self, Admission, HOOKS_V1_SCHEMA_FINGERPRINT, InstalledVersion, SCHEMA_MATCHED_LABEL,
    VersionError,
};
use crate::harness::codex_schema::{self, Unextractable};
use crate::harness::{Capability, setup::plan_codex_for_version};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const CAPTURE: &str = "docs/evidence/codex-158-hook-capture";
const LIVE_ROOT_TOOL: &[u8] =
    include_bytes!("../fixtures/codex-0.158.0/03-pretooluse-bash-root.json");
const LIVE_START: &[u8] = include_bytes!("../fixtures/codex-0.158.0/01-sessionstart-startup.json");

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The committed 0.158.0 extraction, `(file name, bytes)` in name order.
fn committed_schemas() -> Vec<(String, Vec<u8>)> {
    let dir = root().join(CAPTURE).join("schemas-0.158.0");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.to_string_lossy().ends_with(".schema.json"))
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|path| {
            (
                path.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read(&path).unwrap(),
            )
        })
        .collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn private_dir(label: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let p =
        PathBuf::from("/private/tmp").join(format!("ht-codex-fp-{label}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&p).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700)).unwrap();
    p
}

/// A shell script reporting `version` whose unexecuted tail embeds `tail`,
/// optionally with the embedded text re-indented as a compiled binary would
/// hold it (two-space pretty JSON between binary noise).
fn synthetic_binary(dir: &Path, name: &str, version: &str, tail: &[u8]) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    let mut bytes = format!("#!/bin/sh\nprintf 'codex-cli {version}\\n'\nexit 0\n").into_bytes();
    bytes.extend_from_slice(b"\x00\x01binary-noise{\"$schema\": 7}\x00");
    bytes.extend_from_slice(tail);
    bytes.extend_from_slice(b"\x00\xfftrailing-noise\n");
    std::fs::write(&path, bytes).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}

/// The committed schemas as a Codex binary embeds them: two-space pretty
/// JSON, each directly after a string-table neighbour with no separator.
fn embedded(schemas: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut tail = Vec::new();
    for (name, bytes) in schemas {
        let value: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        tail.extend_from_slice(name.trim_end_matches(".schema.json").as_bytes());
        tail.extend_from_slice(serde_json::to_string_pretty(&value).unwrap().as_bytes());
    }
    tail
}

/// The recipe fingerprint is exactly what the committed 0.158.0 extraction
/// hashes to, and the 0.157.1 extraction (committed by checksum) is the same
/// set of files, so both listed versions carry this fingerprint.
/// Kills: a recipe fingerprint not derived from the captured schemas, and a
/// canonicalization that depends on the committed files' formatting.
#[test]
fn recipe_fingerprint_is_computed_from_the_committed_captured_schemas() {
    let schemas = committed_schemas();
    assert_eq!(schemas.len(), 23);
    let computed =
        codex_schema::fingerprint_documents(schemas.iter().map(|(_, bytes)| bytes.as_slice()))
            .unwrap();
    assert_eq!(computed, HOOKS_V1_SCHEMA_FINGERPRINT);
    assert_eq!(
        codex::RECIPES[0].profile.schema_fingerprint,
        HOOKS_V1_SCHEMA_FINGERPRINT
    );
    // Re-embedded with a different layout, the canonical fingerprint holds.
    let far = Instant::now() + Duration::from_secs(60);
    let extracted = codex_schema::extract_schemas(&embedded(&schemas), far).unwrap();
    assert_eq!(extracted.len(), 23);
    assert_eq!(codex_schema::fingerprint(&extracted), computed);
    for sums in ["schemas-0.158.0/SHA256SUMS", "schemas-0.157.1.SHA256SUMS"] {
        let text = std::fs::read_to_string(root().join(CAPTURE).join(sums)).unwrap();
        let listed: Vec<(String, String)> = text
            .lines()
            .map(|line| {
                let (digest, path) = line.split_once("  ").unwrap();
                (
                    path.rsplit('/').next().unwrap().to_owned(),
                    digest.to_owned(),
                )
            })
            .collect();
        assert_eq!(listed.len(), 23, "{sums}");
        for (name, digest) in listed {
            let (_, bytes) = schemas.iter().find(|(file, _)| *file == name).unwrap();
            assert_eq!(sha256_hex(bytes), digest, "{sums}: {name}");
        }
    }
}

/// An unlisted version whose embedded schemas match is admitted to the
/// matching recipe as schema-matched, live-unverified; the evidence line and
/// the parsed event's capability say so; the transport gate stays shut.
/// Kills: skipping the fingerprint (admitting an unlisted version on version
/// syntax alone is caught by the mismatch test), reporting a schema-matched
/// admission as listed, and a schema-matched parse claiming ObservedInput.
#[test]
fn unlisted_version_with_matching_schemas_is_admitted_schema_matched() {
    let dir = private_dir("match");
    let binary = synthetic_binary(&dir, "codex", "0.160.0", &embedded(&committed_schemas()));
    let version = InstalledVersion::observe(&binary).unwrap();
    assert_eq!(version.as_str(), "0.160.0");
    assert_eq!(version.recipe(), &codex::RECIPES[0]);
    let expected_sha = sha256_hex(&std::fs::read(&binary).unwrap());
    assert_eq!(
        version.admission(),
        &Admission::SchemaMatched {
            fingerprint: HOOKS_V1_SCHEMA_FINGERPRINT.into(),
            binary_sha256: expected_sha.clone(),
        }
    );
    let evidence = version.evidence();
    assert!(
        evidence.starts_with(&format!(
            "codex 0.160.0: {SCHEMA_MATCHED_LABEL}: recipe codex-hooks-v1"
        )),
        "{evidence}"
    );
    assert!(evidence.contains(HOOKS_V1_SCHEMA_FINGERPRINT), "{evidence}");
    assert!(
        evidence.ends_with(&format!("binary sha256 {}", &expected_sha[..16])),
        "{evidence}"
    );
    assert!(evidence.len() <= 256);
    let event = codex::parse_event_for_version(LIVE_START, "external", &version).unwrap();
    assert_eq!(event.capability, Capability::SchemaMatchedInput);
    let tool = codex::parse_tool_invocation(LIVE_ROOT_TOOL, "external", &version).unwrap();
    assert_eq!(tool.event().capability, Capability::SchemaMatchedInput);
    assert_eq!(
        tool.scoped_command(),
        Err(codex::TransportError::Unsupported(
            codex::DECLARATION.limitation
        ))
    );
    plan_codex_for_version(&[], &["/private/tmp/hook".into()], &version).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

/// One changed schema (a new SessionStart source) changes the fingerprint:
/// the unlisted, not-older version is admitted optimistically with the
/// measured fingerprint recorded as drift, never as a schema match.
/// Kills: admitting on a mismatched fingerprint as schema-matched,
/// fingerprinting only titles or a subset of the schemas, and losing the
/// measured fingerprint from the evidence.
#[test]
fn unlisted_version_with_mismatched_schemas_is_optimistic_with_drift() {
    let dir = private_dir("mismatch");
    let mut schemas = committed_schemas();
    let start = schemas
        .iter_mut()
        .find(|(name, _)| name == "session-start.command.input.schema.json")
        .unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&start.1).unwrap();
    let text = serde_json::to_string(&value).unwrap();
    assert!(text.contains("\"fork\""));
    value = serde_json::from_str(&text.replace("\"fork\"", "\"fork\",\"branch\"")).unwrap();
    start.1 = serde_json::to_vec(&value).unwrap();
    // A missing schema also mismatches.
    let mut fewer = committed_schemas();
    fewer.retain(|(name, _)| name != "interrupt.command.output.schema.json");
    for (label, tail) in [
        ("changed", embedded(&schemas)),
        ("missing", embedded(&fewer)),
    ] {
        let binary = synthetic_binary(&dir, label, "0.160.0", &tail);
        let version = InstalledVersion::observe(&binary).unwrap();
        let Admission::Optimistic { admission, schema } = version.admission() else {
            panic!("{label}: {:?}", version.admission());
        };
        let codex::SchemaObservation::Drift { fingerprint } = schema else {
            panic!("{label}: {schema:?}");
        };
        assert!(fingerprint.starts_with("sha256:"));
        assert_ne!(fingerprint, HOOKS_V1_SCHEMA_FINGERPRINT, "{label}");
        assert_eq!(admission.assumed_recipe, "codex-hooks-v1");
        assert_eq!(version.recipe().id, "codex-hooks-v1");
        let evidence = version.evidence();
        assert!(
            evidence.starts_with(
                "codex 0.160.0: optimistic (newer-than-verified): assumed recipe codex-hooks-v1; schema drift sha256:"
            ) && evidence.contains(fingerprint.as_str())
                && evidence.len() <= 256,
            "{evidence}"
        );
        let event = codex::parse_event_for_version(LIVE_START, "external", &version).unwrap();
        assert_eq!(event.capability, Capability::OptimisticInput);
        let state = codex::InstalledAdmission {
            binary: Some(binary.clone()),
            result: Ok(version.clone()),
        }
        .state();
        assert_eq!(state, codex::OPTIMISTIC_LABEL);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// No embedded schemas, conflicting duplicates, or a binary that changed
/// while observed: the unlisted version is admitted optimistically with an
/// `Unreadable` schema observation; an already-passed deadline still fails
/// extraction itself.
/// Kills: treating an unextractable binary as matching, extraction that
/// ignores the observation deadline, and dropping the reason.
#[test]
fn unlisted_version_with_unextractable_schemas_is_optimistic_unreadable() {
    let dir = private_dir("unextractable");
    let none = synthetic_binary(&dir, "none", "0.160.0", b"no schemas here");
    let schemas = committed_schemas();
    let mut conflicting = embedded(&schemas);
    let (_, stop) = schemas
        .iter()
        .find(|(name, _)| name == "stop.command.output.schema.json")
        .unwrap();
    let mut changed: serde_json::Value = serde_json::from_slice(stop).unwrap();
    changed["description"] = "a second, different stop output schema".into();
    conflicting.extend_from_slice(serde_json::to_string_pretty(&changed).unwrap().as_bytes());
    let conflicting = synthetic_binary(&dir, "conflicting", "0.160.0", &conflicting);
    for (binary, reason) in [
        (&none, Unextractable::NoSchemas),
        (
            &conflicting,
            Unextractable::Conflicting("stop.command.output".into()),
        ),
    ] {
        let version = InstalledVersion::observe(binary).unwrap();
        let Admission::Optimistic { schema, .. } = version.admission() else {
            panic!("{:?}", version.admission());
        };
        assert_eq!(
            schema,
            &codex::SchemaObservation::Unreadable {
                reason: reason.clone()
            }
        );
        let evidence = version.evidence();
        assert!(
            evidence.contains("schema unreadable") && evidence.contains(&reason.to_string()),
            "{evidence}"
        );
    }
    // A binary that modifies itself when run: the file fingerprinted is not
    // the binary that reported the version.
    let rewriting = {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("rewriting");
        let mut bytes =
            b"#!/bin/sh\nprintf 'codex-cli 0.160.0\\n'\nsleep 0.01; touch \"$0\"\nexit 0\n"
                .to_vec();
        bytes.extend_from_slice(&embedded(&schemas));
        std::fs::write(&path, bytes).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    };
    let version = InstalledVersion::observe(&rewriting).unwrap();
    assert!(
        matches!(
            version.admission(),
            Admission::Optimistic {
                schema: codex::SchemaObservation::Unreadable {
                    reason: Unextractable::Changed
                },
                ..
            }
        ),
        "{:?}",
        version.admission()
    );
    let far_past = Instant::now();
    assert_eq!(
        codex_schema::extract_schemas(&embedded(&schemas), far_past),
        Err(Unextractable::Deadline)
    );
    let fresh = synthetic_binary(&dir, "fresh", "0.160.0", &embedded(&schemas));
    assert_eq!(
        codex_schema::fingerprint_binary(&fresh, far_past, None),
        Err(Unextractable::Deadline)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

/// An older-than-every-recipe version whose binary embeds the matching
/// schemas is refused (ladder row 5), not schema-matched, and the binary is
/// never fingerprinted (design roast r2, ht-p03.62).
/// An npm layout: `.bin/codex` -> `../@openai/codex/bin/codex.js` (a script
/// printing `version`, embedding no schemas) and one native vendor binary per
/// `(package, triple)` in `vendors`, each embedding `tail`.
fn npm_layout(
    dir: &Path,
    version: &str,
    vendors: &[(&str, &str)],
    tail: &[u8],
) -> (PathBuf, Vec<PathBuf>) {
    use std::os::unix::fs::PermissionsExt;
    let modules = dir.join("node_modules");
    let wrapper_dir = modules.join("@openai/codex/bin");
    std::fs::create_dir_all(&wrapper_dir).unwrap();
    let wrapper = wrapper_dir.join("codex.js");
    std::fs::write(
        &wrapper,
        format!("#!/bin/sh\nprintf 'codex-cli {version}\\n'\nexit 0\n"),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::create_dir_all(modules.join(".bin")).unwrap();
    let shim = modules.join(".bin/codex");
    std::os::unix::fs::symlink("../@openai/codex/bin/codex.js", &shim).unwrap();
    let natives = vendors
        .iter()
        .map(|(package, triple)| {
            let bin = modules.join(format!("@openai/{package}/vendor/{triple}/bin"));
            std::fs::create_dir_all(&bin).unwrap();
            synthetic_binary(&bin, "codex", version, tail)
        })
        .collect();
    (shim, natives)
}

/// An npm-installed Codex is fingerprinted through its native vendor binary,
/// not the JS wrapper, and the observation records which file that was.
/// Kills: fingerprinting the wrapper (Optimistic/NoSchemas for a matching
/// install) and not recording the fingerprinted path.
#[test]
fn npm_wrapper_resolves_to_the_vendor_binary_and_schema_matches() {
    let dir = private_dir("npm");
    let (shim, natives) = npm_layout(
        &dir,
        "0.160.0",
        &[("codex-darwin-arm64", "aarch64-apple-darwin")],
        &embedded(&committed_schemas()),
    );
    let version = InstalledVersion::observe(&shim).unwrap();
    assert_eq!(
        version.admission(),
        &Admission::SchemaMatched {
            fingerprint: HOOKS_V1_SCHEMA_FINGERPRINT.into(),
            binary_sha256: sha256_hex(&std::fs::read(&natives[0]).unwrap()),
        }
    );
    assert_eq!(
        version.fingerprinted(),
        Some(std::fs::canonicalize(&natives[0]).unwrap().as_path())
    );
    std::fs::remove_dir_all(dir).unwrap();
}

/// A wrapper with no vendor sibling keeps the existing refusal: optimistic
/// with "no embedded hook command schemas found".
#[test]
fn bare_wrapper_without_vendor_sibling_reports_the_existing_error() {
    let dir = private_dir("npm-bare");
    let (shim, _) = npm_layout(&dir, "0.160.0", &[], b"");
    let version = InstalledVersion::observe(&shim).unwrap();
    assert!(
        matches!(
            version.admission(),
            Admission::Optimistic {
                schema: codex::SchemaObservation::Unreadable {
                    reason: Unextractable::NoSchemas
                },
                ..
            }
        ),
        "{:?}",
        version.admission()
    );
    assert_eq!(
        Unextractable::NoSchemas.to_string(),
        "no embedded hook command schemas found"
    );
    assert_eq!(version.fingerprinted(), None);
    std::fs::remove_dir_all(dir).unwrap();
}

/// Several vendor binaries cannot be attributed to the running wrapper: the
/// error names the count, and nothing is schema-matched.
#[test]
fn several_vendor_binaries_is_an_error() {
    let dir = private_dir("npm-several");
    let (shim, _) = npm_layout(
        &dir,
        "0.160.0",
        &[
            ("codex-darwin-arm64", "aarch64-apple-darwin"),
            ("codex-linux-x64", "x86_64-unknown-linux-musl"),
        ],
        &embedded(&committed_schemas()),
    );
    assert_eq!(
        codex_schema::fingerprint_target(&shim),
        Err(Unextractable::SeveralVendorBinaries(2))
    );
    assert!(
        Unextractable::SeveralVendorBinaries(2)
            .to_string()
            .starts_with("2 platform vendor binaries")
    );
    let version = InstalledVersion::observe(&shim).unwrap();
    assert!(
        matches!(
            version.admission(),
            Admission::Optimistic {
                schema: codex::SchemaObservation::Unreadable {
                    reason: Unextractable::SeveralVendorBinaries(2)
                },
                ..
            }
        ),
        "{:?}",
        version.admission()
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn older_version_with_matching_schemas_is_refused_without_fingerprinting() {
    let dir = private_dir("older-match");
    let binary = synthetic_binary(&dir, "old", "0.155.1", &embedded(&committed_schemas()));
    assert_eq!(
        InstalledVersion::observe(&binary),
        Err(VersionError::Unsupported("0.155.1".into()))
    );
    // Inside the span with matching schemas stays schema-matched.
    let inside = synthetic_binary(&dir, "inside", "0.157.5", &embedded(&committed_schemas()));
    let version = InstalledVersion::observe(&inside).unwrap();
    assert!(matches!(
        version.admission(),
        Admission::SchemaMatched { .. }
    ));
    std::fs::remove_dir_all(dir).unwrap();
}

/// Listed versions keep exact behaviour: admitted as listed without reading
/// the binary, even when it embeds schemas that match no recipe.
/// Kills: fingerprinting (or refusing) a listed version, and a listed
/// admission reported as schema-matched.
#[test]
fn listed_versions_are_admitted_exactly_without_fingerprinting() {
    let dir = private_dir("listed");
    for listed in ["0.157.1", "0.158.0"] {
        let binary = synthetic_binary(&dir, listed, listed, b"no schemas here");
        let version = InstalledVersion::observe(&binary).unwrap();
        assert_eq!(version.admission(), &Admission::Listed);
        assert_eq!(
            version.evidence(),
            format!("codex {listed}: listed recipe codex-hooks-v1")
        );
        let event = codex::parse_event_for_version(LIVE_START, "external", &version).unwrap();
        assert_eq!(event.capability, Capability::ObservedInput);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// A fingerprint is cached per binary identity (path, inode, size, mtime,
/// ctime) and records the binary SHA-256; the persistent cache survives the
/// in-process cache; rewriting the binary invalidates both.
/// Kills: a cache keyed on path alone (a replaced binary reusing a stale
/// admission), and a persistent cache that is never consulted.
#[test]
fn fingerprints_are_cached_per_binary_identity() {
    let dir = private_dir("cache");
    let cache = dir.join("codex-schema-cache.json");
    let schemas = committed_schemas();
    let binary = synthetic_binary(&dir, "codex", "0.160.0", &embedded(&schemas));
    let far = Instant::now() + Duration::from_secs(60);
    let first = codex_schema::fingerprint_binary(&binary, far, Some(&cache)).unwrap();
    assert_eq!(first.fingerprint, HOOKS_V1_SCHEMA_FINGERPRINT);
    assert!(cache.is_file());
    codex_schema::clear_memory_cache_for_test();
    // Served from the persistent cache: an expired deadline would otherwise fail.
    let expired = Instant::now();
    assert_eq!(
        codex_schema::fingerprint_binary(&binary, expired, Some(&cache)),
        Ok(first.clone())
    );
    // And from memory without the file.
    assert_eq!(
        codex_schema::fingerprint_binary(&binary, expired, None),
        Ok(first.clone())
    );
    std::thread::sleep(Duration::from_millis(20));
    let replaced = synthetic_binary(&dir, "codex", "0.160.0", b"no schemas here");
    assert_eq!(replaced, binary);
    assert_eq!(
        codex_schema::fingerprint_binary(&binary, far, Some(&cache)),
        Err(Unextractable::NoSchemas)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

/// Manual real-binary check, read-only: `HT_CODEX_BINARY=/abs/path/codex
/// cargo test --release --lib -- --ignored real_installed_codex_schema_admission --nocapture`.
#[test]
#[ignore]
fn real_installed_codex_schema_admission() {
    let path = std::env::var("HT_CODEX_BINARY").expect("HT_CODEX_BINARY");
    let binary = Path::new(&path);
    let started = Instant::now();
    let measured =
        codex_schema::fingerprint_binary(binary, Instant::now() + Duration::from_secs(60), None);
    eprintln!("fingerprint {measured:?} in {:?}", started.elapsed());
    codex_schema::clear_memory_cache_for_test();
    let started = Instant::now();
    let observed = InstalledVersion::observe(binary);
    eprintln!("observe {:?} in {:?}", observed, started.elapsed());
    match observed {
        Ok(version) => eprintln!("evidence: {}", version.evidence()),
        Err(error) => eprintln!("refusal: {error}"),
    }
}

fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::symlink_metadata(path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777
}

/// The hook entrypoint's Codex observation with a state directory: the
/// fingerprint cache lives in a private `<state>/harness/` (0700) as a 0600
/// file keyed by binary identity and is consulted by a later (one-shot) hook
/// process; the admission evidence is stored beside it and rewritten only
/// when it changes; a cache that is not private is ignored and replaced.
/// Kills: the hook observing without the persistent cache (the tampered
/// entry would not be seen), trusting a group/other-writable cache file,
/// not storing the admission evidence (or storing it as listed), rewriting
/// the record on every hook, and a cache or record wider than 0600/0700.
#[test]
fn hook_observation_uses_a_private_persistent_cache_and_stores_evidence() {
    use crate::cli::hook::{InstalledHarness, observe_harness_in};
    use crate::harness::codex_evidence;
    use crate::harness::context::Harness;
    let root = private_dir("hook-cache");
    let state = root.join("state");
    std::fs::create_dir(&state).unwrap();
    std::fs::set_permissions(&state, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let bin = root.join("bin");
    std::fs::create_dir(&bin).unwrap();
    let schemas = committed_schemas();
    synthetic_binary(&bin, "codex", "0.160.0", &embedded(&schemas));
    let path = std::ffi::OsString::from(bin.as_os_str());
    let budget = Duration::from_secs(5);
    let observe = || observe_harness_in(Harness::Codex, Some(&path), budget, Some(&state));

    // Cold: fingerprinted, cached privately, evidence stored.
    codex_schema::clear_memory_cache_for_test();
    let Ok(InstalledHarness::Codex(version)) = observe() else {
        panic!("cold observation refused");
    };
    assert!(matches!(
        version.admission(),
        Admission::SchemaMatched { .. }
    ));
    let private = state.join("harness");
    let cache = codex_evidence::cache_path(&private);
    let record_path = codex_evidence::admission_path(&private);
    assert_eq!(mode(&private), 0o700);
    assert_eq!(mode(&cache), 0o600);
    assert_eq!(mode(&record_path), 0o600);
    let record = codex_evidence::read(&record_path).unwrap();
    assert!(record.schema_matched(), "{record:?}");
    assert_eq!(record.admission, SCHEMA_MATCHED_LABEL);
    assert_eq!(record.evidence, version.evidence());
    assert_eq!(
        record.binary.as_deref(),
        Some(bin.join("codex").to_str().unwrap())
    );

    // Unchanged evidence is not rewritten.
    let stamped = std::fs::metadata(&record_path).unwrap().modified().unwrap();
    std::thread::sleep(Duration::from_millis(20));
    codex_schema::clear_memory_cache_for_test();
    assert!(observe().is_ok());
    assert_eq!(codex_evidence::read(&record_path).unwrap(), record);
    assert_eq!(
        std::fs::metadata(&record_path).unwrap().modified().unwrap(),
        stamped
    );

    // A later process consults the persistent cache: a tampered entry is
    // what it sees (proof it did not rescan): the fingerprint no longer
    // matches, so it is admitted optimistically with that drift, and the
    // optimistic admission is stored as the hook's evidence.
    let original = std::fs::read(&cache).unwrap();
    let tampered = String::from_utf8(original.clone())
        .unwrap()
        .replace(HOOKS_V1_SCHEMA_FINGERPRINT, "sha256:tampered");
    assert_ne!(tampered.as_bytes(), original.as_slice());
    std::fs::write(&cache, &tampered).unwrap();
    codex_schema::clear_memory_cache_for_test();
    let Ok(InstalledHarness::Codex(drifted)) = observe() else {
        panic!("a drifted fingerprint is admitted optimistically");
    };
    assert!(
        matches!(drifted.admission(), Admission::Optimistic { .. }),
        "{:?}",
        drifted.admission()
    );
    let record = codex_evidence::read(&record_path).unwrap();
    assert_eq!(record.admission, codex::OPTIMISTIC_LABEL);
    assert!(
        record.optimistic() && !record.schema_matched(),
        "{record:?}"
    );
    assert!(record.evidence.contains("sha256:tampered"), "{record:?}");

    // A cache file that is not private is not trusted: rescanned, admitted
    // again, and the cache rewritten private.
    std::fs::set_permissions(&cache, std::os::unix::fs::PermissionsExt::from_mode(0o644)).unwrap();
    codex_schema::clear_memory_cache_for_test();
    assert!(matches!(observe(), Ok(InstalledHarness::Codex(_))));
    assert_eq!(mode(&cache), 0o600);
    assert!(
        !std::fs::read_to_string(&cache)
            .unwrap()
            .contains("sha256:tampered")
    );
    assert!(codex_evidence::read(&record_path).unwrap().schema_matched());
    std::fs::remove_dir_all(root).unwrap();
}

/// Read-only cache access (doctor) consults the cache but never creates or
/// writes it; without a state directory, or under an unsafe state root, the
/// hook observation persists nothing and still admits by fingerprint.
/// Kills: doctor writing the hook's cache, and the hook creating state under
/// a missing or group/other-writable state root.
#[test]
fn read_only_and_stateless_observation_write_nothing() {
    use crate::cli::hook::observe_harness_in;
    use crate::harness::context::Harness;
    let root = private_dir("hook-nostate");
    let bin = root.join("bin");
    std::fs::create_dir(&bin).unwrap();
    let binary = synthetic_binary(&bin, "codex", "0.160.0", &embedded(&committed_schemas()));
    let far = Instant::now() + Duration::from_secs(60);
    let cache = root.join("cache.json");
    codex_schema::clear_memory_cache_for_test();
    let measured = codex_schema::fingerprint_binary_with(
        &binary,
        far,
        codex_schema::FingerprintCache::ReadOnly(&cache),
    )
    .unwrap();
    assert_eq!(measured.fingerprint, HOOKS_V1_SCHEMA_FINGERPRINT);
    assert!(!cache.exists());
    // A read-only consumer is served by an existing cache.
    codex_schema::clear_memory_cache_for_test();
    codex_schema::fingerprint_binary(&binary, far, Some(&cache)).unwrap();
    assert!(cache.is_file());
    codex_schema::clear_memory_cache_for_test();
    assert_eq!(
        codex_schema::fingerprint_binary_with(
            &binary,
            Instant::now(),
            codex_schema::FingerprintCache::ReadOnly(&cache),
        ),
        Ok(measured)
    );

    let path = std::ffi::OsString::from(bin.as_os_str());
    let budget = Duration::from_secs(5);
    assert!(observe_harness_in(Harness::Codex, Some(&path), budget, None).is_ok());
    let missing = root.join("missing-state");
    assert!(observe_harness_in(Harness::Codex, Some(&path), budget, Some(&missing)).is_ok());
    assert!(!missing.exists());
    let open = root.join("open-state");
    std::fs::create_dir(&open).unwrap();
    std::fs::set_permissions(&open, std::os::unix::fs::PermissionsExt::from_mode(0o777)).unwrap();
    assert!(observe_harness_in(Harness::Codex, Some(&path), budget, Some(&open)).is_ok());
    assert!(!open.join("harness").exists());
    std::fs::remove_dir_all(root).unwrap();
}
