use std::collections::HashSet;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;

use crate::model::*;

/// OpenCode: `~/.local/share/opencode/opencode.db` (SQLite). Each assistant row in `message`
/// has provider, model, tokens and OpenCode's own cost estimate in its JSON `data` column.
/// Covers every provider OpenCode talks to: OpenAI, Google, Anthropic, OpenRouter, etc.
#[derive(Default)]
pub struct OpenCode {
    db: Option<Connection>,
    /// Highest `time_updated` (ms) already read.
    cursor: Option<i64>,
    seen: HashSet<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    #[serde(rename = "modelID")]
    model_id: Option<String>,
    #[serde(rename = "providerID")]
    provider_id: Option<String>,
    cost: Option<f64>,
    tokens: Option<Tokens>,
    path: Option<PathInfo>,
    time: Option<Time>,
}

#[derive(Deserialize)]
struct Tokens {
    input: Option<u64>,
    output: Option<u64>,
    reasoning: Option<u64>,
    cache: Option<Cache>,
}

#[derive(Deserialize)]
struct Cache {
    read: Option<u64>,
    write: Option<u64>,
}

#[derive(Deserialize)]
struct PathInfo {
    cwd: Option<String>,
}

#[derive(Deserialize)]
struct Time {
    created: Option<f64>,
    completed: Option<f64>,
}

impl OpenCode {
    fn data_dir() -> PathBuf {
        env_path("XDG_DATA_HOME").unwrap_or_else(|| home().join(".local/share")).join("opencode")
    }

    fn ensure_open(&mut self) -> bool {
        if self.db.is_none() {
            let path = Self::data_dir().join("opencode.db");
            if !path.exists() {
                return false;
            }
            let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
            let Ok(conn) = Connection::open_with_flags(path, flags) else { return false };
            let _ = conn.busy_timeout(std::time::Duration::from_millis(200));
            self.db = Some(conn);
        }
        true
    }
}

impl Provider for OpenCode {
    fn watch_paths(&self) -> Vec<PathBuf> {
        vec![Self::data_dir()]
    }

    fn scan(&mut self, cutoff: DateTime<Utc>, out: &mut ScanOutput) {
        if !self.ensure_open() {
            return;
        }
        let Self { db, cursor, seen } = self;
        let Some(db) = db.as_ref() else { return };
        let since = cursor.unwrap_or(cutoff.timestamp_millis());

        // Rows only get tokens once the response completes; incomplete ones are picked up
        // on a later scan because completing them bumps time_updated past the cursor.
        let Ok(mut stmt) = db.prepare_cached(
            "SELECT id, time_updated, data FROM message
             WHERE time_updated > ?1
               AND json_extract(data, '$.role') = 'assistant'
               AND json_extract(data, '$.time.completed') IS NOT NULL
             ORDER BY time_updated",
        ) else {
            return;
        };
        let Ok(rows) = stmt.query_map([since], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?))) else {
            return;
        };

        let mut newest = since;
        for (id, updated, data) in rows.flatten() {
            newest = newest.max(updated);
            if seen.contains(&id) {
                continue;
            }
            let Some(event) = to_event(&id, &data) else { continue };
            // Failed requests are stored with all-zero tokens.
            if event.total() == 0 || event.date < cutoff {
                continue;
            }
            seen.insert(id);
            out.events.push(event);
        }
        *cursor = Some(newest);
    }
}

fn to_event(id: &str, data: &str) -> Option<UsageEvent> {
    let row: Row = serde_json::from_str(data).ok()?;
    let tokens = row.tokens?;
    let time = row.time?;
    let millis = time.completed.or(time.created)?;
    Some(UsageEvent {
        key: format!("opencode:{id}"),
        date: DateTime::from_timestamp_millis(millis as i64)?,
        tool: Tool::OpenCode,
        model: row.model_id.or(row.provider_id).unwrap_or_else(|| "unknown".into()),
        project: row.path.and_then(|p| p.cwd).map(|c| dir_name(&c)).unwrap_or_else(|| "opencode".into()),
        input: tokens.input.unwrap_or(0),
        output: tokens.output.unwrap_or(0) + tokens.reasoning.unwrap_or(0),
        cache_write: tokens.cache.as_ref().and_then(|c| c.write).unwrap_or(0),
        cache_read: tokens.cache.as_ref().and_then(|c| c.read).unwrap_or(0),
        cost: row.cost,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_message_row() {
        let data = r#"{"role":"assistant","modelID":"deepseek/deepseek-v4.1-flash","providerID":"openrouter","cost":0.0123,"tokens":{"input":100,"output":20,"reasoning":5,"cache":{"read":300,"write":0}},"path":{"cwd":"/home/me/site"},"time":{"created":1790087355877,"completed":1790087356874}}"#;
        let e = to_event("msg_1", data).unwrap();
        assert_eq!((e.input, e.output, e.cache_read), (100, 25, 300));
        assert_eq!(e.project, "site");
        assert_eq!(e.cost, Some(0.0123));
    }
}
