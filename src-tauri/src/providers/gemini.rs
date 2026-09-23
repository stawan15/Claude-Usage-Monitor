use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::tail::{walk, JsonlTailer};
use crate::model::*;

/// Gemini CLI: session recordings under `~/.gemini/tmp/<project>/chats/`.
///
/// Current versions append message records to `session-*.jsonl`; older ones rewrite a whole
/// `session-*.json` (`{"messages": [...]}`). Model replies have `type: "gemini"` and `tokens`.
#[derive(Default)]
pub struct Gemini {
    tailer: JsonlTailer,
    seen: HashSet<String>,
    json_stamps: HashMap<PathBuf, SystemTime>,
}

#[derive(Deserialize)]
struct Message {
    id: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    model: Option<String>,
    tokens: Option<Tokens>,
}

#[derive(Deserialize)]
struct Tokens {
    input: Option<u64>,
    output: Option<u64>,
    cached: Option<u64>,
    thoughts: Option<u64>,
    tool: Option<u64>,
}

#[derive(Deserialize)]
struct Conversation {
    messages: Option<Vec<Message>>,
}

impl Gemini {
    fn root() -> PathBuf {
        env_path("GEMINI_CLI_HOME").unwrap_or_else(home).join(".gemini").join("tmp")
    }
}

fn is_chat_file(path: &Path, ext: &str) -> bool {
    path.extension().is_some_and(|e| e == ext) && path.components().any(|c| c.as_os_str() == "chats")
}

impl Provider for Gemini {
    fn watch_paths(&self) -> Vec<PathBuf> {
        vec![Self::root()]
    }

    fn scan(&mut self, cutoff: DateTime<Utc>, out: &mut ScanOutput) {
        let root = Self::root();
        let seen = &mut self.seen;
        self.tailer.scan(std::slice::from_ref(&root), cutoff, |p| is_chat_file(p, "jsonl"), |path, line| {
            if let Ok(message) = serde_json::from_slice::<Message>(line) {
                record(message, path, cutoff, seen, out);
            }
        });

        // Legacy whole-file JSON: re-read when its modification time changes.
        let mut files = Vec::new();
        walk(&root, &mut files);
        let cutoff_time: SystemTime = cutoff.into();
        for path in files.into_iter().filter(|p| is_chat_file(p, "json")) {
            let Ok(modified) = std::fs::metadata(&path).and_then(|m| m.modified()) else { continue };
            if modified < cutoff_time || self.json_stamps.get(&path) == Some(&modified) {
                continue;
            }
            let Ok(data) = std::fs::read(&path) else { continue };
            let Ok(conversation) = serde_json::from_slice::<Conversation>(&data) else { continue };
            self.json_stamps.insert(path.clone(), modified);
            for message in conversation.messages.unwrap_or_default() {
                record(message, &path, cutoff, &mut self.seen, out);
            }
        }
    }
}

fn record(message: Message, file: &Path, cutoff: DateTime<Utc>, seen: &mut HashSet<String>, out: &mut ScanOutput) {
    if message.kind.as_deref() != Some("gemini") {
        return;
    }
    let (Some(id), Some(tokens)) = (message.id, message.tokens) else { return };
    let Some(date) = message.timestamp.as_deref().and_then(parse_timestamp) else { return };
    if date < cutoff {
        return;
    }

    let session = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let key = format!("gemini:{session}:{id}");
    if !seen.insert(key.clone()) {
        return;
    }

    // <project>/chats/<file>
    let project = file
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "gemini".into());

    // Gemini's prompt count includes cached tokens; thoughts are billed as output.
    let cached = tokens.cached.unwrap_or(0);
    out.events.push(UsageEvent {
        key,
        date,
        tool: Tool::Gemini,
        model: message.model.unwrap_or_else(|| "gemini".into()),
        project,
        input: tokens.input.unwrap_or(0).saturating_sub(cached) + tokens.tool.unwrap_or(0),
        output: tokens.output.unwrap_or(0) + tokens.thoughts.unwrap_or(0),
        cache_write: 0,
        cache_read: cached,
        cost: None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_model_replies_once() {
        let msg = || {
            serde_json::from_str::<Message>(r#"{"id":"g1","timestamp":"2026-09-23T01:00:00Z","type":"gemini","model":"gemini-3.5-pro","tokens":{"input":1000,"output":50,"cached":400,"thoughts":20,"tool":5}}"#).unwrap()
        };
        let file = Path::new("/home/me/.gemini/tmp/proj/chats/session-a.jsonl");
        let (mut seen, mut out) = (HashSet::new(), ScanOutput::default());
        let cutoff = DateTime::from_timestamp(0, 0).unwrap();
        record(msg(), file, cutoff, &mut seen, &mut out);
        record(msg(), file, cutoff, &mut seen, &mut out);
        assert_eq!(out.events.len(), 1);
        let e = &out.events[0];
        assert_eq!((e.input, e.output, e.cache_read), (605, 70, 400));
        assert_eq!(e.project, "proj");
    }
}
