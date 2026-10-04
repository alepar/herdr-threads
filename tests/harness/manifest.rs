//! Manifest reader, embedded copy, cache and fetch policy (ht-xoc.3).
use crate::daemon::settings::{HarnessManifestSetting, InstanceSettings};
use crate::harness::admission::{render_versions_document, versions_document};
use crate::harness::manifest::{
    CacheMeta, CurlFetcher, Decision, EMBEDDED, FETCH_INTERVAL, FetchError, FetchOutcome,
    FetchReason, Fetcher, MANIFEST_URL, MAX_MANIFEST_BYTES, Manifest, ManifestError,
    ManifestPolicy, ManifestService, OffReason, RowEvidence, RowSource, RowStatus, embedded,
    format_rfc3339_utc, parse, policy_from, prefer_newer, read_meta, should_fetch,
};
use crate::protocol::time::{Clock, MonoInstant, UtcMillis};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

// -- fixtures ---------------------------------------------------------------

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "herdr-threads-manifest-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
    fn cache(&self) -> PathBuf {
        self.0.join("harness-manifest")
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct FakeClock(AtomicI64);

impl FakeClock {
    fn at(ms: i64) -> Arc<Self> {
        Arc::new(Self(AtomicI64::new(ms)))
    }
    fn advance(&self, ms: i64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.0.load(Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.load(Ordering::SeqCst) as u64)
    }
}

/// Scripted fetcher: records each call; blocks on `gate` when one is set.
struct FakeFetcher {
    calls: Mutex<Vec<(String, Option<String>)>>,
    script: Mutex<VecDeque<Result<FetchOutcome, FetchError>>>,
    gate: Arc<(Mutex<bool>, Condvar)>,
    gated: bool,
}

impl FakeFetcher {
    fn new(script: Vec<Result<FetchOutcome, FetchError>>) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::default(),
            script: Mutex::new(script.into()),
            gate: Arc::new((Mutex::new(true), Condvar::new())),
            gated: false,
        })
    }
    fn gated(script: Vec<Result<FetchOutcome, FetchError>>) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::default(),
            script: Mutex::new(script.into()),
            gate: Arc::new((Mutex::new(false), Condvar::new())),
            gated: true,
        })
    }
    fn release(&self) {
        *self.gate.0.lock().unwrap() = true;
        self.gate.1.notify_all();
    }
    fn calls(&self) -> Vec<(String, Option<String>)> {
        self.calls.lock().unwrap().clone()
    }
}

impl Fetcher for FakeFetcher {
    fn fetch(&self, url: &str, etag: Option<&str>) -> Result<FetchOutcome, FetchError> {
        self.calls
            .lock()
            .unwrap()
            .push((url.to_owned(), etag.map(str::to_owned)));
        if self.gated {
            let (open, changed) = &*self.gate;
            let mut open = open.lock().unwrap();
            let deadline = Instant::now() + Duration::from_secs(20);
            while !*open && Instant::now() < deadline {
                open = changed
                    .wait_timeout(open, Duration::from_millis(50))
                    .unwrap()
                    .0;
            }
        }
        self.script
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Err(FetchError::Failed("script exhausted".into())))
    }
}

type Lines = Arc<Mutex<Vec<String>>>;

fn service(
    dir: &TestDir,
    policy: ManifestPolicy,
    fetcher: Arc<dyn Fetcher>,
    clock: Arc<FakeClock>,
) -> (ManifestService, Lines) {
    let lines: Lines = Arc::default();
    let sink = Arc::clone(&lines);
    let service = ManifestService::new(
        dir.cache(),
        policy,
        fetcher,
        clock,
        Arc::new(move |line: &str| sink.lock().unwrap().push(line.to_owned())),
    );
    (service, lines)
}

fn doc(schema: u64, latest: &str, rows: Value) -> Vec<u8> {
    doc_at(schema, latest, None, rows)
}

fn doc_at(schema: u64, latest: &str, generated_at: Option<&str>, rows: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schema_version": schema,
        "generated_at": generated_at,
        "latest_release": latest,
        "contracts": {},
        "rows": rows,
    }))
    .unwrap()
}

fn unseen(version: &str) -> FetchReason {
    FetchReason::UnseenVersion {
        version: version.into(),
    }
}

fn body(bytes: Vec<u8>, etag: Option<&str>) -> Result<FetchOutcome, FetchError> {
    Ok(FetchOutcome::Body {
        bytes,
        etag: etag.map(str::to_owned),
    })
}

// -- reader -----------------------------------------------------------------

#[test]
fn embedded_copy_parses_as_schema_2() {
    let manifest = parse(EMBEDDED.as_bytes()).expect("embedded parses");
    assert!(manifest.has_row("claude", "2.1.283"));
    assert!(manifest.has_row("codex", "0.159.3"));
    assert_eq!(embedded(), &manifest);
    assert!(
        embedded().rows.len() >= 8,
        "an empty embedded manifest is the parse-failure fallback"
    );
}

#[test]
fn in_repo_generator_output_passes_the_reader() {
    let manifest = parse(render_versions_document(&versions_document()).as_bytes()).unwrap();
    let row = &manifest.rows[1];
    assert_eq!(
        (row.harness.as_str(), row.version.as_str()),
        ("claude", "2.1.284")
    );
    assert_eq!(row.status, Some(RowStatus::Verified));
    assert_eq!(row.evidence, Some(RowEvidence::NoModel));
    assert_eq!(row.source, Some(RowSource::Manual));
    assert_eq!(row.contract_id, None);
}

/// ht-xoc.6's writer output must keep passing this reader.
#[test]
fn canary_writer_shape_passes_the_reader() {
    let bytes = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/harness/testdata/manifest/schema2-canary.json"),
    )
    .unwrap();
    let manifest = parse(&bytes).unwrap();
    assert_eq!(
        manifest.generated_at.as_deref(),
        Some("2026-10-02T06:00:00Z")
    );
    assert_eq!(manifest.latest_release.as_deref(), Some("0.4.0"));
    assert_eq!(manifest.contracts.len(), 2);
    let broken = &manifest.rows[1];
    assert_eq!(broken.status, Some(RowStatus::KnownBroken));
    assert_eq!(broken.evidence, Some(RowEvidence::Schema));
    assert_eq!(broken.source, Some(RowSource::Canary));
    assert_eq!(broken.broken_event.as_deref(), Some("UserPromptSubmit"));
    assert_eq!(broken.broken_field.as_deref(), Some("session_id"));
    assert_eq!(broken.last_working.as_deref(), Some("0.159.3"));
    assert!(broken.issue_url.as_deref().unwrap().ends_with("/issues/12"));
    let own = manifest.contracts["codex"].clone();
    assert_eq!(
        manifest
            .status_row("codex", "0.160.0", &own)
            .map(|row| row.status),
        Some(Some(RowStatus::KnownBroken))
    );
}

/// `writer-output.json` is the canary writer's own output (scripts/canary/manifest.py `write`): the
/// selftest `all-pass` case, then `payload-break` on top of its output, with the stub binary's contract
/// ids. Regenerate with `HT_BLESS=1 python3 -m unittest scripts/canary/test_manifest.py`; that Python test
/// fails when the committed bytes drift from the writer, so this reader check cannot go stale.
#[test]
fn writer_output_passes_the_reader() {
    let bytes = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/harness/testdata/manifest/writer-output.json"),
    )
    .unwrap();
    let manifest = parse(&bytes).unwrap();
    assert_eq!(manifest.contracts["claude"], "c1a0c1a0c1a0c1a0");
    let verified = manifest
        .rows
        .iter()
        .find(|r| r.harness == "claude" && r.version == "2.1.287")
        .expect("verified row");
    assert_eq!(verified.status, Some(RowStatus::Verified));
    assert_eq!(verified.evidence, Some(RowEvidence::Live));
    assert_eq!(verified.source, Some(RowSource::Canary));
    assert_eq!(verified.contract_id.as_deref(), Some("c1a0c1a0c1a0c1a0"));
    let broken = manifest
        .rows
        .iter()
        .find(|r| r.status == Some(RowStatus::KnownBroken))
        .expect("known_broken row");
    assert_eq!(broken.version, "2.1.289");
    assert_eq!(broken.broken_event.as_deref(), Some("SessionStart"));
    assert_eq!(broken.broken_field.as_deref(), Some("session_id"));
    assert_eq!(broken.last_working.as_deref(), Some("2.1.288"));
    let own = manifest.contracts["claude"].clone();
    assert_eq!(
        manifest
            .status_row("claude", "2.1.289", &own)
            .map(|row| row.status),
        Some(Some(RowStatus::KnownBroken))
    );
}

#[test]
fn schema_1_and_3_are_unsupported() {
    assert_eq!(
        parse(&doc(1, "x", json!([]))),
        Err(ManifestError::UnsupportedSchema(Some(1)))
    );
    assert_eq!(
        parse(&doc(3, "x", json!([]))),
        Err(ManifestError::UnsupportedSchema(Some(3)))
    );
    assert_eq!(
        parse(br#"{"rows": []}"#),
        Err(ManifestError::UnsupportedSchema(None))
    );
}

#[test]
fn oversized_document_is_too_large() {
    let mut bytes = doc(2, "x", json!([]));
    bytes.resize(MAX_MANIFEST_BYTES, b' ');
    assert!(parse(&bytes).is_ok(), "exactly the cap is accepted");
    bytes.push(b' ');
    assert_eq!(parse(&bytes), Err(ManifestError::TooLarge));
}

#[test]
fn not_json_and_malformed_rows_are_rejected() {
    assert_eq!(parse(b"<html>"), Err(ManifestError::NotJson));
    assert!(matches!(
        parse(br#"{"schema_version": 2, "rows": {}}"#),
        Err(ManifestError::Invalid(_))
    ));
    assert!(matches!(
        parse(&doc(2, "x", json!([{"harness": "claude"}]))),
        Err(ManifestError::Invalid(_))
    ));
}

#[test]
fn unknown_fields_and_values_are_ignored() {
    let manifest = parse(&doc(
        2,
        "1.0.0",
        json!([{"harness": "claude", "version": "2.1.1", "status": "weird",
                "evidence": "telepathy", "source": "oracle", "extra": [1, 2],
                "contract_id": "A"}]),
    ))
    .unwrap();
    let row = &manifest.rows[0];
    assert_eq!((row.status, row.evidence, row.source), (None, None, None));
    // A row with an unknown status is no status data even for its own contract.
    assert!(manifest.status_row("claude", "2.1.1", "A").is_none());
    assert!(manifest.has_row("claude", "2.1.1"));
}

fn mixed() -> Manifest {
    parse(&doc(
        2,
        "0.9.0",
        json!([
            {"harness": "codex", "version": "0.170.0", "status": "known_broken",
             "contract_id": "A", "supported_since": null, "issue_url": "https://example.test/a"},
            {"harness": "codex", "version": "0.170.0", "status": "verified",
             "contract_id": "B", "supported_since": "0.3.0", "last_working": "0.169.0",
             "issue_url": "https://example.test/b"},
            {"harness": "codex", "version": "0.171.0", "status": "verified",
             "contract_id": null},
        ]),
    ))
    .unwrap()
}

#[test]
fn row_with_another_contract_is_no_status_data() {
    let manifest = mixed();
    assert!(manifest.status_row("codex", "0.170.0", "C").is_none());
    // A null contract id never matches a daemon contract.
    assert!(manifest.status_row("codex", "0.171.0", "C").is_none());
    assert!(manifest.has_row("codex", "0.171.0"));
    assert!(!manifest.has_row("codex", "0.172.0"));
    assert!(!manifest.has_row("claude", "0.170.0"));
}

#[test]
fn mixed_contract_rows() {
    let manifest = mixed();
    let own_a = manifest.status_row("codex", "0.170.0", "A").unwrap();
    assert_eq!(own_a.status, Some(RowStatus::KnownBroken));
    let pointers = manifest.release_pointers("codex", "0.170.0", "A");
    // The own row's null supported_since falls back to the other contract's.
    assert_eq!(pointers.supported_since.as_deref(), Some("0.3.0"));
    // Non-null fields on the own row win over another row's.
    assert_eq!(
        pointers.issue_url.as_deref(),
        Some("https://example.test/a")
    );
    assert_eq!(pointers.last_working.as_deref(), Some("0.169.0"));
    assert_eq!(pointers.latest_release.as_deref(), Some("0.9.0"));

    assert!(manifest.status_row("codex", "0.170.0", "C").is_none());
    let pointers = manifest.release_pointers("codex", "0.170.0", "C");
    assert_eq!(pointers.supported_since.as_deref(), Some("0.3.0"));
    // For a foreign contract the first row in file order supplies the field.
    assert_eq!(
        pointers.issue_url.as_deref(),
        Some("https://example.test/a")
    );

    let none = manifest.release_pointers("codex", "0.999.0", "A");
    assert_eq!(none.supported_since, None);
    assert_eq!(none.latest_release.as_deref(), Some("0.9.0"));
}

/// Kills: an accessor that returns the own-contract row, a known_broken row, a
/// null-contract row, or the first (not the greatest `supported_since`) row.
#[test]
fn other_contract_verified_picks_foreign_verified_row_with_greatest_supported_since() {
    let manifest = parse(&doc(
        2,
        "0.9.0",
        json!([
            {"harness": "codex", "version": "0.170.0", "status": "verified",
             "contract_id": "A", "supported_since": "9.9.9"},
            {"harness": "codex", "version": "0.170.0", "status": "known_broken",
             "contract_id": "B", "supported_since": "8.0.0"},
            {"harness": "codex", "version": "0.170.0", "status": "verified",
             "contract_id": null, "supported_since": "7.0.0"},
            {"harness": "codex", "version": "0.170.0", "status": "verified",
             "contract_id": "C", "supported_since": "0.3.0"},
            {"harness": "codex", "version": "0.170.0", "status": "verified",
             "contract_id": "D", "supported_since": "v0.12.0"},
            {"harness": "claude", "version": "0.170.0", "status": "verified",
             "contract_id": "E", "supported_since": "99.0.0"},
        ]),
    ))
    .unwrap();
    let found = manifest
        .other_contract_verified("codex", "0.170.0", "A")
        .unwrap();
    assert_eq!(found.contract_id.as_deref(), Some("D"));
    // Own contract excluded: from D's point of view the best other is C or A.
    let from_d = manifest
        .other_contract_verified("codex", "0.170.0", "D")
        .unwrap();
    assert_eq!(from_d.contract_id.as_deref(), Some("A"));
    assert!(
        manifest
            .other_contract_verified("codex", "0.999.0", "A")
            .is_none()
    );
}

// -- policy -----------------------------------------------------------------

fn settings(setting: HarnessManifestSetting) -> InstanceSettings {
    InstanceSettings {
        harness_manifest: setting,
        ..InstanceSettings::default()
    }
}

#[test]
fn policy_from_settings_and_env() {
    let auto = settings(HarnessManifestSetting::Auto);
    let off = settings(HarnessManifestSetting::Off);
    let one = OsString::from("1");
    assert_eq!(policy_from(&auto, None), ManifestPolicy::Auto);
    assert_eq!(
        policy_from(&auto, Some(&one)),
        ManifestPolicy::Off(OffReason::OfflineEnv)
    );
    assert_eq!(
        policy_from(&off, None),
        ManifestPolicy::Off(OffReason::Settings)
    );
    assert_eq!(
        policy_from(&off, Some(&one)),
        ManifestPolicy::Off(OffReason::Settings),
        "settings wins the label"
    );
    // Only the value 1 counts.
    for other in ["0", "", "true", "11"] {
        assert_eq!(
            policy_from(&auto, Some(&OsString::from(other))),
            ManifestPolicy::Auto,
            "{other:?}"
        );
    }
}

#[test]
fn should_fetch_table() {
    let mut meta = CacheMeta::default();
    let now = 10 * DAY_MS;
    let violation = FetchReason::FreshViolation;
    let fetch = |policy, reason: &FetchReason, harness, meta: &CacheMeta, has_row| {
        should_fetch(policy, reason, harness, now, meta, has_row)
    };
    let auto = ManifestPolicy::Auto;
    assert_eq!(
        fetch(auto, &violation, "claude", &meta, true),
        Decision::Fetch
    );
    assert_eq!(
        fetch(auto, &violation, "claude", &meta, false),
        Decision::Fetch
    );
    // Unseen: a row anywhere means no fetch; no row means fetch.
    assert!(matches!(
        fetch(auto, &unseen("2.1.9"), "claude", &meta, true),
        Decision::Skip(_)
    ));
    assert_eq!(
        fetch(auto, &unseen("2.1.9"), "claude", &meta, false),
        Decision::Fetch
    );
    // Once per day per harness: claude's attempt does not throttle codex.
    meta.attempts.insert("claude".into(), now - DAY_MS + 1);
    assert!(matches!(
        fetch(auto, &violation, "claude", &meta, false),
        Decision::Skip(_)
    ));
    assert_eq!(
        fetch(auto, &violation, "codex", &meta, false),
        Decision::Fetch
    );
    meta.attempts.insert("claude".into(), now - DAY_MS);
    assert_eq!(
        fetch(auto, &violation, "claude", &meta, false),
        Decision::Fetch,
        "exactly 24 h later is eligible"
    );
    assert_eq!(FETCH_INTERVAL.as_millis() as i64, DAY_MS);
    // Off never fetches, whatever the reason.
    meta.attempts.clear();
    for off in [OffReason::Settings, OffReason::OfflineEnv] {
        assert!(matches!(
            fetch(ManifestPolicy::Off(off), &violation, "claude", &meta, false),
            Decision::Skip(_)
        ));
        assert!(matches!(
            fetch(
                ManifestPolicy::Off(off),
                &unseen("1.0.0"),
                "claude",
                &meta,
                false
            ),
            Decision::Skip(_)
        ));
    }
}

// -- service ----------------------------------------------------------------

const WAIT: Duration = Duration::from_secs(20);

fn row(version: &str) -> Value {
    json!({"harness": "claude", "version": version, "status": "verified",
           "contract_id": "A", "supported_since": "0.3.0"})
}

#[test]
fn ensure_manifest_writes_cache_then_current_prefers_it() {
    let dir = TestDir::new();
    let clock = FakeClock::at(5 * DAY_MS);
    let fetcher = FakeFetcher::new(vec![body(
        doc(2, "7.7.7", json!([row("9.9.9")])),
        Some("\"v1\""),
    )]);
    let (service, _) = service(&dir, ManifestPolicy::Auto, fetcher.clone(), clock);
    assert_eq!(service.current().latest_release, None, "embedded first");
    service.ensure_manifest("claude", unseen("9.9.9"));
    assert!(service.wait_idle(WAIT));
    assert_eq!(fetcher.calls(), vec![(MANIFEST_URL.to_owned(), None)]);
    assert_eq!(service.current().latest_release.as_deref(), Some("7.7.7"));
    assert!(service.current().has_row("claude", "9.9.9"));
    let meta = read_meta(&dir.cache());
    assert_eq!(meta.etag.as_deref(), Some("\"v1\""));
    assert_eq!(meta.fetched_at_ms, Some(5 * DAY_MS));
    assert_eq!(meta.attempts["claude"], 5 * DAY_MS);

    // A new service (daemon restart) reads the cache, not the embedded copy.
    let (restarted, _) = self::service(
        &dir,
        ManifestPolicy::Off(OffReason::Settings),
        FakeFetcher::new(vec![]),
        FakeClock::at(6 * DAY_MS),
    );
    assert_eq!(restarted.current().latest_release.as_deref(), Some("7.7.7"));
    use std::os::unix::fs::PermissionsExt;
    let mode = |path: PathBuf| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(dir.cache()), 0o700);
    assert_eq!(mode(dir.cache().join("harness-versions.json")), 0o600);
    assert_eq!(mode(dir.cache().join("meta.json")), 0o600);
}

#[test]
fn cache_then_embedded_fallback() {
    let dir = TestDir::new();
    std::fs::create_dir_all(dir.cache()).unwrap();
    std::fs::write(dir.cache().join("harness-versions.json"), b"{ corrupt").unwrap();
    let (service, _) = service(
        &dir,
        ManifestPolicy::Auto,
        FakeFetcher::new(vec![]),
        FakeClock::at(0),
    );
    assert_eq!(&*service.current(), embedded());
    // A schema-3 file on disk is no cache either.
    let dir = TestDir::new();
    std::fs::create_dir_all(dir.cache()).unwrap();
    std::fs::write(
        dir.cache().join("harness-versions.json"),
        doc(3, "3.0.0", json!([])),
    )
    .unwrap();
    let (service, _) = self::service(
        &dir,
        ManifestPolicy::Auto,
        FakeFetcher::new(vec![]),
        FakeClock::at(0),
    );
    assert_eq!(&*service.current(), embedded());
}

#[test]
fn not_modified_keeps_cache_and_updates_fetched_at() {
    let dir = TestDir::new();
    let clock = FakeClock::at(DAY_MS);
    let fetcher = FakeFetcher::new(vec![
        body(doc(2, "1.1.1", json!([row("1.0.0")])), Some("\"e1\"")),
        Ok(FetchOutcome::NotModified),
    ]);
    let (service, _) = service(&dir, ManifestPolicy::Auto, fetcher.clone(), clock.clone());
    service.ensure_manifest("claude", FetchReason::FreshViolation);
    assert!(service.wait_idle(WAIT));
    clock.advance(DAY_MS + 5);
    service.ensure_manifest("claude", FetchReason::FreshViolation);
    assert!(service.wait_idle(WAIT));
    let calls = fetcher.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].1, None);
    assert_eq!(calls[1].1.as_deref(), Some("\"e1\""), "stored ETag is sent");
    assert_eq!(service.current().latest_release.as_deref(), Some("1.1.1"));
    let meta = read_meta(&dir.cache());
    assert_eq!(meta.etag.as_deref(), Some("\"e1\""));
    assert_eq!(meta.fetched_at_ms, Some(2 * DAY_MS + 5));
}

#[test]
fn etag_is_not_sent_without_a_valid_cache() {
    let dir = TestDir::new();
    std::fs::create_dir_all(dir.cache()).unwrap();
    std::fs::write(
        dir.cache().join("meta.json"),
        br#"{"etag":"\"stale\"","fetched_at_ms":1,"attempts":{}}"#,
    )
    .unwrap();
    let fetcher = FakeFetcher::new(vec![Err(FetchError::Failed("x".into()))]);
    let (service, _) = service(
        &dir,
        ManifestPolicy::Auto,
        fetcher.clone(),
        FakeClock::at(DAY_MS),
    );
    service.ensure_manifest("codex", FetchReason::FreshViolation);
    assert!(service.wait_idle(WAIT));
    assert_eq!(fetcher.calls()[0].1, None);
}

#[test]
fn schema_3_fetch_leaves_the_valid_cache_in_place() {
    let dir = TestDir::new();
    let clock = FakeClock::at(DAY_MS);
    let fetcher = FakeFetcher::new(vec![
        body(doc(2, "2.0.0", json!([row("2.0.0")])), Some("\"good\"")),
        body(doc(3, "3.0.0", json!([row("3.0.0")])), Some("\"future\"")),
    ]);
    let (service, lines) = service(&dir, ManifestPolicy::Auto, fetcher, clock.clone());
    service.ensure_manifest("claude", FetchReason::FreshViolation);
    assert!(service.wait_idle(WAIT));
    clock.advance(DAY_MS);
    service.ensure_manifest("claude", FetchReason::FreshViolation);
    assert!(service.wait_idle(WAIT));
    assert_eq!(service.current().latest_release.as_deref(), Some("2.0.0"));
    let on_disk = parse(&std::fs::read(dir.cache().join("harness-versions.json")).unwrap());
    assert_eq!(on_disk.unwrap().latest_release.as_deref(), Some("2.0.0"));
    assert_eq!(
        read_meta(&dir.cache()).etag.as_deref(),
        Some("\"good\""),
        "the rejected body's ETag is not recorded"
    );
    let lines = lines.lock().unwrap();
    assert!(
        lines.iter().any(|line| line.contains("schema_version 3")),
        "{lines:?}"
    );
}

#[test]
fn oversized_fetch_is_discarded() {
    let dir = TestDir::new();
    let mut big = doc(2, "9.9.9", json!([]));
    big.resize(MAX_MANIFEST_BYTES + 1, b' ');
    let fetcher = FakeFetcher::new(vec![body(big, None)]);
    let (service, lines) = service(&dir, ManifestPolicy::Auto, fetcher, FakeClock::at(DAY_MS));
    service.ensure_manifest("claude", FetchReason::FreshViolation);
    assert!(service.wait_idle(WAIT));
    assert!(!dir.cache().join("harness-versions.json").exists());
    assert_eq!(&*service.current(), embedded());
    assert_eq!(lines.lock().unwrap().len(), 1);

    // A fetcher-side TooLarge is the same.
    let dir = TestDir::new();
    let (service, lines) = self::service(
        &dir,
        ManifestPolicy::Auto,
        FakeFetcher::new(vec![Err(FetchError::TooLarge)]),
        FakeClock::at(DAY_MS),
    );
    service.ensure_manifest("claude", FetchReason::FreshViolation);
    assert!(service.wait_idle(WAIT));
    assert_eq!(&*service.current(), embedded());
    assert_eq!(lines.lock().unwrap().len(), 1);
}

#[test]
fn offline_fetcher_logs_and_falls_back() {
    let dir = TestDir::new();
    let (service, lines) = service(
        &dir,
        ManifestPolicy::Auto,
        FakeFetcher::new(vec![Err(FetchError::Offline)]),
        FakeClock::at(DAY_MS),
    );
    service.ensure_manifest("codex", FetchReason::FreshViolation);
    assert!(service.wait_idle(WAIT));
    assert_eq!(&*service.current(), embedded());
    let lines = lines.lock().unwrap();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("curl"), "{lines:?}");
    // The failed attempt still counts against the daily budget.
    assert_eq!(read_meta(&dir.cache()).attempts["codex"], DAY_MS);
}

#[test]
fn triggers_once_a_day_and_only_on_unseen_or_violation() {
    let dir = TestDir::new();
    let clock = FakeClock::at(3 * DAY_MS);
    let fetcher = FakeFetcher::new(vec![
        Err(FetchError::Failed("e1".into())),
        Err(FetchError::Failed("e2".into())),
        Err(FetchError::Failed("e3".into())),
    ]);
    let (service, _) = service(&dir, ManifestPolicy::Auto, fetcher.clone(), clock.clone());
    // A version the embedded copy already has: no fetch.
    service.ensure_manifest("claude", unseen("2.1.283"));
    assert!(service.wait_idle(WAIT));
    assert_eq!(fetcher.calls().len(), 0);
    // A version nothing has: fetch; a repeat within the day: none.
    service.ensure_manifest("claude", unseen("2.1.999"));
    assert!(service.wait_idle(WAIT));
    service.ensure_manifest("claude", FetchReason::FreshViolation);
    service.ensure_manifest("claude", unseen("2.1.1000"));
    assert!(service.wait_idle(WAIT));
    assert_eq!(fetcher.calls().len(), 1);
    // Another harness has its own budget.
    service.ensure_manifest("codex", FetchReason::FreshViolation);
    assert!(service.wait_idle(WAIT));
    assert_eq!(fetcher.calls().len(), 2);
    // A day later claude may fetch again.
    clock.advance(DAY_MS);
    service.ensure_manifest("claude", FetchReason::FreshViolation);
    assert!(service.wait_idle(WAIT));
    assert_eq!(fetcher.calls().len(), 3);
}

#[test]
fn opt_out_never_fetches() {
    for off in [OffReason::Settings, OffReason::OfflineEnv] {
        let dir = TestDir::new();
        let fetcher = FakeFetcher::new(vec![body(doc(2, "1.0.0", json!([])), None)]);
        let (service, _) = service(
            &dir,
            ManifestPolicy::Off(off),
            fetcher.clone(),
            FakeClock::at(DAY_MS),
        );
        service.ensure_manifest("claude", FetchReason::FreshViolation);
        service.ensure_manifest("codex", unseen("0.999.0"));
        assert!(service.wait_idle(WAIT));
        assert!(fetcher.calls().is_empty(), "{off:?}");
        assert_eq!(&*service.current(), embedded());
    }
}

#[test]
fn ensure_manifest_returns_within_10ms_with_a_blocked_fetch() {
    let dir = TestDir::new();
    let fetcher = FakeFetcher::gated(vec![Err(FetchError::Failed("unreachable".into()))]);
    let (service, _) = service(
        &dir,
        ManifestPolicy::Auto,
        fetcher.clone(),
        FakeClock::at(DAY_MS),
    );
    let started = Instant::now();
    service.ensure_manifest("claude", unseen("9.9.9"));
    let took = started.elapsed();
    assert!(took < Duration::from_millis(10), "{took:?}");
    fetcher.release();
    assert!(service.wait_idle(WAIT));
}

fn curl_on_path() -> bool {
    CurlFetcher::new(std::env::temp_dir())
        .fetch("file:///nonexistent", None)
        .map_or_else(|error| error != FetchError::Offline, |_| true)
}

#[test]
fn ensure_manifest_returns_within_10ms_with_an_unreachable_url() {
    if !curl_on_path() {
        eprintln!("curl not on PATH: the real-curl unreachable-URL check did not run");
        return;
    }
    // Port 9 (discard) on loopback refuses the connection; the call must not
    // wait for it. The best of three fresh services shields the bound from a
    // scheduler hiccup on a loaded machine.
    let mut best = Duration::MAX;
    for _ in 0..3 {
        let dir = TestDir::new();
        let lines: Lines = Arc::default();
        let sink = Arc::clone(&lines);
        std::fs::create_dir_all(dir.cache()).unwrap();
        let service = ManifestService::new(
            dir.cache(),
            ManifestPolicy::Auto,
            Arc::new(UnreachableCurl(CurlFetcher::new(dir.cache()))),
            FakeClock::at(DAY_MS),
            Arc::new(move |line: &str| sink.lock().unwrap().push(line.to_owned())),
        );
        let started = Instant::now();
        service.ensure_manifest("claude", unseen("9.9.9"));
        best = best.min(started.elapsed());
        assert!(service.wait_idle(WAIT));
        let lines = lines.lock().unwrap();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("fetch failed"), "{lines:?}");
        assert_eq!(&*service.current(), embedded());
    }
    assert!(best < Duration::from_millis(10), "{best:?}");
}

/// A real `curl` fetch of an unreachable loopback URL, whatever URL the
/// service asks for.
struct UnreachableCurl(CurlFetcher);

impl Fetcher for UnreachableCurl {
    fn fetch(&self, _url: &str, etag: Option<&str>) -> Result<FetchOutcome, FetchError> {
        self.0
            .fetch("http://127.0.0.1:9/harness-versions.json", etag)
    }
}

#[test]
fn concurrent_fetches_for_two_harnesses_are_both_recorded() {
    let dir = TestDir::new();
    let fetcher = FakeFetcher::gated(vec![body(doc(2, "1.0.0", json!([])), None)]);
    let (service, _) = service(
        &dir,
        ManifestPolicy::Auto,
        fetcher.clone(),
        FakeClock::at(DAY_MS),
    );
    service.ensure_manifest("claude", FetchReason::FreshViolation);
    service.ensure_manifest("codex", FetchReason::FreshViolation);
    service.ensure_manifest("claude", unseen("9.9.9"));
    // Let the one fetch thread reach the fetcher before releasing it.
    let deadline = Instant::now() + WAIT;
    while fetcher.calls().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    fetcher.release();
    assert!(service.wait_idle(WAIT));
    // Kills: a dropped call that left codex free to refetch at once, or an
    // attempt that was never persisted. The third call is skipped: claude
    // already attempted.
    assert_eq!(fetcher.calls().len(), 1);
    let attempts = read_meta(&dir.cache()).attempts;
    assert!(attempts.contains_key("claude"), "{attempts:?}");
    assert!(attempts.contains_key("codex"), "{attempts:?}");
}

#[test]
fn older_cache_yields_to_a_newer_embedded_copy() {
    let cache = parse(&doc_at(2, "1.0.0", Some("2026-01-01T00:00:00Z"), json!([]))).unwrap();
    let emb = parse(&doc_at(2, "2.0.0", Some("2026-06-01T00:00:00Z"), json!([]))).unwrap();
    let picked = prefer_newer(Some(cache), &emb);
    assert_eq!(picked.generated_at.as_deref(), Some("2026-06-01T00:00:00Z"));
    assert_eq!(picked.latest_release.as_deref(), Some("2.0.0"));
}

#[test]
fn newer_cache_wins() {
    let cache = parse(&doc_at(2, "1.0.0", Some("2026-07-01T00:00:00Z"), json!([]))).unwrap();
    let emb = parse(&doc_at(2, "2.0.0", Some("2026-06-01T00:00:00Z"), json!([]))).unwrap();
    let picked = prefer_newer(Some(cache), &emb);
    assert_eq!(picked.latest_release.as_deref(), Some("1.0.0"));
}

#[test]
fn cache_without_generated_at_wins() {
    let cache = parse(&doc_at(2, "1.0.0", None, json!([]))).unwrap();
    let emb = parse(&doc_at(2, "2.0.0", Some("2026-06-01T00:00:00Z"), json!([]))).unwrap();
    let picked = prefer_newer(Some(cache), &emb);
    assert_eq!(picked.latest_release.as_deref(), Some("1.0.0"));
}

#[test]
fn no_cache_yields_embedded() {
    let emb = parse(&doc_at(2, "2.0.0", Some("2026-06-01T00:00:00Z"), json!([]))).unwrap();
    assert_eq!(prefer_newer(None, &emb), emb);
}

#[test]
fn service_records_its_policy_at_start() {
    let dir = TestDir::new();
    let (_service, _) = service(
        &dir,
        ManifestPolicy::Off(OffReason::Settings),
        FakeFetcher::new(vec![]),
        FakeClock::at(7 * DAY_MS),
    );
    let recorded = read_meta(&dir.cache()).daemon_policy.expect("recorded");
    assert_eq!(recorded.policy, "off");
    assert_eq!(recorded.source, "settings");
    assert_eq!(recorded.recorded_at_ms, 7 * DAY_MS);
}

// -- curl -------------------------------------------------------------------

#[test]
fn curl_missing_is_offline() {
    let dir = TestDir::new();
    let fetcher = CurlFetcher::with_path(dir.path().to_owned(), OsString::new());
    assert_eq!(
        fetcher.fetch("file:///etc/hosts", None),
        Err(FetchError::Offline)
    );
    // A PATH naming only a directory without curl is the same.
    let fetcher = CurlFetcher::with_path(dir.path().to_owned(), dir.path().as_os_str().to_owned());
    assert_eq!(
        fetcher.fetch("file:///etc/hosts", None),
        Err(FetchError::Offline)
    );
}

#[test]
fn curl_fetches_a_file_url_and_cleans_up() {
    if !curl_on_path() {
        eprintln!("curl not on PATH: the real-curl file fetch did not run");
        return;
    }
    let dir = TestDir::new();
    let source = dir.path().join("source.json");
    let expected = doc(2, "4.4.4", json!([row("4.4.4")]));
    std::fs::write(&source, &expected).unwrap();
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let fetcher = CurlFetcher::new(work.clone());
    let url = format!("file://{}", source.display());
    assert_eq!(
        fetcher.fetch(&url, None),
        Ok(FetchOutcome::Body {
            bytes: expected,
            etag: None
        })
    );
    assert_eq!(
        std::fs::read_dir(&work).unwrap().count(),
        0,
        "temp files removed"
    );
    match fetcher.fetch(
        &format!("file://{}/missing.json", dir.path().display()),
        None,
    ) {
        Err(FetchError::Failed(_)) => (),
        other => panic!("{other:?}"),
    }
}

/// A one-shot HTTP server on loopback: answers `responses.len()` requests and
/// returns the raw request heads it saw.
fn serve(responses: Vec<String>) -> (String, std::thread::JoinHandle<Vec<String>>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!(
        "http://{}/harness-versions.json",
        listener.local_addr().unwrap()
    );
    let handle = std::thread::spawn(move || {
        let mut seen = Vec::new();
        for response in responses {
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(_) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(2))
                    }
                    Err(error) => panic!("no request arrived: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                if stream.read(&mut byte).unwrap_or(0) == 0 {
                    break;
                }
                head.push(byte[0]);
            }
            seen.push(String::from_utf8_lossy(&head).into_owned());
            stream.write_all(response.as_bytes()).unwrap();
        }
        seen
    });
    (url, handle)
}

#[test]
fn curl_http_etag_roundtrip_sends_nothing_about_the_user() {
    if !curl_on_path() {
        eprintln!("curl not on PATH: the real-curl HTTP check did not run");
        return;
    }
    let payload = String::from_utf8(doc(2, "5.5.5", json!([row("5.5.5")]))).unwrap();
    let ok = format!(
        "HTTP/1.1 200 OK\r\nETag: W/\"abc123\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    let not_modified = "HTTP/1.1 304 Not Modified\r\nConnection: close\r\n\r\n".to_owned();
    let (url, server) = serve(vec![ok, not_modified]);
    let dir = TestDir::new();
    let fetcher = CurlFetcher::new(dir.path().to_owned());
    let first = fetcher.fetch(&url, None).unwrap();
    let FetchOutcome::Body { bytes, etag } = first else {
        panic!("expected a body");
    };
    assert_eq!(bytes, payload.as_bytes());
    assert_eq!(etag.as_deref(), Some("W/\"abc123\""));
    assert_eq!(
        fetcher.fetch(&url, etag.as_deref()),
        Ok(FetchOutcome::NotModified)
    );
    let heads = server.join().unwrap();
    assert!(
        !heads[0].to_ascii_lowercase().contains("if-none-match"),
        "{}",
        heads[0]
    );
    assert!(
        heads[1].contains("If-None-Match: W/\"abc123\""),
        "{}",
        heads[1]
    );
    for head in &heads {
        for line in head.lines().skip(1).filter(|line| !line.is_empty()) {
            let name = line.split(':').next().unwrap().to_ascii_lowercase();
            assert!(
                ["host", "user-agent", "accept", "if-none-match"].contains(&name.as_str()),
                "unexpected request header {line:?}"
            );
        }
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn curl_http_error_is_failed() {
    if !curl_on_path() {
        eprintln!("curl not on PATH: the real-curl HTTP error check did not run");
        return;
    }
    let (url, server) = serve(vec![
        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(),
    ]);
    let dir = TestDir::new();
    match CurlFetcher::new(dir.path().to_owned()).fetch(&url, None) {
        Err(FetchError::Failed(_)) => (),
        other => panic!("{other:?}"),
    }
    server.join().unwrap();
}

#[test]
fn rfc3339_formatting() {
    assert_eq!(format_rfc3339_utc(0), "1970-01-01T00:00:00Z");
    assert_eq!(format_rfc3339_utc(951_782_400_000), "2000-02-29T00:00:00Z");
    assert_eq!(
        format_rfc3339_utc(1_790_000_123_999),
        "2026-09-21T14:15:23Z"
    );
}

/// An invalid `settings.json` refuses the daemon start, before the store is
/// opened or any stream is redirected, naming the file and the bad key.
#[test]
fn daemon_start_refuses_an_invalid_settings_file() {
    use crate::app::{LaneProbe, SystemClock, run_elected_probed};
    use crate::daemon::paths::{InstancePaths, RuntimeContext};
    use crate::host::native::NativeCli;
    use crate::protocol::time::Cancellation;
    use crate::service::config::ServiceConfig;

    let dir = TestDir::new();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let endpoint = dir.path().join("herdr.sock");
    let context = RuntimeContext::explicit(state, endpoint.clone(), None).unwrap();
    let paths = InstancePaths::resolve(&context).unwrap();
    std::fs::create_dir_all(&paths.instance_dir).unwrap();
    std::fs::write(
        paths.instance_dir.join("settings.json"),
        br#"{"harness_manifest": "off", "surprise": true}"#,
    )
    .unwrap();
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let host = Arc::new(NativeCli::new(endpoint, Arc::clone(&clock)));
    let error = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(run_elected_probed(
            &paths,
            clock,
            Cancellation::default(),
            ServiceConfig::default(),
            host,
            LaneProbe::default(),
            |_| Ok(()),
        ))
        .expect_err("an invalid settings file must refuse the start");
    let text = error.to_string();
    assert!(text.contains("settings.json"), "{text}");
    assert!(text.contains("unknown field `surprise`"), "{text}");
    assert!(!paths.descriptor_path.exists());
    assert!(!paths.database_path.exists());
}

#[test]
fn exact_runtime_refresh_ignores_legacy_release_rows_and_remains_async_bounded() {
    let identity =
        crate::harness::runtime::RuntimeIdentity::stable_release("2.1.286", "native_transcript")
            .unwrap();
    let reason = FetchReason::UnseenRuntime {
        identity,
        domain: "native_payload".into(),
        origin: crate::harness::evidence::EvidenceOrigin::NativePayload,
        contract_id: "0123456789abcdef".into(),
    };
    assert_eq!(
        should_fetch(
            ManifestPolicy::Auto,
            &reason,
            "claude",
            DAY_MS,
            &CacheMeta::default(),
            true
        ),
        Decision::Fetch,
        "a legacy release row cannot satisfy an exact-domain lookup"
    );
    let dir = TestDir::new();
    let fetcher = FakeFetcher::gated(vec![Err(FetchError::Failed("fixture blocked".into()))]);
    let (service, _) = service(
        &dir,
        ManifestPolicy::Auto,
        fetcher.clone(),
        FakeClock::at(DAY_MS),
    );
    let start = Instant::now();
    service.ensure_manifest("claude", reason.clone());
    let took = start.elapsed();
    service.ensure_manifest("claude", reason);
    fetcher.release();
    assert!(service.wait_idle(WAIT));
    assert!(
        took < Duration::from_millis(100),
        "exact-domain refresh blocked request: {took:?}"
    );
    assert_eq!(
        fetcher.calls().len(),
        1,
        "same bounded per-harness retry spacing"
    );
}

#[test]
fn exact_runtime_refresh_refuses_invalid_descriptor_before_scheduling() {
    let mut identity =
        crate::harness::runtime::RuntimeIdentity::stable_release("2.1.286", "native_transcript")
            .unwrap();
    identity.key = "release:9.9.9".into();
    let reason = FetchReason::UnseenRuntime {
        identity,
        domain: "native_payload".into(),
        origin: crate::harness::evidence::EvidenceOrigin::NativePayload,
        contract_id: "0123456789abcdef".into(),
    };
    assert!(
        matches!(
            should_fetch(
                ManifestPolicy::Auto,
                &reason,
                "claude",
                DAY_MS,
                &CacheMeta::default(),
                false
            ),
            Decision::Skip(_)
        ),
        "invalid rich identity must never start fetch"
    );
}
