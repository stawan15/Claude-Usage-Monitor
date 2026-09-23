use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;

use crate::model::{home, UsageEvent};

/// USD per million tokens.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPrice {
    pub input: f64,
    pub output: f64,
    pub cache_write: f64,
    pub cache_read: f64,
}

const fn price(input: f64, output: f64, cache_write: f64, cache_read: f64) -> ModelPrice {
    ModelPrice { input, output, cache_write, cache_read }
}

/// Input/output rates are Anthropic's published first-party API prices. Cache reads for
/// Fable 5.1 and Opus 5.5 are published; the other cache rates use the standard
/// multipliers (write = 1.25x input for the 5-minute cache, read = 0.1x input).
const DEFAULTS: &[(&str, ModelPrice)] = &[
    ("claude-fable-5-1", price(10.0, 50.0, 12.5, 0.25)),
    ("claude-fable-5", price(10.0, 50.0, 12.5, 1.0)),
    ("claude-mythos-5-1", price(10.0, 50.0, 12.5, 1.0)),
    ("claude-opus-5-5", price(4.0, 20.0, 5.0, 0.20)),
    ("claude-opus-5", price(5.0, 25.0, 6.25, 0.5)),
    ("claude-opus-4-8", price(5.0, 25.0, 6.25, 0.5)),
    ("claude-opus-4-7", price(5.0, 25.0, 6.25, 0.5)),
    ("claude-opus-4-6", price(5.0, 25.0, 6.25, 0.5)),
    ("claude-sonnet-5", price(2.0, 10.0, 2.5, 0.2)),
    ("claude-sonnet-4-6", price(3.0, 15.0, 3.75, 0.3)),
    ("claude-haiku-4-5", price(1.0, 5.0, 1.25, 0.1)),
];

/// API-equivalent cost estimates. On a subscription you are not billed per token;
/// this shows what the same traffic would cost on the API.
pub struct Pricing {
    table: HashMap<String, ModelPrice>,
    /// Longest first, so "claude-opus-5-5" wins over "claude-opus-5".
    keys: Vec<String>,
}

impl Pricing {
    pub fn override_path() -> PathBuf {
        home().join(".config").join("claude-monitor").join("pricing.json")
    }

    /// Built-in prices, with any entries from `~/.config/claude-monitor/pricing.json` on top.
    pub fn load() -> Self {
        let mut table: HashMap<String, ModelPrice> = DEFAULTS.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        if let Ok(data) = std::fs::read(Self::override_path()) {
            if let Ok(overrides) = serde_json::from_slice::<HashMap<String, ModelPrice>>(&data) {
                table.extend(overrides);
            }
        }
        let mut keys: Vec<String> = table.keys().cloned().collect();
        keys.sort_by_key(|k| std::cmp::Reverse(k.len()));
        Self { table, keys }
    }

    pub fn price(&self, model: &str) -> Option<ModelPrice> {
        // "anthropic/claude-sonnet-5" (OpenRouter style) matches "claude-sonnet-5".
        let bare = model.rsplit('/').next().unwrap_or(model);
        self.keys.iter().find(|k| bare.starts_with(k.as_str())).map(|k| self.table[k])
    }

    pub fn cost(&self, e: &UsageEvent) -> Option<f64> {
        let p = self.price(&e.model)?;
        let micro = e.input as f64 * p.input
            + e.output as f64 * p.output
            + e.cache_write as f64 * p.cache_write
            + e.cache_read as f64 * p.cache_read;
        Some(micro / 1_000_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longest_prefix_wins() {
        let pricing = Pricing::load();
        assert_eq!(pricing.price("claude-opus-5-5").unwrap().input, 4.0);
        assert_eq!(pricing.price("claude-opus-5").unwrap().input, 5.0);
        assert_eq!(pricing.price("claude-haiku-4-5-20251001").unwrap().input, 1.0);
        assert_eq!(pricing.price("anthropic/claude-sonnet-5").unwrap().input, 2.0);
        assert!(pricing.price("gpt-5.5").is_none());
    }
}
