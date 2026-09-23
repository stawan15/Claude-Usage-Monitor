use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::claude::is_jsonl;
use super::tail::JsonlTailer;
use crate::model::*;

/// OpenAI Codex CLI: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`.
///
/// Each turn logs an `event_msg` of type `token_count` carrying that turn's usage and the
/// account's current rate limits. The model and working directory come from earlier
/// `turn_context` / `session_meta` lines in the same file.
#[derive(Default)]
pub struct Codex {
    tailer: JsonlTailer,
    seen: HashSet<String>,
    models: HashMap<String, String>,
    projects: HashMap<String, String>,
}

#[derive(Deserialize)]
struct Line {
    timestamp: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    payload: Option<Payload>,
}

#[derive(Deserialize)]
struct Payload {
    #[serde(rename = "type")]
    kind: Option<String>,
    model: Option<String>,
    cwd: Option<String>,
    info: Option<Info>,
    rate_limits: Option<RateLimits>,
}

#[derive(Deserialize)]
struct Info {
    total_token_usage: Option<TokenUsage>,
    last_token_usage: Option<TokenUsage>,
}

#[derive(Deserialize)]
struct TokenUsage {
    input_tokens: Option<u64>,
    cached_input_tokens: Option<u64>,
    cache_write_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    total_tokens: Option<u64>,
}

#[derive(Deserialize)]
struct RateLimits {
    primary: Option<Window>,
    secondary: Option<Window>,
}

#[derive(Deserialize)]
struct Window {
    used_percent: Option<f64>,
    window_minutes: Option<u32>,
    resets_at: Option<f64>,
}

impl Provider for Codex {
    fn watch_paths(&self) -> Vec<PathBuf> {
        let base = env_path("CODEX_HOME").unwrap_or_else(|| home().join(".codex"));
        vec![base.join("sessions"), base.join("archived_sessions")]
    }

    fn scan(&mut self, cutoff: DateTime<Utc>, out: &mut ScanOutput) {
        let roots = self.watch_paths();
        let Self { tailer, seen, models, projects } = self;
        tailer.scan(&roots, cutoff, is_jsonl, |path, line| {
            // Same across sessions/ and archived_sessions/, so an archived file isn't counted twice.
            let session = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            parse_line(line, &session, cutoff, seen, models, projects, out);
        });
    }
}

fn parse_line(
    line: &[u8],
    session: &str,
    cutoff: DateTime<Utc>,
    seen: &mut HashSet<String>,
    models: &mut HashMap<String, String>,
    projects: &mut HashMap<String, String>,
    out: &mut ScanOutput,
) {
    let relevant = [&b"\"token_count\""[..], b"\"turn_context\"", b"\"session_meta\""];
    if !relevant.iter().any(|m| contains(line, m)) {
        return;
    }
    let Ok(entry) = serde_json::from_slice::<Line>(line) else { return };
    let Some(payload) = entry.payload else { return };

    match entry.kind.as_deref() {
        Some("session_meta" | "turn_context") => {
            if let Some(model) = payload.model {
                models.insert(session.to_string(), model);
            }
            if let Some(cwd) = payload.cwd {
                projects.insert(session.to_string(), dir_name(&cwd));
            }
        }
        Some("event_msg") if payload.kind.as_deref() == Some("token_count") => {
            let Some(date) = entry.timestamp.as_deref().and_then(parse_timestamp) else { return };
            if date < cutoff {
                return;
            }

            if let Some(limits) = payload.rate_limits {
                for window in [limits.primary, limits.secondary].into_iter().flatten() {
                    if let (Some(used), Some(minutes)) = (window.used_percent, window.window_minutes) {
                        out.limits.push(LimitStatus {
                            tool: Tool::Codex,
                            window_minutes: minutes,
                            used_percent: used,
                            resets_at: window.resets_at.and_then(|s| DateTime::from_timestamp(s as i64, 0)),
                            observed_at: date,
                        });
                    }
                }
            }

            // Codex repeats token_count with an unchanged running total; the total dedupes it.
            let Some(info) = payload.info else { return };
            let (Some(last), Some(running)) = (info.last_token_usage, info.total_token_usage.and_then(|t| t.total_tokens)) else {
                return;
            };
            let key = format!("codex:{session}:{running}");
            if !seen.insert(key.clone()) {
                return;
            }

            // OpenAI counts cached tokens inside input_tokens, and reasoning inside output_tokens.
            let cached = last.cached_input_tokens.unwrap_or(0);
            out.events.push(UsageEvent {
                key,
                date,
                tool: Tool::Codex,
                model: models.get(session).cloned().unwrap_or_else(|| "codex".into()),
                project: projects.get(session).cloned().unwrap_or_else(|| "codex".into()),
                input: last.input_tokens.unwrap_or(0).saturating_sub(cached),
                output: last.output_tokens.unwrap_or(0),
                cache_write: last.cache_write_input_tokens.unwrap_or(0),
                cache_read: cached,
                cost: None,
            });
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_turns_limits_and_dedupes_repeats() {
        let lines = [
            r#"{"timestamp":"2026-09-22T08:24:28Z","type":"turn_context","payload":{"model":"gpt-5.6-terra","cwd":"/home/me/proj"}}"#,
            r#"{"timestamp":"2026-09-22T08:24:36Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"total_tokens":17395},"last_token_usage":{"input_tokens":17297,"cached_input_tokens":6912,"output_tokens":98}},"rate_limits":{"primary":{"used_percent":98.0,"window_minutes":43200,"resets_at":1792656088}}}}"#,
            r#"{"timestamp":"2026-09-22T08:24:41Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"total_tokens":17395},"last_token_usage":{"input_tokens":17297,"cached_input_tokens":6912,"output_tokens":98}},"rate_limits":null}}"#,
        ];
        let (mut seen, mut models, mut projects, mut out) = Default::default();
        let cutoff = DateTime::from_timestamp(0, 0).unwrap();
        for line in lines {
            parse_line(line.as_bytes(), "s1", cutoff, &mut seen, &mut models, &mut projects, &mut out);
        }
        let out: ScanOutput = out;
        assert_eq!(out.events.len(), 1);
        let e = &out.events[0];
        assert_eq!((e.model.as_str(), e.project.as_str()), ("gpt-5.6-terra", "proj"));
        assert_eq!((e.input, e.cache_read, e.output), (10385, 6912, 98));
        assert_eq!(out.limits.len(), 1);
        assert_eq!(out.limits[0].window_minutes, 43200);
    }
}
