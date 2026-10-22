//! `login`, `logout`, `whoami`.

use std::time::Duration;

use anyhow::Result;

use crate::api::GetMeResponse;
use crate::auth::{self, Stored, login as pkce};
use crate::connect::DEFAULT_BACKEND;
use crate::ctx::{Ctx, print_json};

pub fn login(ctx: &mut Ctx, from_app: bool, open_browser: bool) -> Result<()> {
    let stored = if from_app {
        let creds = auth::app::read()?;
        Stored::App {
            cached_access_token: Some(creds.access_token),
            machine_id: Some(creds.machine_id),
        }
    } else {
        let (access_token, refresh_token) =
            pkce::login(DEFAULT_BACKEND, open_browser, Duration::from_secs(300))?;
        let machine_id = match auth::load(&ctx.profile)? {
            Some(Stored::Login { machine_id, .. }) => machine_id,
            _ => pkce::new_machine_id(),
        };
        Stored::Login {
            access_token,
            refresh_token,
            machine_id,
        }
    };
    auth::save(&ctx.profile, &stored)?;
    let me: GetMeResponse = ctx.call("DashboardService/GetMe", &serde_json::json!({}))?;
    ctx.note(format!(
        "Signed in as {}{}",
        me.email.as_deref().unwrap_or("?"),
        if from_app {
            " (borrowing the Grok Bot app's session)"
        } else {
            ""
        }
    ));
    Ok(())
}

pub fn logout(ctx: &Ctx) -> Result<()> {
    // Our own session could also be revoked server-side via /auth/logout; we only drop it
    // locally so an imported app session is never affected.
    if auth::remove(&ctx.profile)? {
        ctx.note("Removed stored credentials.");
    } else {
        ctx.note("Not logged in.");
    }
    Ok(())
}

pub fn whoami(ctx: &mut Ctx) -> Result<()> {
    let me: GetMeResponse = ctx.call("DashboardService/GetMe", &serde_json::json!({}))?;
    let mode = match auth::load(&ctx.profile)? {
        Some(Stored::Login { .. }) => "login",
        Some(Stored::App { .. }) => "app",
        None => "env",
    };
    if ctx.json {
        return print_json(&serde_json::json!({ "mode": mode, "me": me }));
    }
    println!("{}", me.email.as_deref().unwrap_or("?"));
    if let Some(n) = &me.first_name {
        println!("name:    {n}");
    }
    if let Some(t) = &me.team_name {
        println!("team:    {t}");
    }
    println!("session: {mode}");
    Ok(())
}
