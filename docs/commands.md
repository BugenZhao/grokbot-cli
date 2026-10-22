# Commands → RPC flows

All RPCs are `aiserver.v1.GrokBotService/*` unless noted. Every command that takes a bot
first resolves `-a` through **`ListGrokBotAgents`** (UUID / numeric id → exact name →
unique prefix of name or UUID) and then passes the agent's UUID (`agentId`, not `id`).

## login / logout / whoami
- `login`: PKCE flow (docs/auth.md), then `DashboardService/GetMe` to confirm.
- `login --from-app`: decrypt the app's token (docs/auth.md), then `GetMe`.
- `logout`: deletes `auth.json` only (does not revoke; never touches the app).
- `whoami`: `DashboardService/GetMe {}` → `email`, `firstName`, `teamName`.

## agents
1. `ListGrokBotAgents {includeTeamAgents?}` → `agents[]`.
2. `ListGrokBotAgentStatuses {agentIds: [...all]}` for the STATE/ACTIVE columns.

## sessions
`ListGrokBotAgentSessions {agentId}` → `sessions[] {sessionId, kind, …}`. An empty list
means only the main session exists (use no `--session`).

## ask
1. Read text (arg or stdin).
2. Baseline: `ListGrokBotTranscriptEntries {agentId, limit: 1}` → newest `seq`.
3. For each `-f FILE`:
   `BeginGrokBotAttachmentUpload {agentId, fileName, sizeBytes, sha256Hex, supportsStoreAttachmentRefs: true, supportsMultipart: false}`
   → `mode` must be `SINGLE_PUT` → `PUT put.url` with `put.headers` →
   `CompleteGrokBotAttachmentUpload {uploadToken}` → `alias` (a path inside the bot's box).
   `USE_BOX` and `MULTIPART` are not supported.
4. `SendGrokBotUserMessage {agentId, messageId: uuid4, text, sentAtMs, attachmentPaths: [alias], attachmentNames, source: DESKTOP, initiator: USER, sendSource: CHAT}`.
   `delivery: REFUSED` → exit 4. With `--no-wait`, print `messageId` and stop.
5. Follow the turn (shared with `wait`), polling every `poll_interval` (1 s):
   - `GetGrokBotSendStatus {agentId, messageId}` until `ACCEPTED` (`REJECTED` → exit 4).
   - `ListGrokBotTranscriptEntries` pages back to the cursor; the entry with
     `clientNonce == messageId` gives the turn id (`requestId`). New `send-message`/`text`
     entries of that turn are printed as they appear; unanswered widgets/secret
     requests/drafts are remembered.
   - `ListGrokBotAgentStatuses {agentIds: [agentId]}` until `!live.isRunning` and
     `live.lastTurnSettlement` is our turn; then one last transcript pass.
6. Finish: pending input → exit 6; `outcome != "success"` (e.g. `cancelled`) → exit 1;
   `hasRunningSubagents`/`workingTaskIds` → note on stderr; deadline → exit 5 with a
   `grokbot wait` hint.

`--json` emits `{"type":"sent"}`, `{"type":"reply"}`, `{"type":"needs_input"}`, `{"type":"done"}` lines.

## wait MESSAGE_ID
Same as step 5 of `ask`, but the start point is found by scanning the last 200 entries
for `clientNonce == MESSAGE_ID`.

## log
`ListGrokBotTranscriptEntries {agentId, limit, beforeSeq?}` (pages of ≤200, newest first,
printed oldest first). `--follow` polls every ≥2 s for `seq > last`. `--all` adds
bot-to-bot messages, events and unknown kinds; `--json` prints decoded bodies.

## status
`ListGrokBotAgentStatuses {agentIds}`. STATE: `awaiting you` if `live.awaiting`,
`running (activity)` if `live.isRunning`, `idle, background work` if subagents/working
tasks, else `idle`.

## stop
`InterruptGrokBotAgentRun {agentId, reason: "user_interrupt", sessionId?}` →
`hadActiveRun`. A waiting `ask` then ends with outcome `cancelled`.

## tasks
- default: `ListGrokBotAgentTasks {agentId, includeRecentlyDone}` → background work items
  (`name`, `status`, `subagentIds`).
- `--checklist`: `ListGrokBotTasks {agentId, sessionId}` → the session to-do checklist
  (`WORKING`/`NEEDS_INPUT`/`DONE`).

## routines
What the app calls routines are **automations** for the current (temporal-harness) bots:
`ListGrokBotAgentRoutines` answers `not_found` (“routines are not available for this bot”).

- list: `ListGrokBotAccountAutomations {agentIds: [...], timeZone}` → per bot
  `automations[] {automationId, recordJson}`; `recordJson` holds `name`, `schedule`
  (`CRON_TZ=… m h dom mon dow`), `triggerDescription`, `isEnabled`, `nextRunAt`,
  `lastRunAt`, `runs[]`, `prompt`.
- `enable|disable ID`: find the owning bot from the list (ID prefix accepted), then
  `SetGrokBotAgentAutomationEnabled {agentId, automationId, isEnabled, timeZone}` → updated
  `automations[]`.

Creating/editing routines has no dedicated RPC here; ask the bot in chat.

## rpc METHOD [JSON]
`POST` any method with a raw JSON body (`-` = stdin) and print the response; `-v` also
logs the request. Use it to explore before adding a command.
