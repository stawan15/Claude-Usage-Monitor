use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, Local, TimeZone, Timelike, Utc};
use serde::Serialize;

use crate::model::{LimitStatus, ScanOutput, Tool, UsageEvent};
use crate::pricing::Pricing;

/// Ignore anything older than this; the UI shows at most 7 days.
pub const RETENTION_DAYS: i64 = 8;
const BLOCK_HOURS: i64 = 5;
const BURN_WINDOW_MINUTES: i64 = 15;

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    pub input: u64,
    pub output: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub total: u64,
    pub cost: f64,
    pub requests: u64,
    /// Requests with a known cost. When zero, the cost is unknown rather than free.
    pub priced_requests: u64,
}

impl Totals {
    fn add(&mut self, e: &UsageEvent, cost: Option<f64>) {
        self.input += e.input;
        self.output += e.output;
        self.cache_write += e.cache_write;
        self.cache_read += e.cache_read;
        self.total += e.total();
        self.cost += cost.unwrap_or(0.0);
        self.requests += 1;
        if cost.is_some() {
            self.priced_requests += 1;
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedTotals {
    pub name: String,
    pub totals: Totals,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RangeStats {
    pub totals: Totals,
    pub by_tool: Vec<NamedTotals>,
    pub by_model: Vec<NamedTotals>,
    pub by_project: Vec<NamedTotals>,
}

/// A Claude plan usage window: starts at the top of the hour of the first request
/// after the previous window expired, and lasts five hours.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    pub start: i64,
    pub end: i64,
    pub totals: Totals,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Limit {
    pub tool: Tool,
    pub tool_name: &'static str,
    pub window_minutes: u32,
    pub used_percent: f64,
    pub resets_at: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    pub id: Tool,
    pub name: &'static str,
    pub short_name: &'static str,
}

/// Everything the panel renders. Times are Unix milliseconds.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub loaded: bool,
    pub filter: Option<Tool>,
    pub today: RangeStats,
    pub week: RangeStats,
    /// Claude Code only; None when filtered to another tool or no window is active.
    pub block: Option<Block>,
    pub burn_per_minute: f64,
    pub last_activity: Option<i64>,
    pub unpriced_models: Vec<String>,
    pub limits: Vec<Limit>,
    /// Tools with any usage in the last 7 days, regardless of the filter.
    pub tools: Vec<ToolInfo>,
    /// Today across all tools, for the tray title.
    pub today_all_tools: u64,
    pub block_hours: i64,
}

pub struct Store {
    events: Vec<UsageEvent>,
    limits: HashMap<String, LimitStatus>,
    pricing: Pricing,
    filter: Option<Tool>,
    /// False until the user picks a tab; until then the panel opens on Claude Code
    /// (the "All" view is the tallest) when there is Claude usage to show.
    filter_chosen: bool,
    pub loaded: bool,
}

impl Store {
    pub fn new() -> Self {
        Self { events: Vec::new(), limits: HashMap::new(), pricing: Pricing::load(), filter: None, filter_chosen: false, loaded: false }
    }

    pub fn set_filter(&mut self, filter: Option<Tool>) {
        self.filter = filter;
        self.filter_chosen = true;
    }

    pub fn ingest(&mut self, out: ScanOutput, initial: bool) {
        if initial {
            self.loaded = true;
        }
        self.events.extend(out.events);
        self.events.sort_by_key(|e| e.date);
        for limit in out.limits {
            let id = limit.id();
            if self.limits.get(&id).is_none_or(|old| limit.observed_at >= old.observed_at) {
                self.limits.insert(id, limit);
            }
        }
    }

    pub fn snapshot(&mut self, now: DateTime<Utc>) -> Snapshot {
        self.events.retain(|e| e.date >= now - Duration::days(RETENTION_DAYS));

        let local_now = now.with_timezone(&Local);
        let start_of_today = Local
            .from_local_datetime(&local_now.date_naive().and_hms_opt(0, 0, 0).unwrap())
            .earliest()
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or(now);
        let start_of_week = start_of_today - Duration::days(6);
        let block_len = Duration::hours(BLOCK_HOURS);

        let filter = if self.filter_chosen {
            self.filter
        } else if self.events.iter().any(|e| e.tool == Tool::ClaudeCode && e.date >= start_of_week) {
            Some(Tool::ClaudeCode)
        } else {
            None
        };
        let show_block = matches!(filter, None | Some(Tool::ClaudeCode));

        let mut today = Accumulator::default();
        let mut week = Accumulator::default();
        let mut unpriced = HashSet::new();
        let mut tools = HashSet::new();
        let mut today_all_tools = 0;
        let mut last_activity = None;
        let mut block_start: Option<DateTime<Utc>> = None;
        let mut block_totals = Totals::default();
        let mut burn_tokens = 0;

        for event in &self.events {
            if event.date >= start_of_week {
                tools.insert(event.tool);
            }
            if event.date >= start_of_today {
                today_all_tools += event.total();
            }

            let cost = event.cost.or_else(|| self.pricing.cost(event));

            // Plan windows are a Claude concept; other tools report their own limits.
            if show_block && event.tool == Tool::ClaudeCode {
                match block_start {
                    Some(start) if event.date < start + block_len => block_totals.add(event, cost),
                    _ => {
                        block_start = Some(hour_floor(event.date));
                        block_totals = Totals::default();
                        block_totals.add(event, cost);
                    }
                }
            }

            if filter.is_some_and(|f| f != event.tool) {
                continue;
            }
            if cost.is_none() {
                unpriced.insert(model_name(&event.model));
            }
            last_activity = Some(event.date);
            if event.date >= start_of_week {
                week.add(event, cost);
            }
            if event.date >= start_of_today {
                today.add(event, cost);
            }
            if event.date >= now - Duration::minutes(BURN_WINDOW_MINUTES) {
                burn_tokens += event.total();
            }
        }

        let block = block_start.filter(|s| now < *s + block_len).map(|start| Block {
            start: start.timestamp_millis(),
            end: (start + block_len).timestamp_millis(),
            totals: block_totals,
        });

        let mut limits: Vec<Limit> = self
            .limits
            .values()
            .filter(|l| l.resets_at.is_none_or(|r| r > now) && filter.is_none_or(|f| f == l.tool))
            .map(|l| Limit {
                tool: l.tool,
                tool_name: l.tool.short_name(),
                window_minutes: l.window_minutes,
                used_percent: l.used_percent,
                resets_at: l.resets_at.map(|r| r.timestamp_millis()),
            })
            .collect();
        limits.sort_by_key(|l| (l.tool, l.window_minutes));

        let mut unpriced_models: Vec<String> = unpriced.into_iter().collect();
        unpriced_models.sort();

        Snapshot {
            loaded: self.loaded,
            filter,
            today: today.stats(),
            week: week.stats(),
            block,
            burn_per_minute: burn_tokens as f64 / BURN_WINDOW_MINUTES as f64,
            last_activity: last_activity.map(|d| d.timestamp_millis()),
            unpriced_models,
            limits,
            tools: Tool::ALL
                .into_iter()
                .filter(|t| tools.contains(t))
                .map(|t| ToolInfo { id: t, name: t.name(), short_name: t.short_name() })
                .collect(),
            today_all_tools,
            block_hours: BLOCK_HOURS,
        }
    }
}

/// Top of the local hour (differs from UTC in half-hour time zones).
fn hour_floor(date: DateTime<Utc>) -> DateTime<Utc> {
    let local = date.with_timezone(&Local);
    local
        .with_minute(0)
        .and_then(|d| d.with_second(0))
        .and_then(|d| d.with_nanosecond(0))
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or(date)
}

/// "claude-opus-5-5" → "Opus 5.5", "claude-haiku-4-5-20251001" → "Haiku 4.5".
/// Other vendors' ids are kept as-is, minus any "provider/" prefix.
pub fn model_name(id: &str) -> String {
    let bare = id.rsplit('/').next().unwrap_or(id);
    let Some(rest) = bare.strip_prefix("claude-") else { return bare.to_string() };
    let parts: Vec<&str> = rest
        .split('-')
        .filter(|p| !(p.len() == 8 && p.chars().all(|c| c.is_ascii_digit())))
        .collect();
    let Some((family, version)) = parts.split_first() else { return bare.to_string() };
    let mut name = family[..1].to_uppercase() + &family[1..];
    if !version.is_empty() {
        name.push(' ');
        name.push_str(&version.join("."));
    }
    name
}

#[derive(Default)]
struct Accumulator {
    totals: Totals,
    tools: HashMap<String, Totals>,
    models: HashMap<String, Totals>,
    projects: HashMap<String, Totals>,
}

impl Accumulator {
    fn add(&mut self, e: &UsageEvent, cost: Option<f64>) {
        self.totals.add(e, cost);
        self.tools.entry(e.tool.name().to_string()).or_default().add(e, cost);
        self.models.entry(model_name(&e.model)).or_default().add(e, cost);
        self.projects.entry(e.project.clone()).or_default().add(e, cost);
    }

    fn stats(self) -> RangeStats {
        fn ranked(map: HashMap<String, Totals>) -> Vec<NamedTotals> {
            let mut rows: Vec<NamedTotals> = map.into_iter().map(|(name, totals)| NamedTotals { name, totals }).collect();
            rows.sort_by(|a, b| b.totals.total.cmp(&a.totals.total).then_with(|| a.name.cmp(&b.name)));
            rows
        }
        RangeStats {
            totals: self.totals,
            by_tool: ranked(self.tools),
            by_model: ranked(self.models),
            by_project: ranked(self.projects),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(tool: Tool, minutes_ago: i64, tokens: u64) -> UsageEvent {
        UsageEvent {
            key: format!("{tool:?}{minutes_ago}"),
            date: Utc::now() - Duration::minutes(minutes_ago),
            tool,
            model: "claude-opus-5-5".into(),
            project: "p".into(),
            input: tokens,
            output: 0,
            cache_write: 0,
            cache_read: 0,
            cost: None,
        }
    }

    #[test]
    fn falls_back_to_all_without_claude_usage() {
        let mut store = Store::new();
        store.ingest(ScanOutput { events: vec![event(Tool::Codex, 3, 50)], limits: vec![] }, true);
        assert_eq!(store.snapshot(Utc::now()).filter, None);
    }

    #[test]
    fn model_names() {
        assert_eq!(model_name("claude-opus-5-5"), "Opus 5.5");
        assert_eq!(model_name("claude-haiku-4-5-20251001"), "Haiku 4.5");
        assert_eq!(model_name("deepseek/deepseek-v4.1-flash"), "deepseek-v4.1-flash");
    }

    #[test]
    fn filter_and_block() {
        let mut store = Store::new();
        let out = ScanOutput { events: vec![event(Tool::ClaudeCode, 5, 100), event(Tool::Codex, 3, 50)], limits: vec![] };
        store.ingest(out, true);

        // Opens on Claude Code by default.
        let snap = store.snapshot(Utc::now());
        assert_eq!(snap.filter, Some(Tool::ClaudeCode));
        assert_eq!(snap.week.totals.total, 100);

        store.set_filter(None);
        let snap = store.snapshot(Utc::now());
        assert_eq!(snap.week.totals.total, 150);
        assert_eq!(snap.tools.len(), 2);
        assert_eq!(snap.block.as_ref().unwrap().totals.total, 100);

        store.set_filter(Some(Tool::Codex));
        let snap = store.snapshot(Utc::now());
        assert_eq!(snap.week.totals.total, 50);
        assert!(snap.block.is_none());
        assert_eq!(snap.today_all_tools, 150);
    }
}
