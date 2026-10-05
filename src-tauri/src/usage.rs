//! Usage tab backend: on-demand, metadata-only analysis of the local Claude Code
//! transcripts under `~/.claude/projects/**/*.jsonl`. Reads token counts, model,
//! effort, tool names and error/compaction/rate-limit markers — never prompt text,
//! answers or file contents. Nothing is written anywhere; the report lives only in
//! the open window. The JSONL files already are the database, so there is no
//! collector, no SQLite, no hooks into Claude Code (see CLAUDE.md).

use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::claude_dir;

// ---------------------------------------------------------------------------
// Analyzer rules — all heuristics live here so they can be tuned in one place.
// The weights mirror Anthropic's public API price ratios; how the subscription
// rate limit weights cached tokens is NOT locally observable, so every "share"
// derived from them is labelled "berechnet"/"heuristik" in the UI, never "gemessen".
// ---------------------------------------------------------------------------
const W_INPUT: f64 = 1.0;
const W_CACHE_WRITE: f64 = 1.25;
const W_CACHE_READ: f64 = 0.1;
const W_OUTPUT: f64 = 5.0;

const CTX_LARGE: u64 = 200_000; // upper bucket boundary = "large context"
const RULE_LARGE_CTX_VOLUME_SHARE: f64 = 0.40; // share of context volume from >200k requests
const RULE_AVG_CTX: u64 = 120_000;
const RULE_SESSION_DAYS: f64 = 3.0;
const RULE_SESSION_TURNS: u64 = 100;
const RULE_WORKFLOW_SHARE_HIGH: f64 = 0.20;
const RULE_WORKFLOW_SHARE_MEDIUM: f64 = 0.05;
const RULE_WORKFLOW_AGENTS_PER_SESSION: u64 = 50;
const RULE_XHIGH_SHARE: f64 = 0.40;
const RULE_REQ_PER_TURN: f64 = 8.0;
const TOP_SESSIONS: usize = 12;
const TOP_TOOLS: usize = 10;

// ---------------------------------------------------------------------------
// Report types (camelCase over IPC, 1:1 with api.ts)
// ---------------------------------------------------------------------------

#[derive(Serialize, Default, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct Tokens {
    pub requests: u64,
    pub input: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub output: u64,
}

impl Tokens {
    fn add(&mut self, u: &Usage) {
        self.requests += 1;
        self.input += u.input_tokens;
        self.cache_write += u.cache_creation_input_tokens;
        self.cache_read += u.cache_read_input_tokens;
        self.output += u.output_tokens;
    }
    fn merge(&mut self, o: &Tokens) {
        self.requests += o.requests;
        self.input += o.input;
        self.cache_write += o.cache_write;
        self.cache_read += o.cache_read;
        self.output += o.output;
    }
    fn weighted(&self) -> f64 {
        self.input as f64 * W_INPUT
            + self.cache_write as f64 * W_CACHE_WRITE
            + self.cache_read as f64 * W_CACHE_READ
            + self.output as f64 * W_OUTPUT
    }
    fn context(&self) -> u64 {
        self.input + self.cache_write + self.cache_read
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Named {
    pub name: String,
    pub tokens: Tokens,
    /// Share of the weighted total — heuristic, see weights above.
    pub share_pct: u8,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Bucket {
    pub label: String,
    pub requests: u64,
    pub requests_pct: u8,
    pub volume: u64,
    pub volume_pct: u8,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ToolCount {
    pub name: String,
    pub count: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SessionRow {
    pub id: String,
    pub project: String,
    pub start_day: String,
    pub duration_min: u64,
    pub turns: u64,
    pub requests: u64,
    pub requests_per_turn: f64,
    pub max_context: u64,
    pub subagents: u64,
    pub workflow_agents: u64,
    pub compactions: u64,
    pub errors: u64,
    pub agent_share_pct: u8,
    pub share_pct: u8,
    pub models: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    /// "high" | "medium" | "low" | "info"
    pub severity: String,
    pub title: String,
    pub evidence: String,
    pub recommendation: String,
    /// "gemessen" | "berechnet" | "heuristik"
    pub basis: String,
}

/// One row of the 14-day trend list — always present, even for days with no activity.
#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct DayRow {
    pub day: String,
    pub requests: u64,
    pub avg_context: u64,
    pub turns: u64,
    pub weighted: f64,
}

/// Aggregate over a 7-day window, used for the "letzte 7 Tage vs. 7 Tage davor" comparison.
/// Percentages are of that period's own weighted total (0 when the period had no data).
#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct PeriodStats {
    pub days_active: u64,
    pub requests: u64,
    pub avg_context: u64,
    pub requests_per_turn: f64,
    pub xhigh_pct: u8,
    pub workflow_pct: u8,
    pub five_hour_hits: u64,
    pub weighted: f64,
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Trend {
    pub current: PeriodStats,
    pub previous: PeriodStats,
}

/// A marker that a skill or MCP server was involved in a message — correlation, not cost.
#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct AttributionUsage {
    pub name: String,
    pub messages: u64,
    pub sessions: u64,
    pub last_day: Option<String>,
}

#[derive(Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UsageReport {
    pub files: u64,
    pub unreadable_lines: u64,
    pub first_day: Option<String>,
    pub last_day: Option<String>,
    pub total: Tokens,
    pub by_kind: Vec<Named>,
    pub by_model: Vec<Named>,
    pub by_effort: Vec<Named>,
    pub context_buckets: Vec<Bucket>,
    pub avg_context: u64,
    pub cache_read_pct: u8,
    pub large_context_volume_pct: u8,
    pub api_errors: u64,
    pub tool_errors: u64,
    pub compactions: u64,
    pub five_hour_hits: u64,
    pub last_five_hour_reset: Option<u64>,
    pub main_sessions: u64,
    pub total_turns: u64,
    pub avg_requests_per_turn: f64,
    pub sessions_over_150k: u64,
    pub sessions_compacted: u64,
    pub long_sessions: u64,
    pub longest_session_days: f64,
    pub longest_session_turns: u64,
    pub workflow_share_pct: u8,
    pub workflow_agents: u64,
    pub sessions_with_many_workflow_agents: u64,
    pub max_workflow_agents_in_session: u64,
    pub xhigh_share_pct: u8,
    pub top_tools_main: Vec<ToolCount>,
    pub top_tools_agents: Vec<ToolCount>,
    pub sessions: Vec<SessionRow>,
    pub findings: Vec<Finding>,
    /// Last 14 calendar days ending today, one row per day even if empty.
    pub days: Vec<DayRow>,
    /// Letzte 7 Tage vs. 7 Tage davor, same heuristic weighting as the rest of the tab.
    pub trend: Trend,
    /// From `attributionSkill` transcript markers — correlation, never a cost. Sorted by messages desc.
    pub skill_usage: Vec<AttributionUsage>,
    /// From `attributionMcpServer` transcript markers. Sorted by messages desc.
    pub mcp_usage: Vec<AttributionUsage>,
}

// ---------------------------------------------------------------------------
// Transcript line shape — only the fields we read. serde ignores the rest.
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Line {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    timestamp: String,
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    uuid: Option<String>,
    #[serde(default)]
    effort: Option<String>,
    #[serde(default)]
    prompt_id: Option<String>,
    #[serde(default)]
    message: Option<Msg>,
    #[serde(default)]
    quota_limits: Option<Quota>,
    #[serde(default)]
    is_compact_summary: Option<bool>,
    #[serde(default)]
    compact_metadata: Option<IgnoredAny>,
    #[serde(default)]
    is_api_error_message: Option<bool>,
    #[serde(default)]
    attribution_skill: Option<String>,
    #[serde(default)]
    attribution_mcp_server: Option<String>,
}

#[derive(Deserialize, Default)]
struct Msg {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    usage: Option<Usage>,
    // ponytail: parsed as Value even for huge thinking blocks — fine at ~1 GB of
    // transcripts in release; switch to a streaming visitor if that ever hurts.
    #[serde(default)]
    content: Value,
}

#[derive(Deserialize, Default, Clone, Copy)]
struct Usage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Quota {
    #[serde(default)]
    status: String,
    #[serde(default)]
    rate_limit_type: String,
    #[serde(default)]
    resets_at: Option<u64>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Main,
    Subagent,
    Workflow,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Main => "Hauptsession",
            Kind::Subagent => "Subagent (Agent-Tool)",
            Kind::Workflow => "Workflow-Agent",
        }
    }
}

#[derive(Clone)]
struct FileMeta {
    kind: Kind,
    project: String,
    session: String,
}

/// `<project-dir>/<session>.jsonl` = main; `.../subagents/workflows/<wf>/agent-*.jsonl`
/// = workflow agent; any other nested file = plain subagent.
fn classify(root: &Path, file: &Path) -> Option<FileMeta> {
    let rel = file.strip_prefix(root).ok()?;
    let comps: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    if comps.len() < 2 {
        return None;
    }
    let project = project_label(&comps[0]);
    if comps.len() == 2 {
        return Some(FileMeta {
            kind: Kind::Main,
            project,
            session: comps[1].trim_end_matches(".jsonl").to_string(),
        });
    }
    let kind = if comps.iter().any(|c| c == "workflows") {
        Kind::Workflow
    } else {
        Kind::Subagent
    };
    Some(FileMeta {
        kind,
        project,
        session: comps[1].clone(),
    })
}

/// Project dir names encode the cwd with separators flattened to '-'; the part
/// after the last "Documents-" is what a human recognizes.
pub(crate) fn project_label(dir: &str) -> String {
    dir.rsplit_once("Documents-")
        .map(|(_, r)| r.to_string())
        .unwrap_or_else(|| dir.to_string())
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x == "jsonl") {
            out.push(p);
        }
    }
}

// ---------------------------------------------------------------------------
// Timestamps — ISO 8601 "YYYY-MM-DDTHH:MM:SS(.fff)Z" without pulling in chrono.
// ---------------------------------------------------------------------------

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub(crate) fn parse_ts(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |from: usize, to: usize| s.get(from..to)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, se) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + se)
}

fn day_of(ts: &str) -> Option<String> {
    ts.get(0..10).map(str::to_string)
}

// ---------------------------------------------------------------------------
// Aggregation — fed line by line, no I/O of its own (unit-testable).
// ---------------------------------------------------------------------------

struct Req {
    usage: Usage,
    model: String,
    effort: String,
    kind: Kind,
    session: String,
    day: Option<String>,
}

/// Per-day rollup feeding `trend()`. Turns/five_hour_hits come straight off transcript
/// lines in `feed`; tokens/xhigh_weighted/workflow_weighted come from the deduped
/// requests in `finish` (same "last value per requestId" dedupe as everything else).
#[derive(Default, Clone, Copy)]
pub(crate) struct DayStats {
    pub tokens: Tokens,
    pub turns: u64,
    pub xhigh_weighted: f64,
    pub workflow_weighted: f64,
    pub five_hour_hits: u64,
}

/// Raw accumulator for one attribution marker name (skill or MCP server) before it's
/// turned into the public `AttributionUsage`. `messages` counts every transcript line
/// carrying the marker — verified against a raw grep on real transcripts, these markers
/// repeat once per line of a multi-line response, not once per API request, so this is
/// intentionally NOT deduped by requestId like the token totals above.
#[derive(Default)]
struct AttribUsage {
    messages: u64,
    sessions: HashSet<String>,
    last_day: Option<String>,
}

fn record_attribution(map: &mut HashMap<String, AttribUsage>, name: String, session: &str, day: Option<String>) {
    let e = map.entry(name).or_default();
    e.messages += 1;
    e.sessions.insert(session.to_string());
    if let Some(d) = day {
        if e.last_day.as_ref().is_none_or(|x| d > *x) {
            e.last_day = Some(d);
        }
    }
}

/// Plugin skills are marked `plugin::skill` in transcripts but live on disk as the
/// folder `plugin-skill` (see CLAUDE.md skill-lock note) — normalise so matching by
/// name against the skills list works.
fn normalize_skill_name(raw: &str) -> String {
    match raw.split_once(':') {
        Some((a, b)) => format!("{a}-{b}"),
        None => raw.to_string(),
    }
}

#[derive(Default)]
struct Sess {
    project: String,
    first: Option<i64>,
    last: Option<i64>,
    turns: u64,
    main: Tokens,
    agents: Tokens,
    subagents: u64,
    workflow_agents: u64,
    max_context: u64,
    models: HashMap<String, u64>,
    compactions: u64,
    errors: u64,
}

#[derive(Default)]
struct Aggregator {
    files: u64,
    unreadable: u64,
    requests: HashMap<String, Req>, // requestId → last seen (several lines share one request)
    seen_tool_use: HashSet<String>,
    tools_main: HashMap<String, u64>,
    tools_agents: HashMap<String, u64>,
    sessions: HashMap<String, Sess>,
    api_errors: u64,
    tool_errors: u64,
    compactions: u64,
    five_hour_hits: u64,
    last_five_hour_reset: Option<u64>,
    first_day: Option<String>,
    last_day: Option<String>,
    days: HashMap<String, DayStats>,
    skill_usage: HashMap<String, AttribUsage>,
    mcp_usage: HashMap<String, AttribUsage>,
}

impl Aggregator {
    fn begin_file(&mut self, meta: &FileMeta) {
        self.files += 1;
        let s = self.sessions.entry(meta.session.clone()).or_default();
        if s.project.is_empty() {
            s.project = meta.project.clone();
        }
        match meta.kind {
            Kind::Subagent => s.subagents += 1,
            Kind::Workflow => s.workflow_agents += 1,
            Kind::Main => {}
        }
    }

    fn feed(&mut self, meta: &FileMeta, raw: &str) {
        let line: Line = match serde_json::from_str(raw) {
            Ok(l) => l,
            Err(_) => {
                self.unreadable += 1;
                return;
            }
        };
        let sess = self.sessions.entry(meta.session.clone()).or_default();

        if let Some(day) = day_of(&line.timestamp) {
            if self.first_day.as_ref().is_none_or(|d| day < *d) {
                self.first_day = Some(day.clone());
            }
            if self.last_day.as_ref().is_none_or(|d| day > *d) {
                self.last_day = Some(day);
            }
        }
        if meta.kind == Kind::Main {
            if let Some(ts) = parse_ts(&line.timestamp) {
                sess.first = Some(sess.first.map_or(ts, |f| f.min(ts)));
                sess.last = Some(sess.last.map_or(ts, |l| l.max(ts)));
            }
        }
        if let Some(q) = &line.quota_limits {
            if q.rate_limit_type == "five_hour" && q.status == "rejected" {
                self.five_hour_hits += 1;
                if let Some(r) = q.resets_at {
                    self.last_five_hour_reset = Some(self.last_five_hour_reset.map_or(r, |x| x.max(r)));
                }
                if let Some(day) = day_of(&line.timestamp) {
                    self.days.entry(day).or_default().five_hour_hits += 1;
                }
            }
        }
        if line.is_api_error_message == Some(true) {
            self.api_errors += 1;
        }
        if line.is_compact_summary == Some(true) || line.compact_metadata.is_some() {
            self.compactions += 1;
            sess.compactions += 1;
        }

        // Attribution markers are top-level fields on the line, independent of message
        // shape — read them before the message-dependent logic below, which returns
        // early for several line shapes (user turns, non-assistant lines).
        if line.attribution_skill.is_some() || line.attribution_mcp_server.is_some() {
            let day = day_of(&line.timestamp);
            if let Some(raw) = &line.attribution_skill {
                record_attribution(&mut self.skill_usage, normalize_skill_name(raw), &meta.session, day.clone());
            }
            if let Some(raw) = &line.attribution_mcp_server {
                record_attribution(&mut self.mcp_usage, raw.clone(), &meta.session, day);
            }
        }

        let Some(msg) = line.message else { return };

        if line.kind == "user" {
            match &msg.content {
                Value::String(_) if meta.kind == Kind::Main => {
                    sess.turns += 1;
                    if let Some(day) = day_of(&line.timestamp) {
                        self.days.entry(day).or_default().turns += 1;
                    }
                }
                Value::Array(blocks) => {
                    let mut has_text = false;
                    let mut has_result = false;
                    for b in blocks {
                        match b.get("type").and_then(Value::as_str) {
                            Some("tool_result") => {
                                has_result = true;
                                if b.get("is_error") == Some(&Value::Bool(true)) {
                                    self.tool_errors += 1;
                                    sess.errors += 1;
                                }
                            }
                            Some("text") => has_text = true,
                            _ => {}
                        }
                    }
                    if meta.kind == Kind::Main && has_text && !has_result && line.prompt_id.is_some() {
                        sess.turns += 1;
                        if let Some(day) = day_of(&line.timestamp) {
                            self.days.entry(day).or_default().turns += 1;
                        }
                    }
                }
                _ => {}
            }
            return;
        }

        if line.kind != "assistant" {
            return;
        }
        if let Value::Array(blocks) = &msg.content {
            for b in blocks {
                if b.get("type").and_then(Value::as_str) != Some("tool_use") {
                    continue;
                }
                let (Some(id), Some(name)) = (
                    b.get("id").and_then(Value::as_str),
                    b.get("name").and_then(Value::as_str),
                ) else {
                    continue;
                };
                if self.seen_tool_use.insert(id.to_string()) {
                    let tbl = if meta.kind == Kind::Main { &mut self.tools_main } else { &mut self.tools_agents };
                    *tbl.entry(name.to_string()).or_insert(0) += 1;
                }
            }
        }
        if let Some(usage) = msg.usage {
            let day = day_of(&line.timestamp);
            let key = line.request_id.or(line.uuid).unwrap_or_default();
            if key.is_empty() {
                return;
            }
            self.requests.insert(
                key,
                Req {
                    usage,
                    model: msg.model.unwrap_or_else(|| "?".to_string()),
                    effort: line.effort.unwrap_or_else(|| "unbekannt".to_string()),
                    kind: meta.kind,
                    session: meta.session.clone(),
                    day,
                },
            );
        }
    }

    fn finish(mut self) -> UsageReport {
        let mut total = Tokens::default();
        let mut by_kind: HashMap<Kind, Tokens> = HashMap::new();
        let mut by_model: HashMap<String, Tokens> = HashMap::new();
        let mut by_effort: HashMap<String, Tokens> = HashMap::new();
        let labels = ["<50k", "50–100k", "100–150k", "150–200k", ">200k"];
        let mut bucket_req = [0u64; 5];
        let mut bucket_vol = [0u64; 5];

        let mut days: HashMap<String, DayStats> = std::mem::take(&mut self.days);
        for r in self.requests.values() {
            total.add(&r.usage);
            by_kind.entry(r.kind).or_default().add(&r.usage);
            by_model.entry(r.model.clone()).or_default().add(&r.usage);
            by_effort.entry(r.effort.clone()).or_default().add(&r.usage);
            let ctx = Tokens { input: r.usage.input_tokens, cache_write: r.usage.cache_creation_input_tokens, cache_read: r.usage.cache_read_input_tokens, ..Default::default() }.context();
            let i = match ctx {
                c if c < 50_000 => 0,
                c if c < 100_000 => 1,
                c if c < 150_000 => 2,
                c if c < CTX_LARGE => 3,
                _ => 4,
            };
            bucket_req[i] += 1;
            bucket_vol[i] += ctx;
            if let Some(s) = self.sessions.get_mut(&r.session) {
                if r.kind == Kind::Main { s.main.add(&r.usage) } else { s.agents.add(&r.usage) }
                s.max_context = s.max_context.max(ctx);
                *s.models.entry(r.model.clone()).or_insert(0) += 1;
            }
            if let Some(day) = &r.day {
                let mut one = Tokens::default();
                one.add(&r.usage);
                let ds = days.entry(day.clone()).or_default();
                ds.tokens.merge(&one);
                if r.effort == "xhigh" {
                    ds.xhigh_weighted += one.weighted();
                }
                if r.kind == Kind::Workflow {
                    ds.workflow_weighted += one.weighted();
                }
            }
        }

        let total_w = total.weighted().max(1.0);
        let pct = |part: f64, whole: f64| -> u8 { if whole <= 0.0 { 0 } else { ((100.0 * part / whole).round() as i64).clamp(0, 100) as u8 } };
        let named = |m: HashMap<String, Tokens>| -> Vec<Named> {
            let mut v: Vec<Named> = m
                .into_iter()
                .map(|(name, tokens)| Named { share_pct: pct(tokens.weighted(), total_w), name, tokens })
                .collect();
            v.sort_by(|a, b| b.tokens.weighted().partial_cmp(&a.tokens.weighted()).unwrap_or(std::cmp::Ordering::Equal));
            v
        };
        let by_kind_v: Vec<Named> = [Kind::Main, Kind::Subagent, Kind::Workflow]
            .into_iter()
            .map(|k| {
                let tokens = by_kind.get(&k).copied().unwrap_or_default();
                Named { name: k.label().to_string(), share_pct: pct(tokens.weighted(), total_w), tokens }
            })
            .collect();
        let workflow_share_pct = by_kind_v[2].share_pct;
        let by_effort_v = named(by_effort);
        let xhigh_share_pct = by_effort_v.iter().find(|n| n.name == "xhigh").map(|n| n.share_pct).unwrap_or(0);

        let total_ctx = total.context().max(1);
        let context_buckets: Vec<Bucket> = (0..5)
            .map(|i| Bucket {
                label: labels[i].to_string(),
                requests: bucket_req[i],
                requests_pct: pct(bucket_req[i] as f64, total.requests as f64),
                volume: bucket_vol[i],
                volume_pct: pct(bucket_vol[i] as f64, total_ctx as f64),
            })
            .collect();

        let top_tools = |m: &HashMap<String, u64>| -> Vec<ToolCount> {
            let mut v: Vec<ToolCount> = m.iter().map(|(n, c)| ToolCount { name: n.clone(), count: *c }).collect();
            v.sort_by(|a, b| b.count.cmp(&a.count));
            v.truncate(TOP_TOOLS);
            v
        };
        let top_tools_main = top_tools(&self.tools_main);
        let top_tools_agents = top_tools(&self.tools_agents);

        let mut rows: Vec<(f64, SessionRow)> = Vec::new();
        let mut main_sessions = 0;
        let mut total_turns = 0;
        let mut main_requests = 0;
        let mut sessions_over_150k = 0;
        let mut sessions_compacted = 0;
        let mut long_sessions = 0;
        let mut longest_days = 0.0f64;
        let mut longest_turns = 0;
        let mut many_wf = 0;
        let mut max_wf = 0;
        let mut workflow_agents = 0;
        for (id, s) in self.sessions.drain() {
            workflow_agents += s.workflow_agents;
            max_wf = max_wf.max(s.workflow_agents);
            if s.workflow_agents >= RULE_WORKFLOW_AGENTS_PER_SESSION {
                many_wf += 1;
            }
            if s.main.requests == 0 {
                continue;
            }
            main_sessions += 1;
            total_turns += s.turns;
            main_requests += s.main.requests;
            if s.max_context > 150_000 {
                sessions_over_150k += 1;
            }
            if s.compactions > 0 {
                sessions_compacted += 1;
            }
            let secs = match (s.first, s.last) { (Some(f), Some(l)) if l > f => l - f, _ => 0 };
            let days = secs as f64 / 86_400.0;
            if days >= RULE_SESSION_DAYS || s.turns >= RULE_SESSION_TURNS {
                long_sessions += 1;
            }
            if days > longest_days {
                longest_days = days;
            }
            longest_turns = longest_turns.max(s.turns);

            let mut all = s.main;
            all.merge(&s.agents);
            let w = all.weighted();
            let mut models: Vec<(String, u64)> = s.models.into_iter().collect();
            models.sort_by(|a, b| b.1.cmp(&a.1));
            let models = models
                .iter()
                .map(|(m, n)| format!("{}:{n}", m.trim_start_matches("claude-")))
                .collect::<Vec<_>>()
                .join(" ");
            rows.push((
                w,
                SessionRow {
                    id: id.chars().take(8).collect(),
                    project: s.project,
                    start_day: s.first.map(|f| day_from_epoch(f)).unwrap_or_else(|| "?".to_string()),
                    duration_min: (secs / 60) as u64,
                    turns: s.turns,
                    requests: all.requests,
                    requests_per_turn: if s.turns > 0 { (s.main.requests as f64 / s.turns as f64 * 10.0).round() / 10.0 } else { 0.0 },
                    max_context: s.max_context,
                    subagents: s.subagents,
                    workflow_agents: s.workflow_agents,
                    compactions: s.compactions,
                    errors: s.errors,
                    agent_share_pct: pct(s.agents.weighted(), w),
                    share_pct: pct(w, total_w),
                    models,
                },
            ));
        }
        rows.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let sessions: Vec<SessionRow> = rows.into_iter().take(TOP_SESSIONS).map(|(_, r)| r).collect();

        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let today = day_from_epoch(now_secs);
        let (trend_val, day_rows) = trend(&days, &today);

        let attrib_vec = |m: HashMap<String, AttribUsage>| -> Vec<AttributionUsage> {
            let mut v: Vec<AttributionUsage> = m
                .into_iter()
                .map(|(name, a)| AttributionUsage { name, messages: a.messages, sessions: a.sessions.len() as u64, last_day: a.last_day })
                .collect();
            v.sort_by(|a, b| b.messages.cmp(&a.messages));
            v
        };
        let skill_usage = attrib_vec(self.skill_usage);
        let mcp_usage = attrib_vec(self.mcp_usage);

        let mut report = UsageReport {
            files: self.files,
            unreadable_lines: self.unreadable,
            first_day: self.first_day,
            last_day: self.last_day,
            total,
            by_kind: by_kind_v,
            by_model: named(by_model),
            by_effort: by_effort_v,
            avg_context: if total.requests > 0 { total.context() / total.requests } else { 0 },
            cache_read_pct: pct(total.cache_read as f64, total_ctx as f64),
            large_context_volume_pct: context_buckets[4].volume_pct,
            context_buckets,
            api_errors: self.api_errors,
            tool_errors: self.tool_errors,
            compactions: self.compactions,
            five_hour_hits: self.five_hour_hits,
            last_five_hour_reset: self.last_five_hour_reset,
            main_sessions,
            total_turns,
            avg_requests_per_turn: if total_turns > 0 { (main_requests as f64 / total_turns as f64 * 10.0).round() / 10.0 } else { 0.0 },
            sessions_over_150k,
            sessions_compacted,
            long_sessions,
            longest_session_days: (longest_days * 10.0).round() / 10.0,
            longest_session_turns: longest_turns,
            workflow_share_pct,
            workflow_agents,
            sessions_with_many_workflow_agents: many_wf,
            max_workflow_agents_in_session: max_wf,
            xhigh_share_pct,
            top_tools_main,
            top_tools_agents,
            sessions,
            findings: Vec::new(),
            days: day_rows,
            trend: trend_val,
            skill_usage,
            mcp_usage,
        };
        report.findings = findings(&report);
        report
    }
}

fn day_from_epoch(secs: i64) -> String {
    // inverse of days_from_civil (Howard Hinnant's civil_from_days)
    let z = secs.div_euclid(86_400) + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

fn day_index(s: &str) -> Option<i64> {
    let y = s.get(0..4)?.parse().ok()?;
    let m = s.get(5..7)?.parse().ok()?;
    let d = s.get(8..10)?.parse().ok()?;
    Some(days_from_civil(y, m, d))
}

/// Splits a day→DayStats map into a 14-day row list and a current-vs-previous-7-days
/// trend, both ending at `today` (YYYY-MM-DD). Pure and total: days missing from the
/// map (no activity) contribute zeros, never NaN or a panic.
pub(crate) fn trend(days: &HashMap<String, DayStats>, today: &str) -> (Trend, Vec<DayRow>) {
    let today_idx = day_index(today).unwrap_or(0);

    let rows: Vec<DayRow> = (0..14)
        .rev()
        .map(|back| {
            let day = day_from_epoch((today_idx - back) * 86_400);
            let ds = days.get(&day).copied().unwrap_or_default();
            DayRow {
                requests: ds.tokens.requests,
                avg_context: if ds.tokens.requests > 0 { ds.tokens.context() / ds.tokens.requests } else { 0 },
                turns: ds.turns,
                weighted: ds.tokens.weighted(),
                day,
            }
        })
        .collect();

    let period = |from_back: i64, to_back: i64| -> PeriodStats {
        let mut tokens = Tokens::default();
        let (mut turns, mut xhigh_w, mut workflow_w, mut five_hour, mut days_active) = (0u64, 0.0f64, 0.0f64, 0u64, 0u64);
        for back in from_back..=to_back {
            let day = day_from_epoch((today_idx - back) * 86_400);
            let Some(ds) = days.get(&day) else { continue };
            if ds.tokens.requests > 0 || ds.turns > 0 {
                days_active += 1;
            }
            tokens.merge(&ds.tokens);
            turns += ds.turns;
            xhigh_w += ds.xhigh_weighted;
            workflow_w += ds.workflow_weighted;
            five_hour += ds.five_hour_hits;
        }
        let w = tokens.weighted();
        let pct = |part: f64| -> u8 { if w <= 0.0 { 0 } else { ((100.0 * part / w).round() as i64).clamp(0, 100) as u8 } };
        PeriodStats {
            days_active,
            requests: tokens.requests,
            avg_context: if tokens.requests > 0 { tokens.context() / tokens.requests } else { 0 },
            requests_per_turn: if turns > 0 { (tokens.requests as f64 / turns as f64 * 10.0).round() / 10.0 } else { 0.0 },
            xhigh_pct: pct(xhigh_w),
            workflow_pct: pct(workflow_w),
            five_hour_hits: five_hour,
            weighted: w,
        }
    };

    let trend = Trend { current: period(0, 6), previous: period(7, 13) };
    (trend, rows)
}

// ---------------------------------------------------------------------------
// Deterministic recommendation rules — pure over the report numbers.
// ---------------------------------------------------------------------------

fn k(n: u64) -> String {
    if n >= 1_000_000 { format!("{:.1}M", n as f64 / 1e6) } else if n >= 1_000 { format!("{}k", n / 1_000) } else { n.to_string() }
}

pub(crate) fn findings(r: &UsageReport) -> Vec<Finding> {
    let mut out = Vec::new();
    let f = |severity: &str, title: &str, evidence: String, recommendation: &str, basis: &str| Finding {
        severity: severity.to_string(),
        title: title.to_string(),
        evidence,
        recommendation: recommendation.to_string(),
        basis: basis.to_string(),
    };

    let large_share = r.large_context_volume_pct as f64 / 100.0;
    if large_share >= RULE_LARGE_CTX_VOLUME_SHARE || r.avg_context >= RULE_AVG_CTX {
        out.push(f(
            "high",
            "Kontextgröße ist der größte Treiber",
            format!(
                "{}% des Kontextvolumens stammen aus Requests mit >{} Kontext. Ø {} Tokens pro Request. {} von {} Sessions über 150k, nur {} haben je kompaktiert. Cache-Writes gesamt: {}.",
                r.large_context_volume_pct, k(CTX_LARGE), k(r.avg_context), r.sessions_over_150k, r.main_sessions, r.sessions_compacted, k(r.total.cache_write)
            ),
            "Pro Aufgabe eine neue Session starten. Bei Themenwechsel /clear, bei langen Aufgaben früher /compact. Jeder Request schickt den gesamten Kontext erneut; läuft der Cache in Pausen ab, wird er komplett neu geschrieben.",
            "gemessen",
        ));
    }
    if r.long_sessions > 0 {
        let sev = if r.long_sessions as f64 / r.main_sessions.max(1) as f64 >= 0.3 { "high" } else { "medium" };
        out.push(f(
            sev,
            "Sessions leben zu lange",
            format!(
                "{} von {} Sessions liefen länger als {} Tage oder über {} Turns (längste: {} Tage, {} Turns).",
                r.long_sessions, r.main_sessions, RULE_SESSION_DAYS as u64, RULE_SESSION_TURNS, r.longest_session_days, r.longest_session_turns
            ),
            "Nicht per --continue über Tage weiterarbeiten. Neue Session pro Feature; Zwischenstand in PLAN.md/NOTES.md sichern statt im Kontext.",
            "gemessen",
        ));
    }
    let wf = r.workflow_share_pct as f64 / 100.0;
    if wf >= RULE_WORKFLOW_SHARE_MEDIUM {
        out.push(f(
            if wf >= RULE_WORKFLOW_SHARE_HIGH { "high" } else { "medium" },
            "Workflows / Multi-Agent-Runden",
            format!(
                "{} Workflow-Agents, {}% der gewichteten Nutzung. {} Sessions mit ≥{} Agents (max. {}).",
                r.workflow_agents, r.workflow_share_pct, r.sessions_with_many_workflow_agents, RULE_WORKFLOW_AGENTS_PER_SESSION, r.max_workflow_agents_in_session
            ),
            "Workflows als Ausnahme, nicht als Standard. Mechanische Stufen mit effort 'low'; Recherche-Agents begrenzen statt bis zur Erschöpfung loopen.",
            "berechnet",
        ));
    }
    if r.xhigh_share_pct as f64 / 100.0 >= RULE_XHIGH_SHARE {
        out.push(f(
            "medium",
            "Effort xhigh ist der Standard",
            format!("{}% der gewichteten Nutzung liefen mit effort xhigh.", r.xhigh_share_pct),
            "Effort high als Standard; xhigh gezielt für Planung, Root-Cause und Architektur.",
            "berechnet",
        ));
    }
    if r.avg_requests_per_turn >= RULE_REQ_PER_TURN {
        out.push(f(
            "medium",
            "Viele Requests pro Turn",
            format!(
                "Ø {} API-Requests pro User-Turn. {} fehlgeschlagene Tool-Aufrufe, {} API-Fehler.",
                r.avg_requests_per_turn, r.tool_errors, r.api_errors
            ),
            "Unabhängige Tool-Aufrufe in einem Zug bündeln; fehlgeschlagene Befehle nicht unverändert wiederholen. Jeder Request liest den ganzen Kontext erneut.",
            "gemessen",
        ));
    }
    if r.five_hour_hits > 0 {
        out.push(f(
            "info",
            "5h-Limit erreicht",
            format!("{} Mal laut Transcript-Marker (quotaLimits, five_hour, rejected).", r.five_hour_hits),
            "Eine Live-Prozentanzeige des Limits gibt es lokal nicht — nur diese Treffer und ihre Reset-Zeit.",
            "gemessen",
        ));
    }
    if out.iter().all(|x| x.severity == "info") {
        out.push(f("info", "Keine auffälligen Treiber", "Alle Regeln unter ihren Schwellen.".to_string(), "Nichts zu tun.", "heuristik"));
    }
    out
}

// ---------------------------------------------------------------------------
// Command
// ---------------------------------------------------------------------------

#[tauri::command(async)]
pub fn analyze_usage() -> Result<UsageReport, String> {
    let root = claude_dir()?.join("projects");
    if !root.is_dir() {
        return Err("~/.claude/projects existiert nicht — noch keine Claude-Code-Sessions vorhanden.".to_string());
    }
    let mut files = Vec::new();
    walk(&root, &mut files);
    let mut agg = Aggregator::default();
    for file in &files {
        let Some(meta) = classify(&root, file) else { continue };
        agg.begin_file(&meta);
        let Ok(fh) = File::open(file) else { continue };
        for line in BufReader::new(fh).split(b'\n').flatten() {
            if line.is_empty() {
                continue;
            }
            // Cheap prefilter: only lines carrying something we read get parsed.
            let s = String::from_utf8_lossy(&line);
            if !(s.contains("\"usage\"") || s.contains("\"promptId\"") || s.contains("\"is_error\":true")
                || s.contains("quotaLimits") || s.contains("ompact") || s.contains("isApiErrorMessage")
                || s.contains("attributionSkill") || s.contains("attributionMcpServer"))
            {
                continue;
            }
            agg.feed(&meta, &s);
        }
    }
    Ok(agg.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(kind: Kind) -> FileMeta {
        FileMeta { kind, project: "P".into(), session: "s1".into() }
    }

    #[test]
    fn timestamps_parse_and_roundtrip_days() {
        assert_eq!(parse_ts("2026-01-01T00:00:00.000Z"), Some(1_767_225_600));
        assert_eq!(day_from_epoch(1_767_225_600), "2026-01-01");
        assert_eq!(day_from_epoch(parse_ts("2026-07-24T10:53:09.720Z").unwrap()), "2026-07-24");
        assert_eq!(parse_ts("garbage"), None);
    }

    #[test]
    fn project_label_strips_flattened_home_prefix() {
        assert_eq!(project_label("C--Users-you-Documents-Claude-Skill-switch"), "Claude-Skill-switch");
        assert_eq!(project_label("something-else"), "something-else");
    }

    #[test]
    fn requests_are_deduplicated_by_request_id_and_turns_counted() {
        let mut a = Aggregator::default();
        let m = meta(Kind::Main);
        a.begin_file(&m);
        a.feed(&m, r#"{"type":"user","timestamp":"2026-07-24T10:00:00Z","promptId":"p1","message":{"role":"user","content":"hallo"}}"#);
        a.feed(&m, r#"{"type":"assistant","timestamp":"2026-07-24T10:00:01Z","requestId":"r1","effort":"xhigh","message":{"model":"claude-opus-5","usage":{"input_tokens":10,"cache_creation_input_tokens":100,"cache_read_input_tokens":1000,"output_tokens":5},"content":[{"type":"tool_use","id":"t1","name":"Bash"}]}}"#);
        a.feed(&m, r#"{"type":"assistant","timestamp":"2026-07-24T10:00:02Z","requestId":"r1","effort":"xhigh","message":{"model":"claude-opus-5","usage":{"input_tokens":10,"cache_creation_input_tokens":100,"cache_read_input_tokens":1000,"output_tokens":5},"content":[{"type":"tool_use","id":"t1","name":"Bash"}]}}"#);
        a.feed(&m, r#"{"type":"user","timestamp":"2026-07-24T10:00:03Z","message":{"role":"user","content":[{"type":"tool_result","is_error":true}]}}"#);
        a.feed(&m, r#"{"type":"user","timestamp":"2026-07-24T10:00:04Z","quotaLimits":{"status":"rejected","rateLimitType":"five_hour","resetsAt":123}}"#);
        a.feed(&m, "not json");
        let r = a.finish();
        assert_eq!(r.total.requests, 1, "two lines with the same requestId are one request");
        assert_eq!(r.total.cache_read, 1000);
        assert_eq!(r.total_turns, 1, "tool_result lines are not user turns");
        assert_eq!(r.tool_errors, 1);
        assert_eq!(r.five_hour_hits, 1);
        assert_eq!(r.last_five_hour_reset, Some(123));
        assert_eq!(r.unreadable_lines, 1);
        assert_eq!(r.top_tools_main[0].count, 1, "tool_use ids are deduplicated");
        assert_eq!(r.by_kind[0].tokens.requests, 1);
        assert_eq!(r.sessions[0].models, "opus-5:1");
    }

    #[test]
    fn attribution_markers_are_counted_per_line_with_normalized_plugin_names_and_distinct_sessions() {
        let mut a = Aggregator::default();
        let m1 = FileMeta { kind: Kind::Main, project: "P".into(), session: "s1".into() };
        let m2 = FileMeta { kind: Kind::Main, project: "P".into(), session: "s2".into() };
        a.begin_file(&m1);
        a.begin_file(&m2);
        // Same requestId, two lines — verified against real transcripts that the marker
        // repeats per line, not per API request, so both lines must count separately.
        a.feed(&m1, r#"{"type":"assistant","timestamp":"2026-09-01T10:00:00Z","requestId":"r1","attributionSkill":"claude-api","message":{"model":"claude-opus-5","usage":{"input_tokens":1,"output_tokens":1},"content":[]}}"#);
        a.feed(&m1, r#"{"type":"assistant","timestamp":"2026-09-01T10:00:01Z","requestId":"r1","attributionSkill":"claude-api","message":{"model":"claude-opus-5","usage":{"input_tokens":1,"output_tokens":1},"content":[]}}"#);
        a.feed(&m1, r#"{"type":"assistant","timestamp":"2026-09-02T10:00:00Z","uuid":"u2","attributionSkill":"anthropic-skills:pdf","attributionMcpServer":"Claude Browser","message":{"model":"claude-opus-5","usage":{"input_tokens":1,"output_tokens":1},"content":[]}}"#);
        a.feed(&m2, r#"{"type":"assistant","timestamp":"2026-09-03T10:00:00Z","uuid":"u3","attributionSkill":"anthropic-skills:pdf","message":{"model":"claude-opus-5","usage":{"input_tokens":1,"output_tokens":1},"content":[]}}"#);
        let r = a.finish();
        let pdf = r.skill_usage.iter().find(|s| s.name == "anthropic-skills-pdf").expect("plugin marker normalised to folder form");
        assert_eq!(pdf.messages, 2);
        assert_eq!(pdf.sessions, 2, "two distinct sessions");
        assert_eq!(pdf.last_day.as_deref(), Some("2026-09-03"));
        let api = r.skill_usage.iter().find(|s| s.name == "claude-api").unwrap();
        assert_eq!(api.messages, 2, "two lines sharing a requestId both count — not deduped");
        assert_eq!(api.sessions, 1);
        let browser = r.mcp_usage.iter().find(|s| s.name == "Claude Browser").unwrap();
        assert_eq!(browser.messages, 1);
    }

    #[test]
    fn trend_splits_current_and_previous_week_with_zeros_for_empty_previous() {
        let today = "2026-09-06";
        let today_idx = day_index(today).unwrap();
        let mut days: HashMap<String, DayStats> = HashMap::new();
        for back in 0..7 {
            let day = day_from_epoch((today_idx - back) * 86_400);
            days.insert(
                day,
                DayStats {
                    tokens: Tokens { requests: 10, input: 100_000, ..Default::default() },
                    turns: 2,
                    xhigh_weighted: 50_000.0,
                    workflow_weighted: 0.0,
                    five_hour_hits: 1,
                },
            );
        }
        let (t, rows) = trend(&days, today);
        assert_eq!(rows.len(), 14, "always 14 rows, even for empty days");
        assert_eq!(rows[13].day, today, "last row is today");
        assert_eq!(rows[0].day, day_from_epoch((today_idx - 13) * 86_400), "first row is 13 days back");
        assert_eq!(rows[6].requests, 0, "the previous-period days have no data and stay zero, not missing");

        assert_eq!(t.current.days_active, 7);
        assert_eq!(t.current.requests, 70);
        assert_eq!(t.current.avg_context, 10_000);
        assert_eq!(t.current.requests_per_turn, 5.0);
        assert_eq!(t.current.five_hour_hits, 7);
        assert!(t.current.xhigh_pct > 0);

        assert_eq!(t.previous.days_active, 0, "no activity 8-14 days back");
        assert_eq!(t.previous.requests, 0);
        assert_eq!(t.previous.avg_context, 0, "no division by zero");
        assert_eq!(t.previous.requests_per_turn, 0.0);
        assert_eq!(t.previous.xhigh_pct, 0);
        assert_eq!(t.previous.workflow_pct, 0);
        assert_eq!(t.previous.weighted, 0.0);
    }

    fn base_report() -> UsageReport {
        UsageReport { main_sessions: 5, avg_requests_per_turn: 4.0, ..Default::default() }
    }

    #[test]
    fn efficient_profile_yields_no_high_findings() {
        let mut r = base_report();
        r.avg_context = 70_000;
        r.large_context_volume_pct = 10;
        r.xhigh_share_pct = 10;
        r.workflow_share_pct = 0;
        let f = findings(&r);
        assert!(f.iter().all(|x| x.severity == "info"), "{f:?}");
    }

    #[test]
    fn large_context_and_workflows_are_flagged_high() {
        let mut r = base_report();
        r.avg_context = 300_000;
        r.large_context_volume_pct = 72;
        r.workflow_share_pct = 37;
        r.xhigh_share_pct = 65;
        r.avg_requests_per_turn = 10.5;
        let f = findings(&r);
        let sev = |t: &str| f.iter().find(|x| x.title.starts_with(t)).map(|x| x.severity.clone());
        assert_eq!(sev("Kontextgröße").as_deref(), Some("high"));
        assert_eq!(sev("Workflows").as_deref(), Some("high"));
        assert_eq!(sev("Effort").as_deref(), Some("medium"));
        assert_eq!(sev("Viele Requests").as_deref(), Some("medium"));
        assert!(f.iter().all(|x| !x.evidence.is_empty() && !x.recommendation.is_empty()));
    }

    /// Runs the real analysis over this machine's transcripts. Ignored by default
    /// (slow, machine-specific); `cargo test real_report -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_report_over_local_transcripts() {
        let t = std::time::Instant::now();
        let r = analyze_usage().expect("analysis");
        println!(
            "{} files, {} requests, avg ctx {}, xhigh {}%, workflow {}%, 5h hits {}, sessions {} — {:?}",
            r.files, r.total.requests, r.avg_context, r.xhigh_share_pct, r.workflow_share_pct, r.five_hour_hits, r.main_sessions, t.elapsed()
        );
        for f in &r.findings {
            println!("[{}] {} — {}", f.severity, f.title, f.evidence);
        }
        println!(
            "trend current: {} req, avg ctx {}, req/turn {}, xhigh {}%, workflow {}%, 5h {} ({} Tage aktiv)",
            r.trend.current.requests, r.trend.current.avg_context, r.trend.current.requests_per_turn,
            r.trend.current.xhigh_pct, r.trend.current.workflow_pct, r.trend.current.five_hour_hits, r.trend.current.days_active
        );
        println!(
            "trend previous: {} req, avg ctx {}, req/turn {}, xhigh {}%, workflow {}%, 5h {} ({} Tage aktiv)",
            r.trend.previous.requests, r.trend.previous.avg_context, r.trend.previous.requests_per_turn,
            r.trend.previous.xhigh_pct, r.trend.previous.workflow_pct, r.trend.previous.five_hour_hits, r.trend.previous.days_active
        );
        println!("skill_usage top 5:");
        for s in r.skill_usage.iter().take(5) {
            println!("  {} — {} Nachrichten, {} Sessions, zuletzt {:?}", s.name, s.messages, s.sessions, s.last_day);
        }
        println!("mcp_usage top 5:");
        for s in r.mcp_usage.iter().take(5) {
            println!("  {} — {} Nachrichten, {} Sessions, zuletzt {:?}", s.name, s.messages, s.sessions, s.last_day);
        }
        assert!(r.total.requests > 0);
        assert_eq!(r.days.len(), 14);
    }

    #[test]
    fn a_single_small_workflow_does_not_trigger_the_rule() {
        let mut r = base_report();
        r.workflow_share_pct = 3;
        assert!(findings(&r).iter().all(|x| !x.title.starts_with("Workflows")));
    }
}
