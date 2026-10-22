//! `rpc`: raw access to any method, for exploring schema changes (docs/schema.md).

use std::io::Read;

use anyhow::{Context, Result};

use crate::ctx::Ctx;

pub fn rpc(ctx: &mut Ctx, method: &str, body: Option<String>) -> Result<()> {
    let body = match body.as_deref() {
        Some("-") => {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s)?;
            s
        }
        Some(s) => s.to_owned(),
        None => "{}".to_owned(),
    };
    let body: serde_json::Value =
        serde_json::from_str(&body).context("request body is not JSON")?;
    let resp = ctx.session()?.with_retry(|c| c.call_value(method, &body))?;
    println!(
        "{}",
        if ctx.json {
            serde_json::to_string(&resp)?
        } else {
            serde_json::to_string_pretty(&resp)?
        }
    );
    Ok(())
}
