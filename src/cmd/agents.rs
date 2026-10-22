//! `agents`, `sessions`, `status`, `stop`.

use anyhow::Result;

use crate::api::{
    self, AgentStatus, InterruptRequest, InterruptResponse, ListAgentsRequest, ListAgentsResponse,
    ListSessionsResponse, ListStatusesResponse, SVC,
};
use crate::ctx::{Ctx, ago, one_line, print_json, table};

pub fn fetch_statuses(ctx: &mut Ctx, ids: Vec<String>) -> Result<Vec<AgentStatus>> {
    let resp: ListStatusesResponse = ctx.call(
        &format!("{SVC}/ListGrokBotAgentStatuses"),
        &serde_json::json!({ "agentIds": ids }),
    )?;
    Ok(resp.statuses)
}

/// "running (tool: web_search)" / "awaiting you" / "idle".
pub fn state_label(s: &AgentStatus) -> String {
    let Some(live) = &s.live else {
        return if s.is_running {
            "running".into()
        } else {
            "idle".into()
        };
    };
    if live.awaiting.is_some() {
        return "awaiting you".into();
    }
    if live.is_running || s.is_running {
        let act = live.activity.as_ref().map(|a| {
            if a.tool.is_empty() {
                a.kind.clone()
            } else {
                a.tool.clone()
            }
        });
        return match act {
            Some(a) if !a.is_empty() => format!("running ({a})"),
            _ => "running".into(),
        };
    }
    if live.has_running_subagents || !live.working_task_ids.is_empty() {
        return "idle, background work".into();
    }
    "idle".into()
}

pub fn agents(ctx: &mut Ctx, team: bool) -> Result<()> {
    let req = ListAgentsRequest {
        include_team_agents: team.then_some(true),
    };
    let resp: ListAgentsResponse = ctx.call(&format!("{SVC}/ListGrokBotAgents"), &req)?;
    let statuses = fetch_statuses(
        ctx,
        resp.agents.iter().map(|a| a.agent_id.clone()).collect(),
    )?;
    let status_of = |id: &str| statuses.iter().find(|s| s.agent_id == id);
    if ctx.json {
        for a in &resp.agents {
            print_json(&serde_json::json!({ "agent": a, "status": status_of(&a.agent_id) }))?;
        }
        return Ok(());
    }
    let rows = resp
        .agents
        .iter()
        .map(|a| {
            let s = status_of(&a.agent_id);
            let last = s
                .and_then(|s| s.client.as_ref())
                .and_then(|c| c.last_activity_at_ms)
                .unwrap_or(a.updated_at_ms);
            vec![
                a.agent_id[..8].to_owned(),
                a.name.clone(),
                s.map(state_label).unwrap_or_else(|| "?".into()),
                ago(last),
                one_line(&a.description, 50),
            ]
        })
        .collect();
    table(&["ID", "NAME", "STATE", "ACTIVE", "DESCRIPTION"], rows);
    Ok(())
}

pub fn sessions(ctx: &mut Ctx) -> Result<()> {
    let agent = ctx.resolve_agent()?;
    let resp: ListSessionsResponse = ctx.call(
        &format!("{SVC}/ListGrokBotAgentSessions"),
        &serde_json::json!({ "agentId": agent.agent_id }),
    )?;
    if ctx.json {
        return resp.sessions.iter().try_for_each(print_json);
    }
    if resp.sessions.is_empty() {
        ctx.note(format!(
            "{} only has its main session (use no --session).",
            agent.name
        ));
        return Ok(());
    }
    let rows = resp
        .sessions
        .iter()
        .map(|s| {
            vec![
                s.session_id.clone(),
                api::enum_suffix(&s.kind, "GROK_BOT_AGENT_SESSION_KIND"),
                ago(s.last_activity_at_ms.unwrap_or(s.created_at_ms)),
            ]
        })
        .collect();
    table(&["SESSION", "KIND", "ACTIVE"], rows);
    Ok(())
}

pub fn status(ctx: &mut Ctx, bots: Vec<String>) -> Result<()> {
    let mut agents = Vec::new();
    for q in &bots {
        agents.push(ctx.resolve_agent_named(q)?);
    }
    if agents.is_empty() {
        match ctx.agent_query() {
            Some(_) => agents.push(ctx.resolve_agent()?),
            None => agents = ctx.agents()?.to_vec(),
        }
    }
    let statuses = fetch_statuses(ctx, agents.iter().map(|a| a.agent_id.clone()).collect())?;
    if ctx.json {
        return statuses.iter().try_for_each(print_json);
    }
    let rows = statuses
        .iter()
        .map(|s| {
            let c = s.client.as_ref();
            vec![
                ctx.agent_name(&s.agent_id),
                state_label(s),
                c.map_or(0, |c| c.unread_count).to_string(),
                ago(c.and_then(|c| c.last_activity_at_ms).unwrap_or(0)),
                one_line(
                    c.and_then(|c| c.last_message_preview.as_deref())
                        .unwrap_or(""),
                    60,
                ),
            ]
        })
        .collect();
    table(&["NAME", "STATE", "UNREAD", "ACTIVE", "LAST MESSAGE"], rows);
    Ok(())
}

pub fn stop(ctx: &mut Ctx) -> Result<()> {
    let agent = ctx.resolve_agent()?;
    let session = ctx.session_id.clone();
    let resp: InterruptResponse = ctx.call(
        &format!("{SVC}/InterruptGrokBotAgentRun"),
        &InterruptRequest {
            agent_id: &agent.agent_id,
            reason: "user_interrupt",
            session_id: &session,
        },
    )?;
    if ctx.json {
        return print_json(&serde_json::json!({ "hadActiveRun": resp.had_active_run }));
    }
    ctx.note(if resp.had_active_run {
        format!("Interrupted {}.", agent.name)
    } else {
        format!("{} was not running.", agent.name)
    });
    Ok(())
}
