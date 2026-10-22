# Transcript entries and turns

`ListGrokBotTranscriptEntries` returns `GrokBotTranscriptEntry` (newest first):

```proto
message GrokBotTranscriptEntry {
  uint64 seq = 1;            // monotonically increasing per agent/session
  string entry_kind = 2;     // same as body.kind
  optional bytes body = 3;   // JSON document, described below (not in the schema)
  optional string blob_hash = 4;
  uint64 updated_seq = 5;
  optional string entry_id = 6;
  bool body_omitted = 7;
}
```

Paging: `limit` (≤200 works) and `before_seq`. An empty `session_id` is the bot's main
session.

## Body shapes (observed on Grok Bot 0.75)

Common fields: `kind`, `id`, `timestampMs`, and usually `requestId` (= the turn id).

| kind | discriminator | meaning | key fields |
|---|---|---|---|
| `message` | `role: "user"` | your message | `content`, `richText`, `clientNonce` (= `message_id` you sent), `replyTo`, `batchId` |
| `message` | `role: "assistant"` | bot → another bot | `content`, `toAgent {id,name,kind}` |
| `send-message` | `message.type: "text"` | **reply shown to the user** | `message.content`, `reactions` |
| `send-message` | `message.type: "widget"` | buttons to click | `message.widget {prompt, options[{label,value,style}]}`, `respondedValue` once answered |
| `send-message` | `message.type: "secret-request"` | asks for a secret | `message.secretRequest {label, description, target}`, `secretProvided` |
| `send-message` | `message.type: "*-draft"` (`email-draft`, `slack-draft`) | draft awaiting approval | `message.draft`, `draftRoute`, `draftSendState` once handled |
| `send-message` | `message.type: "auto-review-approval"` | approval request | |
| `user-attachment` | | attachment you sent | `file_name`, `file_path`, `byteSize`, `clientNonce` |
| `event` | | e.g. routine updated | `event {type, action, automationId, automationName}` |
| `voice-call`, `notice` | | | |

`src/transcript.rs` maps these to `View` variants; unknown kinds become `View::Other`
and remain visible via `grokbot log --json` / `--all`.

## Turn model (what `ask` relies on)

1. `SendGrokBotUserMessage` with a fresh `message_id` → `delivery`
   (`ACCEPTED_TEMPORAL`/`ACCEPTED_BOX`/`DUPLICATE` = ok, `REFUSED` + `refusal`).
2. `GetGrokBotSendStatus` → `ACCEPTED` with `echoEntryId` (the entry for your message;
   with attachments this can be the attachment entry, e.g. `t7ua0`) and `acceptedAtMs`.
3. The echo `message` entry has `clientNonce == message_id` and a `requestId`: the turn id.
   Every entry the bot writes for that turn carries the same `requestId`. Several quick
   messages may be batched into one turn.
4. `ListGrokBotAgentStatuses` → `live`:
   `isRunning`, `activity {kind: "thinking"|…, tool}`, `awaiting {reason}`,
   `hasRunningSubagents`, `workingTaskIds`, and
   `lastTurnSettlement {turnId, outcome: "success"|"cancelled"|…, settledAtMs}`.
   The turn is over when `!isRunning` and `lastTurnSettlement.turnId` is our turn (or it
   settled after `acceptedAtMs`).
