//! The per-seat config registry (ht-z41e): the closed set of keys a seat may carry, each with its
//! description, value type, default, write tier and backing store.
//!
//! The registry is the single source: the daemon validates every read and write against it, the
//! store never checks a key in SQL, and `ht config seat list` renders it without a daemon.

use serde::{Deserialize, Serialize};

/// Who may write a key. `Agent` keys are written through the seat's own top-level binding;
/// `Human` keys only as an operator decision (`ht human config seat set`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigTier {
    Agent,
    Human,
}

impl ConfigTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Human => "human",
        }
    }
}

/// The values a key accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigValues {
    /// An integer in `min..=max`, written in plain decimal.
    IntRange { min: i64, max: i64 },
    /// Exactly one of these words (case-sensitive; no synonyms).
    Enum(&'static [&'static str]),
}

/// Where a key's value lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigBacking {
    /// Generic `seat_config` rows: a row exists only while the key is set explicitly.
    Table,
    /// The existing `seats.nudge_threshold` column (and its `wake_on` mirror); the `seat_config`
    /// row is audit-only.
    NudgeThresholdColumn,
}

/// One registered key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigKey {
    pub name: &'static str,
    /// One line, under 110 bytes.
    pub description: &'static str,
    pub values: ConfigValues,
    pub default: &'static str,
    pub tier: ConfigTier,
    pub backing: ConfigBacking,
}

/// The closed registry (human decision S1). Adding a key is one entry here: a `Table`-backed key
/// needs no migration (rows are `(seat, key, value)` text) and no wire change (keys and values
/// travel as strings).
pub const REGISTRY: &[ConfigKey] = &[
    ConfigKey {
        name: "nudge_threshold",
        description: "idle seat wakes for items with priority <= N; -1 wakes for every item",
        values: ConfigValues::IntRange { min: -1, max: 9 },
        default: "0",
        tier: ConfigTier::Agent,
        backing: ConfigBacking::NudgeThresholdColumn,
    },
    ConfigKey {
        name: "programmatic_wake",
        description: "allow idle wake through the harness's programmatic transport, incl. the root session beside a side chat",
        values: ConfigValues::Enum(&["on", "off"]),
        default: "off",
        tier: ConfigTier::Human,
        backing: ConfigBacking::Table,
    },
];

/// The registered key `name`, or the refusal naming every key.
pub fn key(name: &str) -> Result<&'static ConfigKey, String> {
    REGISTRY.iter().find(|k| k.name == name).ok_or_else(|| {
        let names: Vec<&str> = REGISTRY.iter().map(|k| k.name).collect();
        format!(
            "unknown seat config key `{name}`; keys: {}",
            names.join(", ")
        )
    })
}

/// The one-line refusal of an agent-tier write of a human-tier key, naming the `ht human` spelling
/// the person's command takes: `args` is the verb and its operands (`set KEY VALUE`, `unset KEY`).
/// Shared by the daemon (placeholder operands) and the CLI (the operands as typed).
pub fn human_tier_refusal(key: &str, args: &str) -> String {
    format!("seat config key `{key}` needs `ht human config seat {args}`")
}

impl ConfigKey {
    /// `-1..9` or `on|off`.
    pub fn values_label(&self) -> String {
        match self.values {
            ConfigValues::IntRange { min, max } => format!("{min}..{max}"),
            ConfigValues::Enum(words) => words.join("|"),
        }
    }

    /// The canonical stored text of `value`, or the refusal naming the valid set. Exact match
    /// only: `true`, `enable` and `ON` are not `on`.
    pub fn validate(&self, value: &str) -> Result<String, String> {
        let refuse = || {
            format!(
                "invalid value `{value}` for {}; values: {}",
                self.name,
                self.values_label()
            )
        };
        match self.values {
            ConfigValues::IntRange { min, max } => {
                let digits = value.strip_prefix('-').unwrap_or(value);
                if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(refuse());
                }
                let parsed: i64 = value.parse().map_err(|_| refuse())?;
                if (min..=max).contains(&parsed) {
                    Ok(parsed.to_string())
                } else {
                    Err(refuse())
                }
            }
            ConfigValues::Enum(words) => words
                .iter()
                .find(|w| **w == value)
                .map(|w| (*w).to_owned())
                .ok_or_else(refuse),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_exactly_the_two_keys_with_their_tier_range_and_default() {
        let names: Vec<_> = REGISTRY.iter().map(|k| k.name).collect();
        assert_eq!(names, ["nudge_threshold", "programmatic_wake"]);
        let nudge = key("nudge_threshold").unwrap();
        assert_eq!(nudge.tier, ConfigTier::Agent);
        assert_eq!(nudge.values, ConfigValues::IntRange { min: -1, max: 9 });
        assert_eq!(nudge.default, "0");
        assert_eq!(nudge.backing, ConfigBacking::NudgeThresholdColumn);
        let wake = key("programmatic_wake").unwrap();
        assert_eq!(wake.tier, ConfigTier::Human);
        assert_eq!(wake.values, ConfigValues::Enum(&["on", "off"]));
        assert_eq!(wake.default, "off");
        assert_eq!(wake.backing, ConfigBacking::Table);
        assert_eq!(nudge.values_label(), "-1..9");
        assert_eq!(wake.values_label(), "on|off");
    }

    #[test]
    fn unknown_key_refusal_lists_every_key() {
        assert_eq!(
            key("nope").unwrap_err(),
            "unknown seat config key `nope`; keys: nudge_threshold, programmatic_wake"
        );
    }

    #[test]
    fn validation_is_exact_and_names_the_valid_set() {
        let nudge = key("nudge_threshold").unwrap();
        let wake = key("programmatic_wake").unwrap();
        assert_eq!(nudge.validate("-1").unwrap(), "-1");
        assert_eq!(nudge.validate("9").unwrap(), "9");
        assert_eq!(wake.validate("on").unwrap(), "on");
        assert_eq!(wake.validate("off").unwrap(), "off");
        for bad in ["-5", "10", "x", "", "-", "+1", " 1", "99999999999999999999"] {
            assert_eq!(
                nudge.validate(bad).unwrap_err(),
                format!("invalid value `{bad}` for nudge_threshold; values: -1..9")
            );
        }
        for bad in ["true", "enable", "ON", "On", "1", ""] {
            assert_eq!(
                wake.validate(bad).unwrap_err(),
                format!("invalid value `{bad}` for programmatic_wake; values: on|off")
            );
        }
    }

    #[test]
    fn names_are_unique_defaults_validate_and_descriptions_fit_a_list_line() {
        let mut seen = std::collections::BTreeSet::new();
        for k in REGISTRY {
            assert!(seen.insert(k.name), "duplicate key {}", k.name);
            assert_eq!(k.validate(k.default).as_deref(), Ok(k.default));
            assert!(k.description.len() < 130, "{}", k.description);
        }
    }
}
