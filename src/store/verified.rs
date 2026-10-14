//! Successful pure checks of canonical rows, remembered by their exact inputs.
//!
//! A cooperative permit is fresh for `MAX_PERMIT_MILLIS` from issuance to the
//! decision. The decision transaction re-reads every canonical row it decides
//! on (A2), but the checks of a body-bearing row (decode, structural
//! validation, re-derivation) are pure functions of the row's bytes and the
//! request: run again on identical bytes they can only return the same value.
//! The daemon's pre-issuance reads (status, send preparation) already ran them,
//! so inside the decision window a byte-identical input returns the remembered
//! success instead of repeating whole-body work that grows with the body.
//!
//! Only successes are remembered; a failure is recomputed every time. An entry
//! matches only the same step and value type with byte-identical inputs
//! (compared in full, not by digest: hashing a maximum body costs as much as
//! the checks it would skip). A step must depend on nothing but its listed
//! inputs (no clock, environment, file system or database).
use crate::protocol::results::ApiError;
use std::{
    any::{Any, TypeId},
    collections::VecDeque,
    sync::{Arc, Mutex},
};

/// Enough for every step of a couple of concurrent requests. An entry keeps
/// its inputs and value: at most a few maximum bodies each.
const CAPACITY: usize = 8;

struct Entry {
    step: &'static str,
    value_type: TypeId,
    inputs: Vec<Vec<u8>>,
    value: Arc<dyn Any + Send + Sync>,
}
impl Entry {
    fn matches(&self, step: &str, value_type: TypeId, inputs: &[&[u8]]) -> bool {
        self.step == step
            && self.value_type == value_type
            && self.inputs.len() == inputs.len()
            && self
                .inputs
                .iter()
                .zip(inputs)
                .all(|(a, b)| a.as_slice() == *b)
    }
}
static REMEMBERED: Mutex<VecDeque<Entry>> = Mutex::new(VecDeque::new());

/// `check`'s value for exactly `inputs`: remembered after a success,
/// recomputed otherwise. `check` must be a pure function of `inputs`.
pub(crate) fn remembered<T: Clone + Send + Sync + 'static>(
    step: &'static str,
    inputs: &[&[u8]],
    check: impl FnOnce() -> Result<T, ApiError>,
) -> Result<T, ApiError> {
    let value_type = TypeId::of::<T>();
    let hit = {
        let cache = REMEMBERED.lock().unwrap_or_else(|p| p.into_inner());
        cache
            .iter()
            .find(|e| e.matches(step, value_type, inputs))
            .and_then(|e| e.value.downcast_ref::<T>().cloned())
    };
    if let Some(value) = hit {
        return Ok(value);
    }
    let value = check()?;
    let mut cache = REMEMBERED.lock().unwrap_or_else(|p| p.into_inner());
    if !cache.iter().any(|e| e.matches(step, value_type, inputs)) {
        cache.push_front(Entry {
            step,
            value_type,
            inputs: inputs.iter().map(|i| i.to_vec()).collect(),
            value: Arc::new(value.clone()),
        });
        cache.truncate(CAPACITY);
    }
    Ok(value)
}

/// `validate(value)` for a request, remembered by the request's serialized
/// form (its every field): the decision window repeats checks the daemon
/// already ran on the same request before issuing the permit.
pub(crate) fn validated<T: serde::Serialize>(
    step: &'static str,
    value: &T,
    validate: impl FnOnce(&T) -> Result<(), &'static str>,
) -> Result<(), ApiError> {
    let Ok(bytes) = serde_json::to_vec(value) else {
        return validate(value).map_err(ApiError::invalid_request);
    };
    remembered(step, &[&bytes], || {
        validate(value).map_err(ApiError::invalid_request)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::results::ErrorCode;
    use crate::store::connection::api_error;
    use std::cell::Cell;

    #[test]
    fn success_is_remembered_by_exact_inputs_and_failure_is_not() {
        let step = "verified-test-success";
        let calls = Cell::new(0);
        let run = |input: &[u8]| {
            remembered(step, &[input], || {
                calls.set(calls.get() + 1);
                Ok(input.len())
            })
        };
        let unique = uuid::Uuid::new_v4();
        let a = unique.as_bytes().as_slice();
        assert_eq!(run(a).unwrap(), 16);
        assert_eq!(run(a).unwrap(), 16);
        assert_eq!(calls.get(), 1, "an identical input is remembered");
        let mut b = a.to_vec();
        b[0] ^= 1;
        run(&b).unwrap();
        assert_eq!(calls.get(), 2, "one changed byte is a new input");
        let failures = Cell::new(0);
        for _ in 0..2 {
            let failed: Result<(), _> = remembered("verified-test-failure", &[a], || {
                failures.set(failures.get() + 1);
                Err(api_error(ErrorCode::StoreCorrupt, "corrupt"))
            });
            assert!(failed.is_err());
        }
        assert_eq!(failures.get(), 2, "a failure is recomputed");
    }

    #[test]
    fn inputs_are_compared_whole_and_steps_and_types_separate() {
        let calls = Cell::new(0);
        let count = |step: &'static str, inputs: &[&[u8]]| {
            remembered(step, inputs, || {
                calls.set(calls.get() + 1);
                Ok(())
            })
            .unwrap()
        };
        let unique = uuid::Uuid::new_v4();
        let u = unique.as_bytes().as_slice();
        count("verified-test-split", &[u, b"ab", b"c"]);
        count("verified-test-split", &[u, b"a", b"bc"]);
        assert_eq!(calls.get(), 2, "inputs are not concatenated");
        count("verified-test-other", &[u, b"ab", b"c"]);
        assert_eq!(calls.get(), 3, "steps are separate");
        let typed: u8 = remembered("verified-test-split", &[u, b"ab", b"c"], || {
            calls.set(calls.get() + 1);
            Ok(7)
        })
        .unwrap();
        assert_eq!((typed, calls.get()), (7, 4), "value types are separate");
    }
}
