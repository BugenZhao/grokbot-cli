//! `grokbot`: a non-interactive CLI for Grok Bot. See README.md and docs/.

mod api;
mod auth;
mod cmd;
mod connect;
mod ctx;
mod transcript;

use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "grokbot",
    version,
    about = "Talk to your Grok Bot bots from scripts",
    max_term_width = 100
)]
struct Cli {
    /// Bot to act on: UUID, name, or unique name/UUID prefix.
    #[arg(short, long, global = true, env = "GROKBOT_AGENT")]
    agent: Option<String>,
    /// Conversation session id (default: the bot's main session).
    #[arg(long, global = true)]
    session: Option<String>,
    /// Machine-readable output (one JSON object per line).
    #[arg(long, global = true)]
    json: bool,
    /// Suppress progress and hints on stderr.
    #[arg(short, long, global = true)]
    quiet: bool,
    /// Log every RPC request/response to stderr.
    #[arg(short, long, global = true)]
    verbose: bool,
    /// Credentials/config profile (~/.config/grokbot/profiles/NAME).
    #[arg(
        long,
        global = true,
        env = "GROKBOT_PROFILE",
        default_value = "default"
    )]
    profile: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Sign in (browser PKCE flow), or borrow the desktop app's session with --from-app.
    Login {
        /// Read-only import of the Grok Bot app's sign-in (never refreshes the app's tokens).
        #[arg(long)]
        from_app: bool,
        /// Print the URL instead of opening a browser.
        #[arg(long)]
        no_browser: bool,
    },
    /// Forget stored credentials.
    Logout,
    /// Show the signed-in account.
    Whoami,
    /// List your bots.
    Agents {
        /// Include bots shared by your team.
        #[arg(long)]
        team: bool,
    },
    /// List a bot's conversation sessions.
    Sessions,
    /// Send a message and print the bot's reply.
    Ask {
        /// Message text; `-` or omitted reads stdin.
        text: Option<String>,
        /// Attach a file (repeatable).
        #[arg(short, long = "file")]
        files: Vec<PathBuf>,
        /// Return right after the message is accepted; prints the message id.
        #[arg(long)]
        no_wait: bool,
        /// Give up waiting after this long (e.g. 30s, 10m). Default 10m.
        #[arg(long, value_parser = humantime::parse_duration)]
        timeout: Option<Duration>,
    },
    /// Wait for the reply to a message sent with `ask --no-wait`.
    Wait {
        message_id: String,
        #[arg(long, value_parser = humantime::parse_duration)]
        timeout: Option<Duration>,
    },
    /// Print the conversation transcript.
    Log {
        /// Number of entries.
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: u32,
        /// Only entries before this sequence number (paging).
        #[arg(long)]
        before: Option<i64>,
        /// Keep polling and print new entries as they arrive.
        #[arg(short, long)]
        follow: bool,
        /// Also show bot-to-bot messages, events and other internal entries.
        #[arg(long)]
        all: bool,
    },
    /// Show whether bots are running, idle or waiting for you.
    Status {
        /// Bots to show (default: -a, or all bots).
        bots: Vec<String>,
    },
    /// Interrupt the bot's current turn.
    Stop,
    /// List a bot's background work items (or the session checklist with --checklist).
    Tasks {
        /// Include recently finished work.
        #[arg(long)]
        done: bool,
        /// Show the per-session task checklist (ListGrokBotTasks) instead.
        #[arg(long)]
        checklist: bool,
    },
    /// List scheduled routines, or enable/disable one.
    Routines {
        #[command(subcommand)]
        action: Option<RoutineAction>,
    },
    /// Call any RPC with a JSON body (debugging / exploring new schema).
    Rpc {
        /// e.g. GrokBotService/ListGrokBotAgents
        method: String,
        /// Request JSON (default `{}`; `-` reads stdin).
        body: Option<String>,
    },
}

#[derive(Subcommand)]
enum RoutineAction {
    /// Turn a routine on (ID or unique prefix, as printed by `routines`).
    Enable { id: String },
    /// Turn a routine off.
    Disable { id: String },
}

fn main() {
    let cli = Cli::parse();
    let result = run(cli);
    if let Err(e) = result {
        let code = ctx::exit_code(&e);
        eprintln!("grokbot: {e:#}");
        std::process::exit(code);
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    let mut ctx = ctx::Ctx::new(
        cli.profile,
        cli.json,
        cli.quiet,
        cli.verbose,
        cli.agent,
        cli.session,
    )?;
    match cli.command {
        Command::Login {
            from_app,
            no_browser,
        } => cmd::auth::login(&mut ctx, from_app, !no_browser),
        Command::Logout => cmd::auth::logout(&ctx),
        Command::Whoami => cmd::auth::whoami(&mut ctx),
        Command::Agents { team } => cmd::agents::agents(&mut ctx, team),
        Command::Sessions => cmd::agents::sessions(&mut ctx),
        Command::Ask {
            text,
            files,
            no_wait,
            timeout,
        } => cmd::ask::ask(&mut ctx, text, files, no_wait, timeout),
        Command::Wait {
            message_id,
            timeout,
        } => cmd::ask::wait(&mut ctx, &message_id, timeout),
        Command::Log {
            limit,
            before,
            follow,
            all,
        } => cmd::log::log(&mut ctx, limit, before, follow, all),
        Command::Status { bots } => cmd::agents::status(&mut ctx, bots),
        Command::Stop => cmd::agents::stop(&mut ctx),
        Command::Tasks { done, checklist } => cmd::work::tasks(&mut ctx, done, checklist),
        Command::Routines { action } => match action {
            None => cmd::work::routines(&mut ctx),
            Some(RoutineAction::Enable { id }) => cmd::work::set_routine(&mut ctx, &id, true),
            Some(RoutineAction::Disable { id }) => cmd::work::set_routine(&mut ctx, &id, false),
        },
        Command::Rpc { method, body } => cmd::rpc::rpc(&mut ctx, &method, body),
    }
}
