//! Shared command context: config, session, agent resolution, output helpers.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::api::{Agent, ListAgentsRequest, ListAgentsResponse, SVC};
use crate::auth::{self, Session};
use crate::connect::RpcError;

/// Errors that carry a specific process exit code (documented in README).
#[derive(Debug)]
pub enum Exit {
    Usage(String),
    Refused(String),
    Timeout(String),
    NeedsInput(String),
}

impl std::fmt::Display for Exit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Exit::Usage(m) | Exit::Refused(m) | Exit::Timeout(m) | Exit::NeedsInput(m) => {
                f.write_str(m)
            }
        }
    }
}

impl std::error::Error for Exit {}

pub fn exit_code(err: &anyhow::Error) -> i32 {
    if let Some(e) = err.downcast_ref::<Exit>() {
        return match e {
            Exit::Usage(_) => 2,
            Exit::Refused(_) => 4,
            Exit::Timeout(_) => 5,
            Exit::NeedsInput(_) => 6,
        };
    }
    if err.downcast_ref::<auth::NotLoggedIn>().is_some()
        || err
            .downcast_ref::<RpcError>()
            .is_some_and(RpcError::is_unauthenticated)
    {
        return 3;
    }
    1
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    /// e.g. "10m"
    pub timeout: Option<String>,
    /// e.g. "1s"
    pub poll_interval: Option<String>,
}

impl Config {
    pub fn load(profile: &str) -> Result<Self> {
        let path = auth::config_dir(profile)?.join("config.toml");
        match std::fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s).with_context(|| format!("parsing {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }
}

pub struct Ctx {
    pub profile: String,
    pub json: bool,
    pub quiet: bool,
    pub verbose: bool,
    pub agent_flag: Option<String>,
    pub session_id: String,
    pub config: Config,
    session: Option<Session>,
    agents: Option<Vec<Agent>>,
}

impl Ctx {
    pub fn new(
        profile: String,
        json: bool,
        quiet: bool,
        verbose: bool,
        agent: Option<String>,
        session_id: Option<String>,
    ) -> Result<Self> {
        let config = Config::load(&profile)?;
        Ok(Self {
            profile,
            json,
            quiet,
            verbose,
            agent_flag: agent,
            session_id: session_id.unwrap_or_default(),
            config,
            session: None,
            agents: None,
        })
    }

    pub fn session(&mut self) -> Result<&mut Session> {
        if self.session.is_none() {
            self.session = Some(Session::open(&self.profile, self.verbose)?);
        }
        Ok(self.session.as_mut().unwrap())
    }

    /// Make an RPC with one automatic re-auth on 401.
    pub fn call<Q: serde::Serialize, R: serde::de::DeserializeOwned>(
        &mut self,
        method: &str,
        req: &Q,
    ) -> Result<R> {
        self.session()?.with_retry(|c| c.call(method, req))
    }

    pub fn client(&mut self) -> Result<&crate::connect::Client> {
        Ok(&self.session()?.client)
    }

    pub fn agents(&mut self) -> Result<&[Agent]> {
        if self.agents.is_none() {
            let resp: ListAgentsResponse = self.call(
                &format!("{SVC}/ListGrokBotAgents"),
                &ListAgentsRequest::default(),
            )?;
            self.agents = Some(resp.agents);
        }
        Ok(self.agents.as_deref().unwrap())
    }

    /// `-a` value, or `$GROKBOT_AGENT` (filled in by clap).
    pub fn agent_query(&self) -> Option<String> {
        self.agent_flag.clone()
    }

    pub fn resolve_agent(&mut self) -> Result<Agent> {
        let q = self.agent_query().ok_or_else(|| {
            Exit::Usage("no bot selected: pass -a/--agent or set GROKBOT_AGENT".into())
        })?;
        self.resolve_agent_named(&q)
    }

    /// Match by UUID / numeric id, exact name (case-insensitive), then unique prefix.
    pub fn resolve_agent_named(&mut self, q: &str) -> Result<Agent> {
        match_agent(self.agents()?, q).cloned()
    }

    pub fn agent_name(&mut self, agent_id: &str) -> String {
        self.agents()
            .ok()
            .and_then(|a| {
                a.iter()
                    .find(|a| a.agent_id == agent_id)
                    .map(|a| a.name.clone())
            })
            .unwrap_or_else(|| agent_id.chars().take(8).collect())
    }

    pub fn timeout(&self, flag: Option<Duration>) -> Result<Duration> {
        if let Some(t) = flag {
            return Ok(t);
        }
        match &self.config.timeout {
            Some(s) => Ok(humantime::parse_duration(s)?),
            None => Ok(Duration::from_secs(600)),
        }
    }

    pub fn poll_interval(&self) -> Duration {
        self.config
            .poll_interval
            .as_deref()
            .and_then(|s| humantime::parse_duration(s).ok())
            .unwrap_or(Duration::from_secs(1))
    }

    /// Progress line on stderr (suppressed by -q or when stderr is not a terminal-ish consumer).
    pub fn note(&self, msg: impl AsRef<str>) {
        if !self.quiet {
            eprintln!("{}", msg.as_ref());
        }
    }
}

/// Match by UUID / numeric id, exact name (case-insensitive), then unique prefix of the
/// name or UUID.
pub fn match_agent<'a>(agents: &'a [Agent], q: &str) -> Result<&'a Agent> {
    let lq = q.to_lowercase();
    if let Some(a) = agents.iter().find(|a| a.agent_id == q || a.id == q) {
        return Ok(a);
    }
    let exact: Vec<_> = agents
        .iter()
        .filter(|a| a.name.to_lowercase() == lq)
        .collect();
    if let [one] = exact.as_slice() {
        return Ok(one);
    }
    let prefix: Vec<_> = agents
        .iter()
        .filter(|a| a.name.to_lowercase().starts_with(&lq) || a.agent_id.starts_with(&lq))
        .collect();
    match (exact.len(), prefix.as_slice()) {
        (0, [one]) => Ok(one),
        (0, []) => Err(Exit::Usage(format!("no bot matches {q:?}; see `grokbot agents`")).into()),
        _ => {
            let cands = if exact.is_empty() { prefix } else { exact };
            let list: Vec<_> = cands
                .iter()
                .map(|a| {
                    format!(
                        "  {}  {}",
                        a.agent_id.chars().take(8).collect::<String>(),
                        a.name
                    )
                })
                .collect();
            Err(Exit::Usage(format!(
                "{q:?} matches several bots; use the id:\n{}",
                list.join("\n")
            ))
            .into())
        }
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// "3m ago" / "in 2h" / "-" for 0.
pub fn ago(ms: i64) -> String {
    if ms <= 0 {
        return "-".into();
    }
    let d = now_ms() - ms;
    let (secs, fmt): (i64, fn(String) -> String) = if d >= 0 {
        (d / 1000, |s| format!("{s} ago"))
    } else {
        (-d / 1000, |s| format!("in {s}"))
    };
    let s = match secs {
        0..=9 => {
            return if d >= 0 {
                "just now".into()
            } else {
                "now".into()
            };
        }
        10..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86399 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86400),
    };
    fmt(s)
}

/// Local wall-clock time `MM-DD HH:MM` for transcript lines.
pub fn clock(ms: i64) -> String {
    if ms <= 0 {
        return "--:--".into();
    }
    let secs: libc::time_t = ms / 1000;
    // SAFETY: localtime_r only writes into the zeroed `tm` we pass.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&secs, &mut tm);
        tm
    };
    format!(
        "{:02}-{:02} {:02}:{:02}",
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min
    )
}

/// Render rows as an aligned table (last column is not padded).
pub fn table(header: &[&str], rows: Vec<Vec<String>>) {
    let mut w: Vec<usize> = header.iter().map(|h| h.chars().count()).collect();
    for r in &rows {
        for (i, c) in r.iter().enumerate() {
            w[i] = w[i].max(display_width(c));
        }
    }
    let line = |cells: Vec<String>| {
        let n = cells.len();
        let mut s = String::new();
        for (i, c) in cells.into_iter().enumerate() {
            if i + 1 == n {
                s.push_str(&c);
            } else {
                let pad = w[i] - display_width(&c);
                s.push_str(&c);
                s.push_str(&" ".repeat(pad + 2));
            }
        }
        println!("{}", s.trim_end());
    };
    line(header.iter().map(|s| s.to_string()).collect());
    for r in rows {
        line(r);
    }
}

/// Rough terminal width: CJK and other wide chars count as 2.
fn display_width(s: &str) -> usize {
    s.chars()
        .map(|c| match c as u32 {
            0x1100..=0x115F
            | 0x2E80..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE30..=0xFE4F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F300..=0x1FAFF
            | 0x20000..=0x3FFFD => 2,
            _ => 1,
        })
        .sum()
}

/// First line, truncated to `max` display columns.
pub fn one_line(s: &str, max: usize) -> String {
    let first = s.lines().next().unwrap_or("");
    let mut out = String::new();
    let mut w = 0;
    for c in first.chars() {
        let cw = display_width(&c.to_string());
        if w + cw > max.saturating_sub(1) {
            out.push('…');
            return out;
        }
        out.push(c);
        w += cw;
    }
    if s.lines().nth(1).is_some() && w < max {
        out.push('…');
    }
    out
}

pub fn print_json(v: &impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string(v)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str, uuid: &str, name: &str) -> Agent {
        Agent {
            id: id.into(),
            agent_id: uuid.into(),
            name: name.into(),
            ..Default::default()
        }
    }

    fn agents() -> Vec<Agent> {
        vec![
            agent("1", "aaaa1111-0000-0000-0000-000000000000", "Research"),
            agent("2", "bbbb2222-0000-0000-0000-000000000000", "Researcher"),
            agent("3", "cccc3333-0000-0000-0000-000000000000", "Ops"),
            agent("4", "dddd4444-0000-0000-0000-000000000000", "ops"),
        ]
    }

    fn usage_err(r: Result<&Agent>) -> String {
        let e = r.unwrap_err();
        assert_eq!(exit_code(&e), 2);
        e.to_string()
    }

    #[test]
    fn agent_matching() {
        let a = agents();
        assert_eq!(
            match_agent(&a, "bbbb2222-0000-0000-0000-000000000000")
                .unwrap()
                .id,
            "2"
        );
        assert_eq!(match_agent(&a, "3").unwrap().name, "Ops");
        // Exact name beats a longer name sharing the prefix.
        assert_eq!(match_agent(&a, "research").unwrap().id, "1");
        assert_eq!(match_agent(&a, "researche").unwrap().id, "2");
        assert_eq!(match_agent(&a, "cccc").unwrap().id, "3");
        assert!(usage_err(match_agent(&a, "nope")).contains("no bot matches"));
        assert!(usage_err(match_agent(&a, "res")).contains("several bots"));
        // Two bots whose names differ only in case are ambiguous.
        assert!(usage_err(match_agent(&a, "OPS")).contains("several bots"));
    }

    #[test]
    fn exit_codes() {
        assert_eq!(exit_code(&Exit::Refused("x".into()).into()), 4);
        assert_eq!(exit_code(&Exit::Timeout("x".into()).into()), 5);
        assert_eq!(exit_code(&Exit::NeedsInput("x".into()).into()), 6);
        assert_eq!(exit_code(&auth::NotLoggedIn("x".into()).into()), 3);
        assert_eq!(exit_code(&anyhow::anyhow!("x")), 1);
    }

    #[test]
    fn text_helpers() {
        assert_eq!(display_width("ab中文"), 6);
        assert_eq!(one_line("hello\nworld", 20), "hello…");
        assert_eq!(one_line("abcdef", 4), "abc…");
        assert_eq!(one_line("中文中文", 5), "中文…");
        assert_eq!(ago(0), "-");
        assert_eq!(ago(now_ms() - 120_000), "2m ago");
        assert!(ago(now_ms() + 3 * 3_600_000 + 60_000).starts_with("in 3h"));
    }
}
