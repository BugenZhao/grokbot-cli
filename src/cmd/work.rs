//! `tasks` and `routines`.

use anyhow::Result;

use crate::api::{
    Agent, Automation, ListAccountAutomationsResponse, ListAgentTasksResponse,
    ListSessionTasksResponse, SVC, SetAutomationEnabledResponse, enum_suffix,
};
use crate::ctx::{Ctx, Exit, ago, one_line, print_json, table};

pub fn tasks(ctx: &mut Ctx, done: bool, checklist: bool) -> Result<()> {
    let agent = ctx.resolve_agent()?;
    if checklist {
        let session = ctx.session_id.clone();
        let resp: ListSessionTasksResponse = ctx.call(
            &format!("{SVC}/ListGrokBotTasks"),
            &serde_json::json!({ "agentId": agent.agent_id, "sessionId": session }),
        )?;
        if ctx.json {
            return resp.tasks.iter().try_for_each(print_json);
        }
        if resp.tasks.is_empty() {
            ctx.note(format!(
                "{} has no checklist items in this session.",
                agent.name
            ));
            return Ok(());
        }
        let rows = resp
            .tasks
            .iter()
            .map(|t| {
                vec![
                    enum_suffix(&t.state, "GROK_BOT_TASK_STATE"),
                    one_line(&t.title, 50),
                    ago(if t.done_at_ms > 0 {
                        t.done_at_ms
                    } else {
                        t.created_at_ms
                    }),
                    one_line(&t.note, 50),
                ]
            })
            .collect();
        table(&["STATE", "TITLE", "WHEN", "NOTE"], rows);
        return Ok(());
    }
    let resp: ListAgentTasksResponse = ctx.call(
        &format!("{SVC}/ListGrokBotAgentTasks"),
        &serde_json::json!({ "agentId": agent.agent_id, "includeRecentlyDone": done }),
    )?;
    if ctx.json {
        return resp.tasks.iter().try_for_each(print_json);
    }
    if resp.tasks.is_empty() {
        ctx.note(format!(
            "{} has no {}tasks.",
            agent.name,
            if done { "" } else { "open " }
        ));
        return Ok(());
    }
    let rows = resp
        .tasks
        .iter()
        .map(|t| {
            let name = if t.subagent_ids.is_empty() {
                t.name.clone()
            } else {
                format!("{} ({} subagents)", t.name, t.subagent_ids.len())
            };
            vec![
                t.id.clone(),
                t.status.clone(),
                one_line(&name, 50),
                ago(t.updated_at_ms),
            ]
        })
        .collect();
    table(&["ID", "STATUS", "NAME", "UPDATED"], rows);
    Ok(())
}

fn tz() -> Option<String> {
    std::env::var("TZ")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            // /etc/localtime -> /var/db/timezone/zoneinfo/Asia/Singapore
            std::fs::read_link("/etc/localtime").ok().and_then(|p| {
                p.to_str()
                    .and_then(|s| s.split("zoneinfo/").nth(1))
                    .map(str::to_owned)
            })
        })
}

/// One `ListGrokBotAccountAutomations` call covers every bot.
fn list_automations(ctx: &mut Ctx, agents: &[Agent]) -> Result<Vec<(Agent, Automation)>> {
    let ids: Vec<&str> = agents.iter().map(|a| a.agent_id.as_str()).collect();
    let resp: ListAccountAutomationsResponse = ctx.call(
        &format!("{SVC}/ListGrokBotAccountAutomations"),
        &serde_json::json!({ "agentIds": ids, "timeZone": tz() }),
    )?;
    let mut out = Vec::new();
    for group in resp.agents {
        if let Some(a) = agents.iter().find(|a| a.agent_id == group.agent_id) {
            out.extend(group.automations.into_iter().map(|au| (a.clone(), au)));
        }
    }
    Ok(out)
}

pub fn routines(ctx: &mut Ctx) -> Result<()> {
    let agents = match ctx.agent_query() {
        Some(_) => vec![ctx.resolve_agent()?],
        None => ctx.agents()?.to_vec(),
    };
    let list = list_automations(ctx, &agents)?;
    if ctx.json {
        for (a, au) in &list {
            let record: serde_json::Value =
                serde_json::from_str(&au.record_json).unwrap_or_default();
            print_json(&serde_json::json!({
                "agentId": a.agent_id, "agentName": a.name, "automationId": au.automation_id, "record": record,
            }))?;
        }
        return Ok(());
    }
    if list.is_empty() {
        ctx.note("No routines.");
        return Ok(());
    }
    let rows = list
        .iter()
        .map(|(a, au)| {
            let r = au.record();
            let when = if r.trigger_description.is_empty() {
                r.schedule.clone()
            } else {
                r.trigger_description.clone()
            };
            vec![
                au.automation_id[..8.min(au.automation_id.len())].to_owned(),
                a.name.clone(),
                if r.is_enabled {
                    "on".into()
                } else {
                    "off".into()
                },
                one_line(&when, 32),
                if r.is_enabled {
                    r.next_run_at.map(ago).unwrap_or_else(|| "-".into())
                } else {
                    "-".into()
                },
                one_line(&r.name, 40),
            ]
        })
        .collect();
    table(&["ID", "BOT", "STATE", "WHEN", "NEXT", "NAME"], rows);
    Ok(())
}

pub fn set_routine(ctx: &mut Ctx, id: &str, enabled: bool) -> Result<()> {
    let agents = match ctx.agent_query() {
        Some(_) => vec![ctx.resolve_agent()?],
        None => ctx.agents()?.to_vec(),
    };
    // Accept a unique prefix of the automation id, as printed by `routines`.
    let matches: Vec<_> = list_automations(ctx, &agents)?
        .into_iter()
        .filter(|(_, au)| au.automation_id.starts_with(id))
        .collect();
    let (agent, au) = match matches.as_slice() {
        [one] => one.clone(),
        [] => {
            return Err(
                Exit::Usage(format!("no routine with id {id:?}; see `grokbot routines`")).into(),
            );
        }
        _ => {
            return Err(Exit::Usage(format!(
                "routine id {id:?} is ambiguous; give more characters"
            ))
            .into());
        }
    };
    let resp: SetAutomationEnabledResponse = ctx.call(
        &format!("{SVC}/SetGrokBotAgentAutomationEnabled"),
        &serde_json::json!({
            "agentId": agent.agent_id, "automationId": au.automation_id, "isEnabled": enabled, "timeZone": tz(),
        }),
    )?;
    let updated = resp
        .automations
        .iter()
        .find(|x| x.automation_id == au.automation_id)
        .map(Automation::record);
    if ctx.json {
        return print_json(
            &serde_json::json!({ "automationId": au.automation_id, "record": updated }),
        );
    }
    let name = updated
        .as_ref()
        .map_or_else(|| au.record().name, |r| r.name.clone());
    if let Some(r) = &updated
        && r.is_enabled != enabled
    {
        anyhow::bail!(
            "server did not apply the change (routine is still {})",
            if r.is_enabled { "on" } else { "off" }
        );
    }
    ctx.note(format!(
        "{} routine {name:?} on {}.",
        if enabled { "Enabled" } else { "Disabled" },
        agent.name
    ));
    Ok(())
}
