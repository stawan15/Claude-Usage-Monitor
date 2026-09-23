use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The coding agent a usage record came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Tool {
    ClaudeCode,
    Codex,
    OpenCode,
    Gemini,
}

impl Tool {
    pub const ALL: [Tool; 4] = [Tool::ClaudeCode, Tool::Codex, Tool::OpenCode, Tool::Gemini];

    pub fn name(self) -> &'static str {
        match self {
            Tool::ClaudeCode => "Claude Code",
            Tool::Codex => "Codex",
            Tool::OpenCode => "OpenCode",
            Tool::Gemini => "Gemini CLI",
        }
    }

    pub fn short_name(self) -> &'static str {
        match self {
            Tool::ClaudeCode => "Claude",
            Tool::Gemini => "Gemini",
            other => other.name(),
        }
    }
}

/// One billed model response.
#[derive(Clone, Debug)]
pub struct UsageEvent {
    pub key: String,
    pub date: DateTime<Utc>,
    pub tool: Tool,
    pub model: String,
    pub project: String,
    /// Uncached input tokens.
    pub input: u64,
    /// Output tokens, including reasoning/thinking.
    pub output: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    /// Set when the tool reports its own cost (OpenCode); otherwise priced from `Pricing`.
    pub cost: Option<f64>,
}

impl UsageEvent {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_write + self.cache_read
    }
}

/// A plan rate limit as reported by the tool itself (currently Codex).
#[derive(Clone, Debug)]
pub struct LimitStatus {
    pub tool: Tool,
    pub window_minutes: u32,
    pub used_percent: f64,
    pub resets_at: Option<DateTime<Utc>>,
    pub observed_at: DateTime<Utc>,
}

impl LimitStatus {
    pub fn id(&self) -> String {
        format!("{:?}-{}", self.tool, self.window_minutes)
    }
}

#[derive(Default, Debug)]
pub struct ScanOutput {
    pub events: Vec<UsageEvent>,
    pub limits: Vec<LimitStatus>,
}

/// Reads one tool's local logs. Only ever called from the watcher thread.
pub trait Provider: Send {
    /// Directories to watch for changes. They need not exist yet.
    fn watch_paths(&self) -> Vec<PathBuf>;
    /// Appends anything new since the previous scan.
    fn scan(&mut self, cutoff: DateTime<Utc>, out: &mut ScanOutput);
}

pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

pub fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

pub fn parse_timestamp(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc))
}

/// Last path component of a directory as a display name, e.g. "/Users/me/app" → "app".
pub fn dir_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}
