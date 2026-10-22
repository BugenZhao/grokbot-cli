//! `log`: print the transcript, optionally following it.

use anyhow::Result;

use crate::ctx::{Ctx, clock, print_json};
use crate::transcript::{self, Entry, View};

pub fn log(ctx: &mut Ctx, limit: u32, before: Option<i64>, follow: bool, all: bool) -> Result<()> {
    let agent = ctx.resolve_agent()?;
    let session = ctx.session_id.clone();
    let entries = transcript::last(ctx.client()?, &agent.agent_id, &session, limit, before)?;
    let mut cursor = entries.last().map_or(0, |e| e.seq);
    for e in &entries {
        print_entry(ctx, &agent.name, e, all)?;
    }
    if !follow {
        return Ok(());
    }
    if cursor == 0 {
        cursor = transcript::latest_seq(ctx.client()?, &agent.agent_id, &session)?;
    }
    let poll = ctx.poll_interval().max(std::time::Duration::from_secs(2));
    loop {
        std::thread::sleep(poll);
        for e in transcript::since(ctx.client()?, &agent.agent_id, &session, cursor)? {
            cursor = cursor.max(e.seq);
            print_entry(ctx, &agent.name, &e, all)?;
        }
    }
}

fn print_entry(ctx: &Ctx, bot: &str, e: &Entry, all: bool) -> Result<()> {
    if ctx.json {
        return print_json(&serde_json::json!({
            "seq": e.seq, "id": e.id, "kind": e.kind, "timestampMs": e.timestamp_ms,
            "requestId": e.request_id, "body": e.body,
        }));
    }
    let t = clock(e.timestamp_ms);
    let line = |who: &str, text: &str| {
        let mut lines = text.lines();
        println!("[{t}] {who}: {}", lines.next().unwrap_or(""));
        for l in lines {
            println!("    {l}");
        }
    };
    match e.view() {
        View::User { text, .. } => line("you", text),
        View::Reply { text } => line(bot, text),
        View::NeedsInput { what, answered } => line(
            bot,
            &format!(
                "[{}] {what}",
                if answered { "answered" } else { "needs input" }
            ),
        ),
        View::Attachment { name } => line("you", &format!("[attachment] {name}")),
        View::AgentMessage { to, text } if all => {
            line(&format!("{bot} → {}", to.unwrap_or("?")), text)
        }
        View::Event { summary } if all => println!("[{t}] · {summary}"),
        View::Other if all => println!("[{t}] · {} (seq {})", e.kind, e.seq),
        _ => {}
    }
    Ok(())
}
