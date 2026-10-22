//! Typed subset of `aiserver.v1` used by the CLI, in protobuf-JSON form.
//!
//! Field names/numbers come from schema/aiserver.proto (regenerate with
//! `tools/extract_schema.py`). Only fields we read or write are declared; unknown fields are
//! ignored, so additive server changes never break decoding. Conventions of the JSON mapping:
//! camelCase names, int64 as strings, enums as full `SCREAMING_SNAKE` names, default values
//! omitted (hence `#[serde(default)]` everywhere).

use serde::{Deserialize, Deserializer, Serialize};

pub const SVC: &str = "GrokBotService";

/// int64/uint64 arrive as JSON strings; accept numbers too.
pub fn i64_str<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum N {
        S(String),
        I(i64),
    }
    match N::deserialize(d)? {
        N::S(s) => s.parse().map_err(serde::de::Error::custom),
        N::I(i) => Ok(i),
    }
}

pub fn opt_i64_str<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    i64_str(d).map(Some)
}

/// Strip an enum's `SCREAMING_SNAKE` type prefix: `GROK_BOT_SEND_STATUS_ACCEPTED` -> `accepted`.
pub fn enum_suffix(value: &str, prefix: &str) -> String {
    value
        .strip_prefix(prefix)
        .unwrap_or(value)
        .trim_start_matches('_')
        .to_lowercase()
}

// ---- GetMe (DashboardService) ----

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
#[derive(Default)]
pub struct GetMeResponse {
    pub email: Option<String>,
    pub first_name: Option<String>,
    pub team_name: Option<String>,
    pub public_user_id: Option<String>,
}

// ---- ListGrokBotAgents ----

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ListAgentsRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_team_agents: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct ListAgentsResponse {
    pub agents: Vec<Agent>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Agent {
    /// Numeric row id; RPCs take `agent_id` (the UUID) instead.
    pub id: String,
    pub agent_id: String,
    pub name: String,
    pub description: String,
    pub kind: String,
    pub visibility: Option<String>,
    pub viewer_is_owner: Option<bool>,
    pub owner_display_name: Option<String>,
    #[serde(deserialize_with = "i64_str")]
    pub updated_at_ms: i64,
}

// ---- ListGrokBotAgentStatuses ----

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct ListStatusesResponse {
    pub statuses: Vec<AgentStatus>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentStatus {
    pub agent_id: String,
    pub is_running: bool,
    pub live: Option<LiveState>,
    pub client: Option<ClientState>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct LiveState {
    pub is_running: bool,
    pub is_composing_message: bool,
    pub activity: Option<LiveActivity>,
    pub awaiting: Option<Awaiting>,
    pub has_running_subagents: bool,
    pub working_task_ids: Vec<String>,
    pub last_turn_settlement: Option<TurnSettlement>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct LiveActivity {
    pub kind: String,
    pub tool: String,
    pub detail: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct Awaiting {
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct TurnSettlement {
    pub turn_id: String,
    pub outcome: String,
    #[serde(deserialize_with = "i64_str")]
    pub settled_at_ms: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ClientState {
    pub unread_count: u32,
    #[serde(deserialize_with = "opt_i64_str")]
    pub last_activity_at_ms: Option<i64>,
    pub last_message_preview: Option<String>,
}

// ---- SendGrokBotUserMessage / GetGrokBotSendStatus ----

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendRequest<'a> {
    pub agent_id: &'a str,
    /// Client-generated idempotency key; echoed back as the entry's `clientNonce`.
    pub message_id: &'a str,
    pub text: &'a str,
    pub sent_at_ms: String,
    #[serde(skip_serializing_if = "str::is_empty")]
    pub session_id: &'a str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attachment_paths: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attachment_names: Vec<String>,
    pub source: &'static str,
    pub initiator: &'static str,
    pub send_source: &'static str,
}

#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct SendResponse {
    pub dispatched: bool,
    pub delivery: String,
    pub workflow_id: Option<String>,
    pub refusal: Option<Refusal>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Refusal {
    pub failure_code: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendStatusRequest<'a> {
    pub agent_id: &'a str,
    pub message_id: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    pub session_id: &'a str,
}

#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct SendStatusResponse {
    pub status: String,
    pub echo_entry_id: Option<String>,
    pub rejection_code: Option<String>,
    #[serde(deserialize_with = "opt_i64_str")]
    pub accepted_at_ms: Option<i64>,
}

// ---- ListGrokBotTranscriptEntries ----

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListEntriesRequest<'a> {
    pub agent_id: &'a str,
    pub limit: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_seq: Option<String>,
    #[serde(skip_serializing_if = "str::is_empty")]
    pub session_id: &'a str,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct ListEntriesResponse {
    /// Newest first.
    pub entries: Vec<RawEntry>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct RawEntry {
    #[serde(deserialize_with = "i64_str")]
    pub seq: i64,
    pub entry_kind: String,
    pub entry_id: Option<String>,
    /// base64 of a JSON document; see transcript.rs.
    pub body: Option<String>,
    pub body_omitted: bool,
}

// ---- InterruptGrokBotAgentRun ----

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InterruptRequest<'a> {
    pub agent_id: &'a str,
    pub reason: &'static str,
    #[serde(skip_serializing_if = "str::is_empty")]
    pub session_id: &'a str,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct InterruptResponse {
    pub had_active_run: bool,
}

// ---- ListGrokBotAgentSessions ----

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct ListSessionsResponse {
    pub sessions: Vec<AgentSession>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentSession {
    pub session_id: String,
    pub kind: String,
    #[serde(deserialize_with = "i64_str")]
    pub created_at_ms: i64,
    #[serde(deserialize_with = "opt_i64_str")]
    pub last_activity_at_ms: Option<i64>,
}

// ---- ListGrokBotAgentTasks / ListGrokBotTasks ----

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct ListAgentTasksResponse {
    pub tasks: Vec<AgentTask>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentTask {
    pub id: String,
    pub name: String,
    pub description: String,
    pub status: String,
    #[serde(deserialize_with = "i64_str")]
    pub updated_at_ms: i64,
    pub subagent_ids: Vec<String>,
    pub unread_preview: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct ListSessionTasksResponse {
    pub tasks: Vec<SessionTask>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct SessionTask {
    pub id: String,
    pub title: String,
    pub state: String,
    pub note: String,
    #[serde(deserialize_with = "i64_str")]
    pub created_at_ms: i64,
    #[serde(deserialize_with = "i64_str")]
    pub done_at_ms: i64,
}

// ---- Automations (what the app shows as routines) ----
//
// `ListGrokBotAgentRoutines` answers NOT_FOUND ("routines are not available for this bot")
// for temporal-harness bots; their schedules live in automations, whose payload is a JSON
// string (`record_json`). See docs/commands.md#routines.

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct ListAccountAutomationsResponse {
    pub agents: Vec<AgentAutomations>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentAutomations {
    pub agent_id: String,
    pub automations: Vec<Automation>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Automation {
    pub automation_id: String,
    pub record_json: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct SetAutomationEnabledResponse {
    pub automations: Vec<Automation>,
}

/// The parts of `record_json` we use.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AutomationRecord {
    pub name: String,
    pub schedule: String,
    pub trigger_description: String,
    pub is_enabled: bool,
    pub next_run_at: Option<i64>,
    pub last_run_at: Option<i64>,
}

impl Automation {
    pub fn record(&self) -> AutomationRecord {
        serde_json::from_str(&self.record_json).unwrap_or_default()
    }
}

// ---- Attachments ----

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BeginUploadRequest<'a> {
    pub agent_id: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    pub session_id: &'a str,
    pub file_name: &'a str,
    pub size_bytes: String,
    pub supports_store_attachment_refs: bool,
    pub supports_multipart: bool,
    pub sha256_hex: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct BeginUploadResponse {
    pub mode: String,
    pub use_box_reason: String,
    pub refusal: String,
    pub upload_token: String,
    pub put: Option<UploadTarget>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct UploadTarget {
    pub url: String,
    pub headers: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct CompleteUploadResponse {
    pub refusal: String,
    pub alias: String,
    pub file_name: String,
}
