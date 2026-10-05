//! Live tab: which Claude Code sessions are active right now (Desktop app and CLI
//! write the same transcripts — 99 % of this machine's lines say
//! `entrypoint: claude-desktop`), how big their context currently is, and what to do
//! before the next prompt. The frontend polls; every poll reads only the bytes
//! appended since the last one (per-session cursor in managed state), so polling
//! stays cheap on 100 MB transcripts. Nothing is stored; a session's cursor lives
//! only as long as the app window.
//!
//! Honest limits: there is no documented way to trigger `/compact` or `/clear` inside
//! a running session from outside, and no local live percentage of the 5h limit.
//! So advice is text plus a copyable command, and the limit shows only as "hit,
//! resets in …" when the transcript recorded a rejection.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;

use crate::claude_dir;
use crate::usage::{parse_ts, project_label};

// Rules — same style as usage.rs: one block, tune here.
const ACTIVE_SECS: i64 = 10 * 60; // written within 10 min = active
const SHOW_SECS: i64 = 6 * 60 * 60; // listed (as idle) up to 6 h
const CTX_COMPACT_NOW: u64 = 400_000;
const CTX_COMPACT_SOON: u64 = 200_000;
const RULE_TURN_REQUESTS: u64 = 15;
const RULE_TURN_ERRORS: u64 = 3;
const RULE_AGE_DAYS: f64 = 3.0;
const RULE_TURNS: u64 = 100;
const MAX_CHUNK: u64 = 64 * 1024 * 1024; // never read more than this per poll

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Advice {
    /// "high" | "medium" | "ok" | "info"
    pub level: String,
    pub text: String,
    /// Ready-to-paste slash command, if one applies.
    pub command: Option<String>,
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct LiveSession {
    pub id: String,
    pub title: Option<String>,
    pub project: String,
    pub cwd: Option<String>,
    /// "Desktop" | "Terminal" | "?"
    pub entrypoint: String,
    pub active: bool,
    pub seconds_since_activity: i64,
    pub age_secs: i64,
    pub turns: u64,
    pub requests: u64,
    pub turn_requests: u64,
    pub turn_errors: u64,
    pub compactions: u64,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Context the last request actually read: input + cache read + cache write.
    pub context: u64,
    pub limit_reset_in_secs: Option<i64>,
    pub agents_active: u64,
    pub advice: Vec<Advice>,
}

#[derive(Default, Clone)]
struct Cursor {
    scanned: u64,
    first_ts: Option<i64>,
    last_ts: Option<i64>,
    title: Option<String>,
    entrypoint: Option<String>,
    cwd: Option<String>,
    turns: u64,
    requests: u64,
    last_request_id: Option<String>,
    turn_requests: u64,
    turn_errors: u64,
    compactions: u64,
    model: Option<String>,
    effort: Option<String>,
    context: u64,
    limit_reset_at: Option<u64>,
}

#[derive(Default)]
pub struct LiveState(Mutex<HashMap<PathBuf, Cursor>>);

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
    effort: Option<String>,
    #[serde(default)]
    prompt_id: Option<String>,
    #[serde(default)]
    entrypoint: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    custom_title: Option<String>,
    #[serde(default)]
    ai_title: Option<String>,
    #[serde(default)]
    message: Option<Msg>,
    #[serde(default)]
    quota_limits: Option<Quota>,
    #[serde(default)]
    is_compact_summary: Option<bool>,
    #[serde(default)]
    compact_metadata: Option<serde::de::IgnoredAny>,
}

#[derive(Deserialize, Default)]
struct Msg {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    usage: Option<Usage>,
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

impl Cursor {
    /// Consume one complete transcript line. Pure over the cursor — unit-tested.
    fn feed(&mut self, raw: &str) {
        let Ok(line) = serde_json::from_str::<Line>(raw) else { return };
        if let Some(ts) = parse_ts(&line.timestamp) {
            if self.first_ts.is_none() {
                self.first_ts = Some(ts);
            }
            self.last_ts = Some(ts);
        }
        if let Some(t) = line.custom_title.or(line.ai_title) {
            if !t.trim().is_empty() {
                self.title = Some(t);
            }
        }
        if self.entrypoint.is_none() {
            self.entrypoint = line.entrypoint;
        }
        if self.cwd.is_none() {
            self.cwd = line.cwd;
        }
        if let Some(q) = &line.quota_limits {
            if q.rate_limit_type == "five_hour" && q.status == "rejected" {
                self.limit_reset_at = q.resets_at.or(self.limit_reset_at);
            }
        }
        if line.is_compact_summary == Some(true) || line.compact_metadata.is_some() {
            self.compactions += 1;
        }
        let Some(msg) = line.message else { return };
        match line.kind.as_str() {
            "user" => {
                let is_prompt = match &msg.content {
                    Value::String(_) => true,
                    Value::Array(blocks) => {
                        let mut text = false;
                        let mut result = false;
                        for b in blocks {
                            match b.get("type").and_then(Value::as_str) {
                                Some("text") => text = true,
                                Some("tool_result") => {
                                    result = true;
                                    if b.get("is_error") == Some(&Value::Bool(true)) {
                                        self.turn_errors += 1;
                                    }
                                }
                                _ => {}
                            }
                        }
                        text && !result && line.prompt_id.is_some()
                    }
                    _ => false,
                };
                if is_prompt {
                    self.turns += 1;
                    self.turn_requests = 0;
                    self.turn_errors = 0;
                }
            }
            "assistant" => {
                if let Some(u) = msg.usage {
                    // Several lines share one requestId (one per content block) and
                    // they are consecutive — count a request when the id changes.
                    if line.request_id.is_some() && line.request_id != self.last_request_id {
                        self.requests += 1;
                        self.turn_requests += 1;
                        self.last_request_id = line.request_id;
                    }
                    self.context = u.input_tokens + u.cache_read_input_tokens + u.cache_creation_input_tokens;
                }
                if msg.model.is_some() {
                    self.model = msg.model;
                }
                if line.effort.is_some() {
                    self.effort = line.effort;
                }
            }
            _ => {}
        }
    }

    /// Read whatever was appended since the last poll. Only complete lines are
    /// consumed; a line still being written stays for the next poll.
    fn catch_up(&mut self, path: &Path) -> Result<(), String> {
        let len = fs::metadata(path).map_err(|e| e.to_string())?.len();
        if len < self.scanned {
            *self = Cursor::default(); // truncated/rewritten — start over
        }
        if len == self.scanned {
            return Ok(());
        }
        let mut f = File::open(path).map_err(|e| e.to_string())?;
        f.seek(SeekFrom::Start(self.scanned)).map_err(|e| e.to_string())?;
        let want = (len - self.scanned).min(MAX_CHUNK);
        let mut buf = Vec::with_capacity(want as usize);
        f.take(want).read_to_end(&mut buf).map_err(|e| e.to_string())?;
        let Some(last_nl) = buf.iter().rposition(|&b| b == b'\n') else {
            // No newline: either a line still being written (wait for it) or a single
            // line bigger than MAX_CHUNK — skip that one rather than re-reading the
            // same window forever and freezing this session's cursor.
            if buf.len() as u64 >= MAX_CHUNK {
                self.scanned = len;
            }
            return Ok(());
        };
        let text = String::from_utf8_lossy(&buf[..last_nl]);
        for raw in text.split('\n') {
            let raw = raw.trim_end_matches('\r');
            if !raw.is_empty() {
                self.feed(raw);
            }
        }
        self.scanned += last_nl as u64 + 1;
        Ok(())
    }
}

pub(crate) fn advice(s: &LiveSession) -> Vec<Advice> {
    let mut out = Vec::new();
    let a = |level: &str, text: String, command: Option<&str>| Advice {
        level: level.to_string(),
        text,
        command: command.map(str::to_string),
    };
    let k = |n: u64| format!("{}k", n / 1_000);

    // Compact advice is always present — the user asked for it as the standing nudge.
    if s.context >= CTX_COMPACT_NOW {
        out.push(a(
            "high",
            format!("Kontext {} — jetzt kompaktieren oder die nächste Aufgabe in einer neuen Session starten. Jeder Request liest das alles erneut.", k(s.context)),
            Some("/compact focus on "),
        ));
    } else if s.context >= CTX_COMPACT_SOON {
        out.push(a(
            "medium",
            format!("Kontext {} — vor der nächsten größeren Aufgabe kompaktieren.", k(s.context)),
            Some("/compact focus on "),
        ));
    } else {
        out.push(a("ok", format!("Kontext {} — unkritisch. Bei Themenwechsel trotzdem /clear.", k(s.context)), Some("/clear")));
    }

    if let Some(secs) = s.limit_reset_in_secs {
        out.push(a(
            "info",
            format!("5h-Limit erreicht — Reset in {} min.", (secs / 60).max(1)),
            None,
        ));
    }
    if s.turn_requests >= RULE_TURN_REQUESTS || s.turn_errors >= RULE_TURN_ERRORS {
        out.push(a(
            "medium",
            format!(
                "Aktueller Turn: {} Requests, {} fehlgeschlagene Tool-Aufrufe — sieht nach einer Schleife aus. Ansatz ändern statt wiederholen lassen.",
                s.turn_requests, s.turn_errors
            ),
            None,
        ));
    }
    let days = s.age_secs as f64 / 86_400.0;
    if days >= RULE_AGE_DAYS || s.turns >= RULE_TURNS {
        out.push(a(
            "medium",
            format!("Session ist {:.0} Tage alt ({} Turns). Neue Session pro Aufgabe; Zwischenstand in eine Datei sichern.", days, s.turns),
            None,
        ));
    }
    if s.effort.as_deref() == Some("xhigh") {
        out.push(a("info", "Effort xhigh aktiv — für Implementierung reicht meist high.".to_string(), None));
    }
    if s.agents_active > 0 {
        out.push(a("info", format!("{} Agent(s) schreiben gerade — jeder mit eigenem Kontext.", s.agents_active), None));
    }
    out
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn mtime_secs(p: &Path) -> Option<i64> {
    fs::metadata(p).ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs() as i64)
}

fn count_active_agents(session_dir: &Path, now: i64) -> u64 {
    fn walk(dir: &Path, now: i64, n: &mut u64) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, now, n);
            } else if p.extension().is_some_and(|x| x == "jsonl") && mtime_secs(&p).is_some_and(|m| now - m <= ACTIVE_SECS) {
                *n += 1;
            }
        }
    }
    let mut n = 0;
    walk(&session_dir.join("subagents"), now, &mut n);
    n
}

#[tauri::command(async)]
pub fn list_live_sessions(state: State<LiveState>) -> Result<Vec<LiveSession>, String> {
    let root = claude_dir()?.join("projects");
    let mut cursors = state.0.lock().map_err(|_| "Interner Sperr-Fehler.".to_string())?;
    collect(&root, &mut cursors, now_secs())
}

/// The command minus Tauri plumbing, so it can run against the real transcripts in
/// a test with a throwaway cursor map.
fn collect(root: &Path, cursors: &mut HashMap<PathBuf, Cursor>, now: i64) -> Result<Vec<LiveSession>, String> {
    let mut out = Vec::new();
    let Ok(projects) = fs::read_dir(root) else { return Ok(out) };
    for proj in projects.flatten() {
        let proj_path = proj.path();
        if !proj_path.is_dir() {
            continue;
        }
        let project = project_label(&proj.file_name().to_string_lossy());
        let Ok(files) = fs::read_dir(&proj_path) else { continue };
        for f in files.flatten() {
            let path = f.path();
            if !path.is_file() || path.extension().is_none_or(|x| x != "jsonl") {
                continue;
            }
            let Some(mtime) = mtime_secs(&path) else { continue };
            if now - mtime > SHOW_SECS {
                continue;
            }
            let cur = cursors.entry(path.clone()).or_default();
            // One locked/unreadable file must not blank every other session's card.
            if cur.catch_up(&path).is_err() {
                continue;
            }
            if cur.requests == 0 && cur.turns == 0 {
                continue; // title-only or empty file
            }
            let id = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let session_dir = proj_path.join(&id);
            let last = cur.last_ts.unwrap_or(mtime);
            let mut s = LiveSession {
                id: id.chars().take(8).collect(),
                title: cur.title.clone(),
                project: project.clone(),
                cwd: cur.cwd.clone(),
                entrypoint: match cur.entrypoint.as_deref() {
                    Some("claude-desktop") => "Desktop".to_string(),
                    Some("cli") => "Terminal".to_string(),
                    Some(other) => other.to_string(),
                    None => "?".to_string(),
                },
                active: now - mtime <= ACTIVE_SECS,
                seconds_since_activity: (now - mtime).max(0),
                age_secs: cur.first_ts.map(|f| (last - f).max(0)).unwrap_or(0),
                turns: cur.turns,
                requests: cur.requests,
                turn_requests: cur.turn_requests,
                turn_errors: cur.turn_errors,
                compactions: cur.compactions,
                model: cur.model.clone(),
                effort: cur.effort.clone(),
                context: cur.context,
                limit_reset_in_secs: cur.limit_reset_at.map(|r| r as i64 - now).filter(|d| *d > 0),
                agents_active: count_active_agents(&session_dir, now),
                advice: Vec::new(),
            };
            s.advice = advice(&s);
            out.push(s);
        }
    }
    // Drop cursors of files that fell out of the window so the map can't grow forever.
    cursors.retain(|p, _| mtime_secs(p).is_some_and(|m| now - m <= SHOW_SECS));
    out.sort_by(|a, b| a.seconds_since_activity.cmp(&b.seconds_since_activity));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_tracks_turns_requests_context_and_title() {
        let mut c = Cursor::default();
        c.feed(r#"{"type":"custom-title","customTitle":"Mein Thema","sessionId":"x"}"#);
        c.feed(r#"{"type":"user","timestamp":"2026-09-06T01:00:00Z","entrypoint":"claude-desktop","cwd":"C:\\p","promptId":"p1","message":{"role":"user","content":"hallo"}}"#);
        c.feed(r#"{"type":"assistant","timestamp":"2026-09-06T01:00:01Z","requestId":"r1","effort":"xhigh","message":{"model":"claude-opus-5","usage":{"input_tokens":5,"cache_creation_input_tokens":1000,"cache_read_input_tokens":300000},"content":[{"type":"text","text":"a"}]}}"#);
        c.feed(r#"{"type":"assistant","timestamp":"2026-09-06T01:00:02Z","requestId":"r1","effort":"xhigh","message":{"model":"claude-opus-5","usage":{"input_tokens":5,"cache_creation_input_tokens":1000,"cache_read_input_tokens":300000},"content":[{"type":"tool_use","id":"t","name":"Bash"}]}}"#);
        c.feed(r#"{"type":"user","timestamp":"2026-09-06T01:00:03Z","message":{"role":"user","content":[{"type":"tool_result","is_error":true}]}}"#);
        c.feed(r#"{"type":"assistant","timestamp":"2026-09-06T01:00:04Z","requestId":"r2","message":{"model":"claude-opus-5","usage":{"input_tokens":1,"cache_creation_input_tokens":0,"cache_read_input_tokens":310000},"content":[]}}"#);
        c.feed(r#"{"type":"user","timestamp":"2026-09-06T01:05:00Z","quotaLimits":{"status":"rejected","rateLimitType":"five_hour","resetsAt":999}}"#);
        assert_eq!(c.turns, 1);
        assert_eq!(c.requests, 2, "same requestId on consecutive lines counts once");
        assert_eq!(c.turn_requests, 2);
        assert_eq!(c.turn_errors, 1);
        assert_eq!(c.context, 310_001);
        assert_eq!(c.title.as_deref(), Some("Mein Thema"));
        assert_eq!(c.entrypoint.as_deref(), Some("claude-desktop"));
        assert_eq!(c.effort.as_deref(), Some("xhigh"));
        assert_eq!(c.limit_reset_at, Some(999));
        // next prompt resets the per-turn counters
        c.feed(r#"{"type":"user","timestamp":"2026-09-06T01:06:00Z","promptId":"p2","message":{"role":"user","content":[{"type":"text","text":"weiter"}]}}"#);
        assert_eq!((c.turns, c.turn_requests, c.turn_errors), (2, 0, 0));
    }

    #[test]
    fn catch_up_consumes_only_complete_lines_and_resumes() {
        let dir = std::env::temp_dir().join(format!("skillswitch-live-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.jsonl");
        let l1 = r#"{"type":"user","timestamp":"2026-09-06T01:00:00Z","promptId":"p1","message":{"role":"user","content":"a"}}"#;
        let l2 = r#"{"type":"assistant","timestamp":"2026-09-06T01:00:01Z","requestId":"r1","message":{"model":"m","usage":{"input_tokens":1,"cache_creation_input_tokens":0,"cache_read_input_tokens":0},"content":[]}}"#;
        fs::write(&p, format!("{l1}\n{{\"type\":\"user\",\"partial")).unwrap();
        let mut c = Cursor::default();
        c.catch_up(&p).unwrap();
        assert_eq!(c.turns, 1);
        assert_eq!(c.scanned as usize, l1.len() + 1, "partial trailing line is left for later");
        fs::write(&p, format!("{l1}\n{l2}\n")).unwrap();
        c.catch_up(&p).unwrap();
        assert_eq!(c.requests, 1);
        assert_eq!(c.scanned as usize, l1.len() + 1 + l2.len() + 1);
        fs::write(&p, "").unwrap(); // truncated → cursor resets
        c.catch_up(&p).unwrap();
        assert_eq!(c.turns, 0);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Real transcripts on this machine; `cargo test --release real_live -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_live_sessions_on_this_machine() {
        let root = claude_dir().unwrap().join("projects");
        let mut cursors = HashMap::new();
        let t = std::time::Instant::now();
        let first = collect(&root, &mut cursors, now_secs()).unwrap();
        let cold = t.elapsed();
        let t = std::time::Instant::now();
        let second = collect(&root, &mut cursors, now_secs()).unwrap();
        let warm = t.elapsed();
        println!("cold {cold:?} → {} sessions, warm {warm:?} → {} sessions", first.len(), second.len());
        for s in &second {
            println!(
                "{} {:<22} {:<8} active={} ago={}s ctx={} turns={} req={}/{} err={} age={}s agents={} advice={}",
                s.id, s.project, s.entrypoint, s.active, s.seconds_since_activity, s.context, s.turns, s.turn_requests, s.requests, s.turn_errors, s.age_secs, s.agents_active, s.advice.len()
            );
        }
        assert!(warm < cold);
    }

    fn sess(context: u64) -> LiveSession {
        LiveSession { context, ..Default::default() }
    }

    #[test]
    fn compact_advice_is_always_present_and_scales_with_context() {
        let lvl = |ctx: u64| advice(&sess(ctx))[0].level.clone();
        assert_eq!(lvl(50_000), "ok");
        assert_eq!(lvl(250_000), "medium");
        assert_eq!(lvl(700_000), "high");
        assert_eq!(advice(&sess(700_000))[0].command.as_deref(), Some("/compact focus on "));
    }

    #[test]
    fn loops_age_and_limit_are_flagged_but_quiet_sessions_are_not() {
        let quiet = advice(&sess(50_000));
        assert_eq!(quiet.len(), 1, "{quiet:?}");
        let mut s = sess(50_000);
        s.turn_requests = 20;
        s.turn_errors = 4;
        s.age_secs = 5 * 86_400;
        s.turns = 3;
        s.limit_reset_in_secs = Some(600);
        s.effort = Some("xhigh".into());
        let texts: Vec<String> = advice(&s).iter().map(|a| a.text.clone()).collect();
        assert!(texts.iter().any(|t| t.contains("Schleife")));
        assert!(texts.iter().any(|t| t.contains("Tage alt")));
        assert!(texts.iter().any(|t| t.contains("Reset in 10 min")));
        assert!(texts.iter().any(|t| t.contains("xhigh")));
    }
}
