//! `ask` and `wait`: send a message, then follow its turn until the bot settles.
//! The step-by-step RPC flow is documented in docs/commands.md#ask.

use std::collections::HashSet;
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::api::{
    BeginUploadRequest, BeginUploadResponse, CompleteUploadResponse, SVC, SendRequest,
    SendResponse, SendStatusRequest, SendStatusResponse, enum_suffix,
};
use crate::cmd::agents::fetch_statuses;
use crate::ctx::{Ctx, Exit, now_ms, print_json};
use crate::transcript::{self, View};

pub fn ask(
    ctx: &mut Ctx,
    text: Option<String>,
    files: Vec<PathBuf>,
    no_wait: bool,
    timeout: Option<Duration>,
) -> Result<()> {
    let text = match text.as_deref() {
        Some("-") | None => {
            let mut stdin = std::io::stdin();
            if stdin.is_terminal() {
                return Err(Exit::Usage("no message: pass TEXT or pipe it on stdin".into()).into());
            }
            let mut s = String::new();
            stdin.read_to_string(&mut s)?;
            s
        }
        Some(t) => t.to_owned(),
    };
    if text.trim().is_empty() && files.is_empty() {
        return Err(Exit::Usage("empty message".into()).into());
    }
    let agent = ctx.resolve_agent()?;
    let session = ctx.session_id.clone();
    let deadline = Instant::now() + ctx.timeout(timeout)?;

    // Remember where the transcript ends so we only look at what this message causes.
    let baseline = transcript::latest_seq(ctx.client()?, &agent.agent_id, &session)?;

    let mut paths = Vec::new();
    let mut names = Vec::new();
    for f in &files {
        let (alias, name) = upload(ctx, &agent.agent_id, &session, f)?;
        paths.push(alias);
        names.push(name);
    }

    let message_id = uuid::Uuid::new_v4().to_string();
    let resp: SendResponse = ctx.call(
        &format!("{SVC}/SendGrokBotUserMessage"),
        &SendRequest {
            agent_id: &agent.agent_id,
            message_id: &message_id,
            text: &text,
            sent_at_ms: now_ms().to_string(),
            session_id: &session,
            attachment_paths: paths,
            attachment_names: names,
            source: "GROK_BOT_CLIENT_SURFACE_DESKTOP",
            initiator: "GROK_BOT_USER_MESSAGE_INITIATOR_USER",
            send_source: "GROK_BOT_SEND_SOURCE_CHAT",
        },
    )?;
    match enum_suffix(&resp.delivery, "GROK_BOT_USER_MESSAGE_DELIVERY").as_str() {
        "refused" => {
            let r = resp.refusal.unwrap_or_default();
            return Err(Exit::Refused(
                format!("message refused: {} {}", r.failure_code, r.message)
                    .trim()
                    .to_owned(),
            )
            .into());
        }
        "accepted_box" | "accepted_temporal" | "duplicate" => {}
        other => ctx.note(format!("warning: unexpected delivery state {other:?}")),
    }

    if ctx.json {
        print_json(
            &serde_json::json!({ "type": "sent", "agentId": agent.agent_id, "messageId": message_id }),
        )?;
    }
    if no_wait {
        if !ctx.json {
            println!("{message_id}");
        }
        return Ok(());
    }
    ctx.note(format!("sent · waiting for {}…", agent.name));
    follow_turn(
        ctx,
        &agent.agent_id,
        &agent.name,
        &message_id,
        Some(baseline),
        deadline,
    )
}

pub fn wait(ctx: &mut Ctx, message_id: &str, timeout: Option<Duration>) -> Result<()> {
    let agent = ctx.resolve_agent()?;
    let deadline = Instant::now() + ctx.timeout(timeout)?;
    follow_turn(
        ctx,
        &agent.agent_id,
        &agent.name,
        message_id,
        None,
        deadline,
    )
}

/// Poll until the turn that handles `message_id` settles, printing its replies as they appear.
///
/// `baseline` is the last seq before the message was sent; when unknown (`wait`), the echo
/// entry is located by scanning back for `clientNonce == message_id`.
fn follow_turn(
    ctx: &mut Ctx,
    agent_id: &str,
    agent_name: &str,
    message_id: &str,
    baseline: Option<i64>,
    deadline: Instant,
) -> Result<()> {
    let session = ctx.session_id.clone();
    let poll = ctx.poll_interval();
    let mut cursor = match baseline {
        Some(b) => b,
        None => find_echo_seq(ctx, agent_id, &session, message_id)?.map_or(0, |s| s - 1),
    };
    let mut accepted_at: Option<i64> = None;
    let mut echo_id: Option<String> = None;
    let mut turn_id: Option<String> = None;
    let mut replies = 0usize;
    let mut pending_input: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    let mut settled_once = false;

    loop {
        // 1. Until the server has durably accepted the message we cannot know its turn.
        if turn_id.is_none() && echo_id.is_none() {
            let st: SendStatusResponse = ctx.call(
                &format!("{SVC}/GetGrokBotSendStatus"),
                &SendStatusRequest {
                    agent_id,
                    message_id,
                    session_id: &session,
                },
            )?;
            match enum_suffix(&st.status, "GROK_BOT_SEND_STATUS").as_str() {
                "accepted" => {
                    echo_id = st.echo_entry_id;
                    accepted_at = st.accepted_at_ms;
                }
                "rejected" => {
                    return Err(Exit::Refused(format!(
                        "message rejected: {}",
                        st.rejection_code.unwrap_or_else(|| "unknown".into())
                    ))
                    .into());
                }
                _ => {} // pending / not_found yet / unknown_durability: keep polling
            }
        }

        // 2. New transcript entries: find our echo (gives the turn id), print the turn's replies.
        for e in transcript::since(ctx.client()?, agent_id, &session, cursor)? {
            cursor = cursor.max(e.seq);
            if !seen.insert(e.seq) {
                continue;
            }
            if let View::User { nonce, .. } = e.view()
                && (nonce == Some(message_id) || echo_id.as_deref() == Some(e.id.as_str()))
            {
                turn_id = e.request_id.clone();
                accepted_at = accepted_at.or(Some(e.timestamp_ms));
                continue;
            }
            if turn_id.is_none() || e.request_id != turn_id {
                continue;
            }
            match e.view() {
                View::Reply { text } => {
                    replies += 1;
                    if ctx.json {
                        print_json(
                            &serde_json::json!({ "type": "reply", "seq": e.seq, "id": e.id, "text": text }),
                        )?;
                    } else {
                        if replies > 1 {
                            println!();
                        }
                        println!("{text}");
                    }
                }
                View::NeedsInput {
                    what,
                    answered: false,
                } => {
                    if ctx.json {
                        print_json(
                            &serde_json::json!({ "type": "needs_input", "seq": e.seq, "id": e.id, "what": what, "body": e.body }),
                        )?;
                    }
                    pending_input.push(what);
                }
                _ => {}
            }
        }

        // 3. Has the turn settled?
        if let Some(turn) = &turn_id {
            let status = fetch_statuses(ctx, vec![agent_id.to_owned()])?
                .into_iter()
                .next()
                .unwrap_or_default();
            let live = status.live.clone().unwrap_or_default();
            let running = live.is_running || status.is_running;
            let settlement = live.last_turn_settlement.clone();
            let ours = settlement.as_ref().is_some_and(|s| {
                &s.turn_id == turn || accepted_at.is_some_and(|a| s.settled_at_ms >= a)
            });
            if let Some(awaiting) = &live.awaiting
                && !running
            {
                let what = format!("bot is awaiting you ({})", awaiting.reason);
                if !pending_input.contains(&what) {
                    pending_input.push(what);
                }
            }
            if (ours && !running) || !pending_input.is_empty() {
                // Settled: take one more pass over entries so nothing written at the end is missed.
                if !settled_once {
                    settled_once = true;
                    continue;
                }
                let outcome = settlement.map(|s| s.outcome).unwrap_or_default();
                if ctx.json {
                    print_json(&serde_json::json!({
                        "type": "done",
                        "outcome": outcome,
                        "replies": replies,
                        "backgroundWork": live.has_running_subagents || !live.working_task_ids.is_empty(),
                    }))?;
                }
                if live.has_running_subagents || !live.working_task_ids.is_empty() {
                    ctx.note(format!(
                        "note: {agent_name} is still working in the background; see `grokbot tasks` / `grokbot log`"
                    ));
                }
                if !pending_input.is_empty() {
                    return Err(Exit::NeedsInput(format!(
                        "{agent_name} needs your input in the Grok Bot app: {}",
                        pending_input.join("; ")
                    ))
                    .into());
                }
                if replies == 0 {
                    ctx.note(format!("({agent_name} finished without a text reply)"));
                }
                if !outcome.is_empty() && outcome != "success" {
                    bail!("turn ended with outcome {outcome:?}");
                }
                return Ok(());
            }
        }

        if Instant::now() >= deadline {
            return Err(Exit::Timeout(format!(
                "timed out; {agent_name} may still be working. Resume with: grokbot wait -a {agent_id} {message_id}"
            ))
            .into());
        }
        std::thread::sleep(poll);
    }
}

fn find_echo_seq(
    ctx: &mut Ctx,
    agent_id: &str,
    session: &str,
    message_id: &str,
) -> Result<Option<i64>> {
    let entries = transcript::last(ctx.client()?, agent_id, session, 200, None)?;
    Ok(entries
        .iter()
        .rev()
        .find(|e| matches!(e.view(), View::User { nonce: Some(n), .. } if n == message_id))
        .map(|e| e.seq))
}

/// Upload one file and return `(alias, file_name)` for `attachment_paths`/`attachment_names`.
fn upload(ctx: &mut Ctx, agent_id: &str, session: &str, path: &Path) -> Result<(String, String)> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .context("attachment needs a file name")?
        .to_owned();
    ctx.note(format!("uploading {name} ({} bytes)…", bytes.len()));
    let begin: BeginUploadResponse = ctx.call(
        &format!("{SVC}/BeginGrokBotAttachmentUpload"),
        &BeginUploadRequest {
            agent_id,
            session_id: session,
            file_name: &name,
            size_bytes: bytes.len().to_string(),
            supports_store_attachment_refs: true,
            supports_multipart: false,
            sha256_hex: hex::encode(Sha256::digest(&bytes)),
        },
    )?;
    match enum_suffix(&begin.mode, "GROK_BOT_ATTACHMENT_UPLOAD_MODE").as_str() {
        "single_put" => {}
        "refused" => {
            return Err(Exit::Refused(format!(
                "upload of {name} refused: {}",
                enum_suffix(&begin.refusal, "GROK_BOT_ATTACHMENT_UPLOAD_REFUSAL")
            ))
            .into());
        }
        "use_box" => bail!(
            "upload of {name} must go through the bot's computer ({}); not supported",
            begin.use_box_reason
        ),
        other => bail!("unsupported upload mode {other:?} for {name}"),
    }
    let target = begin.put.context("upload: server returned no PUT target")?;
    let headers: Vec<(String, String)> = target.headers.into_iter().collect();
    ctx.client()?.put_presigned(&target.url, &headers, bytes)?;
    let done: CompleteUploadResponse = ctx.call(
        &format!("{SVC}/CompleteGrokBotAttachmentUpload"),
        &serde_json::json!({ "uploadToken": begin.upload_token }),
    )?;
    if !done.refusal.is_empty() && !done.refusal.ends_with("UNSPECIFIED") {
        return Err(Exit::Refused(format!("upload of {name} refused: {}", done.refusal)).into());
    }
    anyhow::ensure!(
        !done.alias.is_empty(),
        "upload of {name}: no alias returned"
    );
    let file_name = if done.file_name.is_empty() {
        name
    } else {
        done.file_name
    };
    Ok((done.alias, file_name))
}
