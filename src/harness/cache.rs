//! Immutable identifiers for locally cached CheckIn output.
use crate::{
    harness::context::{
        CheckInMode, CheckInResponse, OccupantContext, PendingCheckIn, Role, SessionReference,
    },
    protocol::{
        output::{ContinuationContext, OutputFormat, OutputSpec, encode_selected},
        results::{ApiError, CommandResult, ErrorCode},
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CachedCheckInKey {
    pub instance: Uuid,
    pub seat: String,
    pub event_id: String,
    pub operation_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedCheckIn {
    pub request: PendingCheckIn,
    pub response: CheckInResponse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheRefV1 {
    pub version: u32,
    pub key: CachedCheckInKey,
    pub request_sha256: String,
    pub output_sha256: String,
    pub response_meta_sha256: String,
    pub selectors_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheCursorV1 {
    pub version: u32,
    pub reference_sha256: [u8; 32],
    pub offset: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachePageRequest {
    pub reference: CacheRefV1,
    pub cursor: Option<CacheCursorV1>,
    pub max_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CachedCheckInPage {
    pub reference: String,
    pub output_sha256: String,
    pub cached: bool,
    pub historical: bool,
    pub start: u32,
    pub end: u32,
    pub total: u32,
    pub chunk_data: String,
    pub next_argv: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheReadError {
    Context(crate::harness::context::ContextError),
    NotFound,
    ReferenceMismatch,
    InvalidCursor,
    UnsupportedVersion,
    InvalidBudget(u32),
    Cancelled,
    DeadlineExceeded,
}

impl From<crate::harness::context::ContextError> for CacheReadError {
    fn from(error: crate::harness::context::ContextError) -> Self {
        Self::Context(error)
    }
}
impl From<std::io::Error> for CacheReadError {
    fn from(error: std::io::Error) -> Self {
        Self::Context(crate::harness::context::ContextError::Io(error.to_string()))
    }
}

fn hash(tag: &str, parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(tag.as_bytes());
    h.update([0]);
    for part in parts {
        h.update((part.len() as u64).to_be_bytes());
        h.update(part);
    }
    h.finalize().into()
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 15) as usize] as char);
    }
    out
}

fn context_tuple(c: &OccupantContext) -> serde_json::Value {
    let session = match &c.session {
        SessionReference::Native(value) => serde_json::json!(["native", value]),
        SessionReference::PluginContext(id) => serde_json::json!(["plugin_context", id]),
    };
    let harness = c.harness.as_str();
    let role = match c.role {
        Role::TopLevel => "top_level",
        Role::Subagent => "subagent",
    };
    serde_json::json!([
        c.format_version,
        c.instance,
        c.seat,
        c.target,
        harness,
        c.binding_generation,
        c.execution,
        session,
        role
    ])
}

pub fn cache_reference(
    saved: &CachedCheckIn,
    selectors: &ContinuationContext,
) -> Result<CacheRefV1, CacheReadError> {
    let req = &saved.request;
    let resp = &saved.response;
    if req.operation_id.is_nil()
        || req.context.instance.is_nil()
        || req.event_id.is_empty()
        || resp.output.len() > 65_536
        || std::str::from_utf8(&resp.output).is_err()
    {
        return Err(CacheReadError::ReferenceMismatch);
    }
    let mode = match req.mode {
        CheckInMode::Current => "current",
        CheckInMode::Lifecycle => "lifecycle",
    };
    let req_meta = serde_json::json!([
        req.payload_version,
        req.operation_id,
        mode,
        req.expected_generation,
        req.event_id,
        context_tuple(&req.context)
    ]);
    let response_meta = serde_json::json!([context_tuple(&resp.context), resp.historical]);
    let selectors_meta = serde_json::json!([selectors.state_dir, selectors.host.as_deref()]);
    let req_bytes = serde_json::to_vec(&req_meta).map_err(|_| CacheReadError::ReferenceMismatch)?;
    let response_bytes =
        serde_json::to_vec(&response_meta).map_err(|_| CacheReadError::ReferenceMismatch)?;
    let selectors_bytes =
        serde_json::to_vec(&selectors_meta).map_err(|_| CacheReadError::ReferenceMismatch)?;
    Ok(CacheRefV1 {
        version: 1,
        key: CachedCheckInKey {
            instance: req.context.instance,
            seat: req.context.seat.clone(),
            event_id: req.event_id.clone(),
            operation_id: req.operation_id,
        },
        request_sha256: hex(&hash(
            "herdr-threads/cache/request/v1",
            &[&req.payload, &req_bytes],
        )),
        output_sha256: hex(&hash("herdr-threads/cache/output/v1", &[&resp.output])),
        response_meta_sha256: hex(&hash(
            "herdr-threads/cache/response-meta/v1",
            &[&response_bytes],
        )),
        selectors_sha256: hex(&hash(
            "herdr-threads/cache/selectors/v1",
            &[&selectors_bytes],
        )),
    })
}

fn b64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity((data.len() * 4).div_ceil(3));
    for chunk in data.chunks(3) {
        let a = chunk[0];
        let b = *chunk.get(1).unwrap_or(&0);
        let c = *chunk.get(2).unwrap_or(&0);
        out.push(TABLE[(a >> 2) as usize] as char);
        out.push(TABLE[(((a & 3) << 4) | (b >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(((b & 15) << 2) | (c >> 6)) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(TABLE[(c & 63) as usize] as char);
        }
    }
    out
}

fn b64_decode(value: &str) -> Option<Vec<u8>> {
    if value.len() % 4 == 1 {
        return None;
    }
    let digit = |b: u8| -> Option<u8> {
        match b {
            b'A'..=b'Z' => Some(b - b'A'),
            b'a'..=b'z' => Some(b - b'a' + 26),
            b'0'..=b'9' => Some(b - b'0' + 52),
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    };
    let mut out = Vec::with_capacity(value.len() * 3 / 4);
    for chunk in value.as_bytes().chunks(4) {
        let a = digit(chunk[0])?;
        let b = digit(chunk[1])?;
        out.push((a << 2) | (b >> 4));
        if chunk.len() > 2 {
            let c = digit(chunk[2])?;
            out.push((b << 4) | (c >> 2));
            if chunk.len() > 3 {
                let d = digit(chunk[3])?;
                out.push((c << 6) | d);
            }
        }
    }
    (b64_encode(&out) == value).then_some(out)
}

impl CacheRefV1 {
    pub fn token(&self) -> Result<String, CacheReadError> {
        if self.version != 1 {
            return Err(CacheReadError::UnsupportedVersion);
        }
        let bytes = serde_json::to_vec(self).map_err(|_| CacheReadError::ReferenceMismatch)?;
        let token = format!("cc1.{}", b64_encode(&bytes));
        if token.len() > 8192 {
            return Err(CacheReadError::ReferenceMismatch);
        }
        Ok(token)
    }
    pub fn parse(token: &str) -> Result<Self, CacheReadError> {
        if token.len() > 8192 {
            return Err(CacheReadError::ReferenceMismatch);
        }
        let encoded = token
            .strip_prefix("cc1.")
            .ok_or(CacheReadError::ReferenceMismatch)?;
        let bytes = b64_decode(encoded).ok_or(CacheReadError::ReferenceMismatch)?;
        let value: Self =
            serde_json::from_slice(&bytes).map_err(|_| CacheReadError::ReferenceMismatch)?;
        if value.version != 1 {
            return Err(CacheReadError::UnsupportedVersion);
        }
        let valid_text = |text: &str| {
            !text.is_empty() && text.len() <= 1024 && !text.chars().any(char::is_control)
        };
        let valid_digest = |digest: &str| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        };
        if value.key.instance.is_nil()
            || value.key.operation_id.is_nil()
            || !valid_text(&value.key.seat)
            || !valid_text(&value.key.event_id)
            || !valid_digest(&value.request_sha256)
            || !valid_digest(&value.output_sha256)
            || !valid_digest(&value.response_meta_sha256)
            || !valid_digest(&value.selectors_sha256)
        {
            return Err(CacheReadError::ReferenceMismatch);
        }
        if value.token()? != token {
            return Err(CacheReadError::ReferenceMismatch);
        }
        Ok(value)
    }
    pub fn digest(&self) -> Result<[u8; 32], CacheReadError> {
        let bytes = serde_json::to_vec(self).map_err(|_| CacheReadError::ReferenceMismatch)?;
        Ok(hash("herdr-threads/cache/reference/v1", &[&bytes]))
    }
}

impl CacheCursorV1 {
    pub fn token(&self) -> Result<String, CacheReadError> {
        if self.version != 1 {
            return Err(CacheReadError::UnsupportedVersion);
        }
        let bytes = serde_json::to_vec(self).map_err(|_| CacheReadError::InvalidCursor)?;
        let token = format!("ccp1.{}", b64_encode(&bytes));
        if token.len() > 256 {
            return Err(CacheReadError::InvalidCursor);
        }
        Ok(token)
    }
    pub fn parse(token: &str) -> Result<Self, CacheReadError> {
        if token.len() > 256 {
            return Err(CacheReadError::InvalidCursor);
        }
        let encoded = token
            .strip_prefix("ccp1.")
            .ok_or(CacheReadError::InvalidCursor)?;
        let bytes = b64_decode(encoded).ok_or(CacheReadError::InvalidCursor)?;
        let value: Self =
            serde_json::from_slice(&bytes).map_err(|_| CacheReadError::InvalidCursor)?;
        if value.version != 1 {
            return Err(CacheReadError::UnsupportedVersion);
        }
        if value.token()? != token {
            return Err(CacheReadError::InvalidCursor);
        }
        Ok(value)
    }
}

pub(crate) fn page_error(error: CacheReadError) -> ApiError {
    let (code, required_minimum_bytes) = match error {
        CacheReadError::InvalidBudget(size) => (ErrorCode::InvalidBudget, Some(size)),
        CacheReadError::InvalidCursor => (ErrorCode::InvalidCursor, None),
        CacheReadError::NotFound => (ErrorCode::NotFound, None),
        CacheReadError::Cancelled | CacheReadError::DeadlineExceeded => {
            (ErrorCode::ReadBudgetExhausted, None)
        }
        CacheReadError::Context(crate::harness::context::ContextError::LockTimeout) => {
            (ErrorCode::StoreBusy, None)
        }
        CacheReadError::Context(crate::harness::context::ContextError::Corrupt) => {
            (ErrorCode::StoreCorrupt, None)
        }
        _ => (ErrorCode::InvalidRequest, None),
    };
    let mut api_error = ApiError::new(code, format!("cached CheckIn page: {error:?}"));
    api_error.required_minimum_bytes = required_minimum_bytes;
    api_error
}

fn page_argv(
    spec: &OutputSpec,
    reference: &str,
    max_bytes: u32,
    cursor: CacheCursorV1,
) -> Result<Vec<String>, CacheReadError> {
    let mut argv = vec!["herdr-threads".to_string()];
    if let Some(state) = &spec.context.state_dir {
        argv.extend(["--state-dir".into(), state.clone()]);
    }
    if let Some(host) = &spec.context.host {
        argv.extend(["--host-endpoint".into(), host.as_str().into()]);
    }
    if spec.format == OutputFormat::Json {
        argv.push("--json".into());
    }
    argv.extend([
        "cached-check-in".into(),
        "--reference".into(),
        reference.into(),
        "--max-bytes".into(),
        max_bytes.to_string(),
        "--cursor".into(),
        cursor.token()?,
    ]);
    Ok(argv)
}

/// Fit the exact selected representation of one immutable UTF-8 fragment.
pub fn cached_output_page(
    saved: &CachedCheckIn,
    request: &CachePageRequest,
    output: &OutputSpec,
) -> Result<CommandResult, ApiError> {
    let result = (|| -> Result<CommandResult, CacheReadError> {
        if !(256..=65_536).contains(&request.max_bytes) {
            return Err(CacheReadError::InvalidBudget(256));
        }
        if output
            .context
            .state_dir
            .as_ref()
            .is_some_and(|s| s.len() > 1024 || s.is_empty())
            || output
                .context
                .host
                .as_ref()
                .is_some_and(|h| h.as_str().len() > 1024)
        {
            return Err(CacheReadError::ReferenceMismatch);
        }
        let expected = cache_reference(saved, &output.context)?;
        if request.reference != expected {
            return Err(CacheReadError::ReferenceMismatch);
        }
        let reference = request.reference.token()?;
        let digest = request.reference.digest()?;
        let source = std::str::from_utf8(&saved.response.output)
            .map_err(|_| CacheReadError::ReferenceMismatch)?;
        let start = request
            .cursor
            .as_ref()
            .map_or(0, |cursor| cursor.offset as usize);
        if let Some(cursor) = &request.cursor {
            if cursor.version != 1 {
                return Err(CacheReadError::UnsupportedVersion);
            }
            if cursor.reference_sha256 != digest {
                return Err(CacheReadError::InvalidCursor);
            }
        }
        if start > source.len() || !source.is_char_boundary(start) {
            return Err(CacheReadError::InvalidCursor);
        }
        let make = |end: usize| -> Result<CommandResult, CacheReadError> {
            let next_argv = if end == source.len() {
                None
            } else {
                Some(page_argv(
                    output,
                    &reference,
                    request.max_bytes,
                    CacheCursorV1 {
                        version: 1,
                        reference_sha256: digest,
                        offset: end as u32,
                    },
                )?)
            };
            Ok(CommandResult::CachedCheckInPage(CachedCheckInPage {
                reference: reference.clone(),
                output_sha256: expected.output_sha256.clone(),
                cached: true,
                historical: saved.response.historical,
                start: start as u32,
                end: end as u32,
                total: source.len() as u32,
                chunk_data: source[start..end].into(),
                next_argv,
            }))
        };
        let size = |end: usize| -> Result<usize, CacheReadError> {
            let page = make(end)?;
            encode_selected(&page, output)
                .map(|b| b.len())
                .map_err(|_| CacheReadError::ReferenceMismatch)
        };
        let full_size = size(source.len())?;
        if full_size <= request.max_bytes as usize {
            return make(source.len());
        }
        if start == source.len() {
            return Err(CacheReadError::InvalidBudget(full_size as u32));
        }
        let boundaries: Vec<usize> = source[start..]
            .char_indices()
            .skip(1)
            .map(|(i, _)| start + i)
            .chain(std::iter::once(source.len()))
            .collect();
        let first = *boundaries.first().ok_or(CacheReadError::InvalidCursor)?;
        let minimum = size(first)?;
        if minimum > request.max_bytes as usize {
            return Err(CacheReadError::InvalidBudget(minimum as u32));
        }
        let mut low = 0usize;
        let mut high = boundaries.len();
        while low + 1 < high {
            let mid = low + (high - low) / 2;
            if size(boundaries[mid])? <= request.max_bytes as usize {
                low = mid;
            } else {
                high = mid;
            }
        }
        let chosen = boundaries[low];
        let page = make(chosen)?;
        if encode_selected(&page, output)
            .map_err(|_| CacheReadError::ReferenceMismatch)?
            .len()
            > request.max_bytes as usize
        {
            return Err(CacheReadError::InvalidBudget(size(chosen)? as u32));
        }
        Ok(page)
    })();
    result.map_err(page_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{output::OutputSpec, results::CommandResult};

    #[test]
    fn reference_and_cursor_tokens_require_canonical_bytes() {
        let reference = CacheRefV1 {
            version: 1,
            key: CachedCheckInKey {
                instance: Uuid::from_u128(1),
                seat: "seat".into(),
                event_id: "event".into(),
                operation_id: Uuid::from_u128(2),
            },
            request_sha256: "a".repeat(64),
            output_sha256: "b".repeat(64),
            response_meta_sha256: "c".repeat(64),
            selectors_sha256: "d".repeat(64),
        };
        let token = reference.token().unwrap();
        assert_eq!(CacheRefV1::parse(&token).unwrap(), reference);
        let mut uppercase = reference.clone();
        uppercase.request_sha256 = "A".repeat(64);
        assert_eq!(
            CacheRefV1::parse(&uppercase.token().unwrap()).unwrap_err(),
            CacheReadError::ReferenceMismatch
        );
        assert_eq!(
            CacheRefV1::parse(&format!("{token}=")).unwrap_err(),
            CacheReadError::ReferenceMismatch
        );
        let cursor = CacheCursorV1 {
            version: 1,
            reference_sha256: reference.digest().unwrap(),
            offset: 42,
        };
        let token = cursor.token().unwrap();
        assert_eq!(CacheCursorV1::parse(&token).unwrap(), cursor);
        assert_eq!(
            CacheCursorV1::parse(&format!("{token}=")).unwrap_err(),
            CacheReadError::InvalidCursor
        );
    }

    #[test]
    fn near_limit_pages_reconstruct_exact_utf8_bytes() {
        let context = OccupantContext {
            format_version: 1,
            instance: Uuid::from_u128(1),
            seat: "seat".into(),
            target: "target".into(),
            harness: crate::harness::context::Harness::Codex,
            binding_generation: 1,
            execution: Uuid::from_u128(3),
            session: SessionReference::PluginContext(Uuid::from_u128(3)),
            role: Role::TopLevel,
        };
        let original = format!("{}🧭\u{001b}\u{0085}\u{2028}\n", "é".repeat(32_000)).into_bytes();
        let saved = CachedCheckIn {
            request: PendingCheckIn {
                operation_id: Uuid::from_u128(2),
                mode: CheckInMode::Current,
                context: context.clone(),
                expected_generation: None,
                event_id: "event".into(),
                payload_version: 1,
                payload: b"frozen request".to_vec(),
            },
            response: CheckInResponse {
                context,
                historical: false,
                output: original.clone(),
            },
        };
        let output = OutputSpec::default();
        let reference = cache_reference(&saved, &output.context).unwrap();
        let mut cursor = None;
        let mut collected = Vec::new();
        let mut pages = 0;
        loop {
            // Each page re-hashes and re-encodes the whole ~64 KiB output, so
            // the budget keeps the walk to about eight full pages (each cut
            // inside a run of two-byte characters) rather than dozens.
            let result = cached_output_page(
                &saved,
                &CachePageRequest {
                    reference: reference.clone(),
                    cursor,
                    max_bytes: 8192,
                },
                &output,
            )
            .unwrap();
            let CommandResult::CachedCheckInPage(page) = result else {
                panic!("wrong result")
            };
            pages += 1;
            assert_eq!(page.start as usize, collected.len());
            collected.extend_from_slice(page.chunk_data.as_bytes());
            if page.next_argv.is_none() {
                break;
            }
            cursor = Some(CacheCursorV1 {
                version: 1,
                reference_sha256: reference.digest().unwrap(),
                offset: page.end,
            });
        }
        assert!(pages > 2, "{pages} pages");
        assert_eq!(collected, original);
        let mut wrong_reference = reference.clone();
        wrong_reference.output_sha256 = "0".repeat(64);
        assert_eq!(
            cached_output_page(
                &saved,
                &CachePageRequest {
                    reference: wrong_reference,
                    cursor: None,
                    max_bytes: 65_536
                },
                &output
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidRequest
        );
        let selected_elsewhere = OutputSpec {
            context: ContinuationContext {
                state_dir: Some("other".into()),
                host: None,
            },
            ..output.clone()
        };
        assert_eq!(
            cached_output_page(
                &saved,
                &CachePageRequest {
                    reference: reference.clone(),
                    cursor: None,
                    max_bytes: 65_536
                },
                &selected_elsewhere
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidRequest
        );
        let wrong_digest = CacheCursorV1 {
            version: 1,
            reference_sha256: [0; 32],
            offset: 0,
        };
        assert_eq!(
            cached_output_page(
                &saved,
                &CachePageRequest {
                    reference: reference.clone(),
                    cursor: Some(wrong_digest),
                    max_bytes: 65_536
                },
                &output
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidCursor
        );
        let too_far = CacheCursorV1 {
            version: 1,
            reference_sha256: reference.digest().unwrap(),
            offset: original.len() as u32 + 1,
        };
        assert_eq!(
            cached_output_page(
                &saved,
                &CachePageRequest {
                    reference: reference.clone(),
                    cursor: Some(too_far),
                    max_bytes: 65_536
                },
                &output
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidCursor
        );
        let minimum = cached_output_page(
            &saved,
            &CachePageRequest {
                reference: reference.clone(),
                cursor: None,
                max_bytes: 256,
            },
            &output,
        )
        .unwrap_err();
        assert_eq!(minimum.code, ErrorCode::InvalidBudget);
        assert!(minimum.required_minimum_bytes.unwrap() > 256);
        let max_selector = OutputSpec {
            context: ContinuationContext {
                state_dir: Some("s".repeat(1024)),
                host: Some(Uuid::from_u128(8).to_string()),
            },
            ..output.clone()
        };
        let max_reference = cache_reference(&saved, &max_selector.context).unwrap();
        let max_page = cached_output_page(
            &saved,
            &CachePageRequest {
                reference: max_reference,
                cursor: None,
                max_bytes: 65_536,
            },
            &max_selector,
        )
        .unwrap();
        let CommandResult::CachedCheckInPage(max_page) = max_page else {
            panic!("wrong result")
        };
        assert!(max_page.end > 0);
        let max_terminal = CachePageRequest {
            reference: cache_reference(&saved, &max_selector.context).unwrap(),
            cursor: Some(CacheCursorV1 {
                version: 1,
                reference_sha256: cache_reference(&saved, &max_selector.context)
                    .unwrap()
                    .digest()
                    .unwrap(),
                offset: original.len() as u32,
            }),
            max_bytes: 65_536,
        };
        let complete = cached_output_page(&saved, &max_terminal, &max_selector).unwrap();
        let full_length = encode_selected(&complete, &max_selector).unwrap().len() as u32;
        assert!(
            cached_output_page(
                &saved,
                &CachePageRequest {
                    max_bytes: full_length,
                    ..max_terminal.clone()
                },
                &max_selector
            )
            .is_ok()
        );
        let error = cached_output_page(
            &saved,
            &CachePageRequest {
                max_bytes: full_length - 1,
                ..max_terminal
            },
            &max_selector,
        )
        .unwrap_err();
        assert_eq!(error.required_minimum_bytes, Some(full_length));
        let bad = CacheCursorV1 {
            version: 1,
            reference_sha256: reference.digest().unwrap(),
            offset: 1,
        };
        assert_eq!(
            cached_output_page(
                &saved,
                &CachePageRequest {
                    reference: reference.clone(),
                    cursor: Some(bad),
                    max_bytes: 65_536
                },
                &output
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidCursor
        );
        let terminal_cursor = CacheCursorV1 {
            version: 1,
            reference_sha256: reference.digest().unwrap(),
            offset: original.len() as u32,
        };
        let terminal_request = CachePageRequest {
            reference: reference.clone(),
            cursor: Some(terminal_cursor),
            max_bytes: 65_536,
        };
        let terminal = cached_output_page(&saved, &terminal_request, &output).unwrap();
        assert!(
            serde_json::from_value::<CommandResult>(serde_json::to_value(&terminal).unwrap())
                .is_err()
        );
        let CommandResult::CachedCheckInPage(ref terminal_page) = terminal else {
            panic!("wrong result")
        };
        assert!(terminal_page.chunk_data.is_empty());
        assert_eq!(terminal_page.start, terminal_page.end);
        assert!(terminal_page.next_argv.is_none());
        let exact = encode_selected(&terminal, &output).unwrap().len() as u32;
        assert!(exact >= 256);
        assert!(
            cached_output_page(
                &saved,
                &CachePageRequest {
                    max_bytes: exact,
                    ..terminal_request.clone()
                },
                &output
            )
            .is_ok()
        );
        let too_small = cached_output_page(
            &saved,
            &CachePageRequest {
                max_bytes: exact - 1,
                ..terminal_request
            },
            &output,
        )
        .unwrap_err();
        assert_eq!(too_small.code, ErrorCode::InvalidBudget);
        assert_eq!(too_small.required_minimum_bytes, Some(exact));
        let mut small = saved.clone();
        small.response.output = "\u{001b}\u{0085}\u{2028}".as_bytes().to_vec();
        let small_ref = cache_reference(&small, &output.context).unwrap();
        let small_req = CachePageRequest {
            reference: small_ref,
            cursor: None,
            max_bytes: 65_536,
        };
        let json = encode_selected(
            &cached_output_page(&small, &small_req, &output).unwrap(),
            &output,
        )
        .unwrap();
        assert!(!json.contains(&0x1b));
        assert!(String::from_utf8(json).unwrap().contains("\\u0085"));
        let text_spec = OutputSpec {
            format: OutputFormat::Text,
            ..output
        };
        let text_ref = cache_reference(&small, &text_spec.context).unwrap();
        let text_req = CachePageRequest {
            reference: text_ref,
            cursor: None,
            max_bytes: 65_536,
        };
        let text = String::from_utf8(
            encode_selected(
                &cached_output_page(&small, &text_req, &text_spec).unwrap(),
                &text_spec,
            )
            .unwrap(),
        )
        .unwrap();
        assert!(text.contains("untrusted_chunk_data: \"\\u001b\\u0085\\u2028\""));
    }

    #[test]
    fn local_cache_command_parses_canonical_reference() {
        let reference = CacheRefV1 {
            version: 1,
            key: CachedCheckInKey {
                instance: Uuid::from_u128(1),
                seat: "seat".into(),
                event_id: "event".into(),
                operation_id: Uuid::from_u128(2),
            },
            request_sha256: "a".repeat(64),
            output_sha256: "b".repeat(64),
            response_meta_sha256: "c".repeat(64),
            selectors_sha256: "d".repeat(64),
        };
        let token = reference.token().unwrap();
        let parsed = crate::cli::commands::parse_argv([
            "herdr-threads",
            "--json",
            "cached-check-in",
            "--reference",
            &token,
            "--max-bytes",
            "2048",
        ])
        .unwrap();
        let crate::cli::commands::CliAction::CachedCheckIn(request) = parsed.action else {
            panic!("expected local cache read")
        };
        assert_eq!(request.reference, reference);
        assert_eq!(request.max_bytes, 2048);
    }

    #[test]
    fn completed_journal_read_preserves_saved_bytes() {
        use crate::{
            app::SystemClock,
            harness::context::ContextJournal,
            protocol::time::{CallBudget, Cancellation, Clock, MonoInstant},
        };
        use std::{fs, time::Duration};
        let directory = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("cache-read-{}", Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        }
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());
        let context = OccupantContext {
            format_version: 1,
            instance: Uuid::from_u128(1),
            seat: "seat".into(),
            target: "target".into(),
            harness: crate::harness::context::Harness::Codex,
            binding_generation: 1,
            execution: Uuid::from_u128(3),
            session: SessionReference::PluginContext(Uuid::from_u128(3)),
            role: Role::TopLevel,
        };
        let journal = ContextJournal::open(
            &directory,
            context.instance,
            &context.seat,
            Duration::from_secs(1),
        )
        .unwrap();
        let request = PendingCheckIn {
            operation_id: Uuid::from_u128(2),
            mode: CheckInMode::Lifecycle,
            context: context.clone(),
            expected_generation: None,
            event_id: "event".into(),
            payload_version: 1,
            payload: b"frozen".to_vec(),
        };
        journal.prepare(request.clone()).unwrap();
        let response = CheckInResponse {
            context,
            historical: false,
            output: format!("{}🧭", "é".repeat(32_000)).into_bytes(),
        };
        journal
            .dispatch("event", &mut |_: &PendingCheckIn| Ok(response.clone()))
            .unwrap();
        let before = fs::read(directory.join("context.json")).unwrap();
        let clock = SystemClock::new();
        let budget = CallBudget {
            deadline: MonoInstant(clock.monotonic_now().0 + 5_000),
            cancellation: Cancellation::default(),
        };
        let key = CachedCheckInKey {
            instance: request.context.instance,
            seat: request.context.seat.clone(),
            event_id: request.event_id.clone(),
            operation_id: request.operation_id,
        };
        let saved = journal.read_completed(&key, &budget, &clock).unwrap();
        assert_eq!(saved.request, request);
        assert_eq!(saved.response.output, response.output);
        assert_eq!(before, fs::read(directory.join("context.json")).unwrap());
        struct Never;
        impl crate::ports::LocalClient for Never {
            crate::default_output_local_client!();
            fn call(
                &self,
                _: crate::protocol::commands::Command,
                _: &CallBudget,
            ) -> Result<CommandResult, crate::protocol::results::ApiError> {
                panic!("cached page must not dispatch transport")
            }
        }
        let intents_dir = directory.join("intents");
        let intents = crate::cli::journal::Journal::open(&intents_dir).unwrap();
        let snapshot = |path: &std::path::Path| -> Vec<(String, Vec<u8>)> {
            let mut files = Vec::new();
            for entry in fs::read_dir(path).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_file() {
                    files.push((
                        entry.file_name().to_string_lossy().into_owned(),
                        fs::read(entry.path()).unwrap(),
                    ));
                }
            }
            files.sort_by(|a, b| a.0.cmp(&b.0));
            files
        };
        let intents_before = snapshot(&intents_dir);
        let reference = cache_reference(&saved, &OutputSpec::default().context)
            .unwrap()
            .token()
            .unwrap();
        let parsed = crate::cli::commands::parse_argv([
            "herdr-threads",
            "--json",
            "cached-check-in",
            "--reference",
            &reference,
            "--max-bytes",
            "2048",
        ])
        .unwrap();
        let mut page_output = Vec::new();
        crate::cli::run_cooperative(
            parsed,
            &intents,
            &journal,
            None,
            Role::Subagent,
            &Never,
            &clock,
            &mut page_output,
        )
        .unwrap();
        assert!(!page_output.is_empty());
        assert_eq!(snapshot(&intents_dir), intents_before);
        assert_eq!(fs::read(directory.join("context.json")).unwrap(), before);
        let mut wrong = key.clone();
        wrong.operation_id = Uuid::from_u128(9);
        assert_eq!(
            journal.read_completed(&wrong, &budget, &clock).unwrap_err(),
            CacheReadError::NotFound
        );
        budget.cancellation.cancel();
        assert_eq!(
            journal.read_completed(&key, &budget, &clock).unwrap_err(),
            CacheReadError::Cancelled
        );
        let live = CallBudget {
            deadline: MonoInstant(clock.monotonic_now().0 + 5_000),
            cancellation: Cancellation::default(),
        };
        let mut successor = request.clone();
        successor.operation_id = Uuid::from_u128(4);
        successor.event_id = "successor".into();
        successor.expected_generation = Some(1);
        successor.context.execution = Uuid::from_u128(5);
        successor.context.session = SessionReference::PluginContext(Uuid::from_u128(5));
        successor.context.binding_generation = 2;
        journal.prepare(successor.clone()).unwrap();
        journal
            .dispatch("successor", &mut |_: &PendingCheckIn| {
                Ok(CheckInResponse {
                    context: successor.context.clone(),
                    historical: false,
                    output: b"successor".to_vec(),
                })
            })
            .unwrap();
        let current_before = journal.current().unwrap();
        assert_eq!(
            journal
                .read_completed(&key, &live, &clock)
                .unwrap()
                .response
                .output,
            response.output
        );
        assert_eq!(journal.current().unwrap(), current_before);
        let complete_bytes = fs::read(directory.join("context.json")).unwrap();
        fs::write(directory.join("context.json"), b"{broken").unwrap();
        assert_eq!(
            journal.read_completed(&key, &live, &clock).unwrap_err(),
            CacheReadError::Context(crate::harness::context::ContextError::Corrupt)
        );
        fs::write(directory.join("context.json"), &complete_bytes).unwrap();
        let mut duplicate: serde_json::Value = serde_json::from_slice(&complete_bytes).unwrap();
        let first = duplicate["completed"][0].clone();
        duplicate["completed"].as_array_mut().unwrap().push(first);
        fs::write(
            directory.join("context.json"),
            serde_json::to_vec(&duplicate).unwrap(),
        )
        .unwrap();
        assert_eq!(
            journal.read_completed(&key, &live, &clock).unwrap_err(),
            CacheReadError::Context(crate::harness::context::ContextError::Corrupt)
        );
        fs::write(directory.join("context.json"), &complete_bytes).unwrap();
        let mut divergent: serde_json::Value = serde_json::from_slice(&complete_bytes).unwrap();
        let mut second = divergent["completed"][0].clone();
        second["request"]["operation_id"] = serde_json::json!(Uuid::from_u128(9));
        divergent["completed"]
            .as_array_mut()
            .unwrap()
            .insert(0, second);
        fs::write(
            directory.join("context.json"),
            serde_json::to_vec(&divergent).unwrap(),
        )
        .unwrap();
        assert_eq!(
            journal.read_completed(&key, &live, &clock).unwrap_err(),
            CacheReadError::Context(crate::harness::context::ContextError::Corrupt)
        );
        fs::write(directory.join("context.json"), &complete_bytes).unwrap();
        let lock_file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(directory.join("context.lock"))
            .unwrap();
        lock_file.lock().unwrap();
        let short = CallBudget {
            deadline: MonoInstant(clock.monotonic_now().0 + 20),
            cancellation: Cancellation::default(),
        };
        assert_eq!(
            journal.read_completed(&key, &short, &clock).unwrap_err(),
            CacheReadError::DeadlineExceeded
        );
        drop(lock_file);
        fs::remove_file(directory.join("context.lock")).unwrap();
        assert_eq!(
            journal.read_completed(&key, &live, &clock).unwrap_err(),
            CacheReadError::NotFound
        );
        assert!(!directory.join("context.lock").exists());
    }
}
