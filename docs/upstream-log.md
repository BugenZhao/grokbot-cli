# Upstream log

Observed behaviour of the Grok Bot backend/app that affected this CLI, newest first.
Append an entry whenever upstream changes or surprises you (see AGENTS.md). Each entry:
date, app version, what was observed, how it was verified, what changed here.

## 2026-10-22 — Grok Bot 0.75.0 (initial survey)

- **Routines live in automations.** `ListGrokBotAgentRoutines` returns `not_found`
  (“Grok Bot routines are not available for this bot.”) for every bot on the account (all
  `harness: "temporal"`). Schedules are in `ListGrokBotAccountAutomations` →
  `automations[].recordJson`; toggling is `SetGrokBotAgentAutomationEnabled`.
  Verified by listing all bots and toggling the test bot's routine.
  → `routines` uses the automation RPCs (docs/commands.md#routines).
- **JSON codec accepted.** The app sends binary protobuf, but the backend accepts
  `application/json` on every RPC used. Verified with all commands.
- **Replies are `send-message` entries**, not `message` with role `assistant` (those are
  bot-to-bot). Turn completion is reported by
  `ListGrokBotAgentStatuses.live.lastTurnSettlement {turnId, outcome}`; `turnId` equals the
  `requestId` on the turn's entries. → docs/transcript.md.
- **Echo entry id can be the attachment.** With attachments, `GetGrokBotSendStatus.echoEntryId`
  points at the `user-attachment` entry (e.g. `t7ua0`), so `ask` matches the user message by
  `clientNonce` instead.
- **Interrupt outcome** is `"cancelled"`.
- **Enum JSON names** use the full prefix (`GROK_BOT_AGENT_KIND_AGENT`), matching what
  `extract_schema.py` reconstructs for top-level enums.
- **Update feed / downloads** need no auth; `downloads.cursor.com` rejects the
  `Python-urllib` user agent (HTTP 403) and supports range requests. → docs/schema.md.
- The app auto-updated from 0.72.1 to 0.75.0 during the survey; 0.75.0 added fields such as
  `SendGrokBotUserMessageRequest.reply_quote` and `language` (not used here).
