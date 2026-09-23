use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::tail::JsonlTailer;
use crate::model::*;

/// Claude Code: `~/.claude/projects/<project>/<session>.jsonl`, one line per content block.
#[derive(Default)]
pub struct ClaudeCode {
    tailer: JsonlTailer,
    seen: HashSet<String>,
}

#[derive(Deserialize)]
struct Line {
    timestamp: Option<String>,
    #[serde(rename = "requestId")]
    request_id: Option<String>,
    uuid: Option<String>,
    cwd: Option<String>,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    id: Option<String>,
    model: Option<String>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Usage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
}

impl Provider for ClaudeCode {
    fn watch_paths(&self) -> Vec<PathBuf> {
        let mut roots = vec![home().join(".claude/projects"), home().join(".config/claude/projects")];
        if let Some(custom) = env_path("CLAUDE_CONFIG_DIR") {
            roots.insert(0, custom.join("projects"));
        }
        roots
    }

    fn scan(&mut self, cutoff: DateTime<Utc>, out: &mut ScanOutput) {
        let roots = self.watch_paths();
        let seen = &mut self.seen;
        self.tailer.scan(&roots, cutoff, is_jsonl, |path, line| {
            let project = path
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if let Some(event) = parse(line, &project) {
                if event.date >= cutoff && seen.insert(event.key.clone()) {
                    out.events.push(event);
                }
            }
        });
    }
}

pub fn is_jsonl(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "jsonl")
}

pub fn parse(line: &[u8], fallback_project: &str) -> Option<UsageEvent> {
    // Cheap prefilter: most lines (user turns, tool results) have no usage block.
    if !contains(line, b"\"usage\"") {
        return None;
    }
    let entry: Line = serde_json::from_slice(line).ok()?;
    let message = entry.message?;
    let usage = message.usage?;
    let model = message.model.filter(|m| m != "<synthetic>")?;
    let date = parse_timestamp(entry.timestamp.as_deref()?)?;

    // Every content block of one response shares message id + request id.
    let key = match (&message.id, &entry.request_id) {
        (Some(id), Some(request)) => format!("{id}:{request}"),
        _ => message.id.or(entry.uuid).unwrap_or_else(|| format!("{date}:{model}")),
    };

    let project = entry.cwd.map(|c| dir_name(&c)).filter(|p| !p.is_empty());

    Some(UsageEvent {
        key,
        date,
        tool: Tool::ClaudeCode,
        model,
        project: project.unwrap_or_else(|| fallback_project.to_string()),
        input: usage.input_tokens.unwrap_or(0),
        output: usage.output_tokens.unwrap_or(0),
        cache_write: usage.cache_creation_input_tokens.unwrap_or(0),
        cache_read: usage.cache_read_input_tokens.unwrap_or(0),
        cost: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_assistant_usage() {
        let line = br#"{"timestamp":"2026-09-23T00:52:44.855Z","requestId":"req_1","cwd":"/Users/me/app","message":{"id":"msg_1","model":"claude-opus-5-5","usage":{"input_tokens":2,"cache_creation_input_tokens":9140,"cache_read_input_tokens":29570,"output_tokens":383}}}"#;
        let e = parse(line, "fallback").unwrap();
        assert_eq!(e.key, "msg_1:req_1");
        assert_eq!(e.project, "app");
        assert_eq!((e.input, e.output, e.cache_write, e.cache_read), (2, 383, 9140, 29570));
    }

    #[test]
    fn skips_synthetic_and_usage_free_lines() {
        assert!(parse(br#"{"type":"user","message":{"content":"hi"}}"#, "p").is_none());
        let synthetic = br#"{"timestamp":"2026-09-23T00:52:44Z","message":{"model":"<synthetic>","usage":{"input_tokens":0}}}"#;
        assert!(parse(synthetic, "p").is_none());
    }
}
