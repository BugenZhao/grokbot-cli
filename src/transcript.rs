//! Decoding transcript entries.
//!
//! `GrokBotTranscriptEntry.body` is bytes holding a JSON document whose `kind` matches
//! `entry_kind`. This shape is *not* part of the protobuf schema, so it was mapped by
//! observation (docs/transcript.md). Anything unrecognised is kept as `Other` and still
//! available raw via `--json`.

use anyhow::Result;
use base64::Engine;
use serde_json::Value;

use crate::api::{ListEntriesRequest, ListEntriesResponse, RawEntry, SVC};
use crate::connect::Client;

#[derive(Debug, Clone)]
pub struct Entry {
    pub seq: i64,
    pub id: String,
    pub kind: String,
    pub timestamp_ms: i64,
    /// Turn id; all entries produced while handling one user message share it.
    pub request_id: Option<String>,
    pub body: Value,
}

/// What an entry means to a reader.
pub enum View<'a> {
    /// The user's own message (`message`, role `user`). `nonce` is our `message_id`.
    User {
        text: &'a str,
        nonce: Option<&'a str>,
    },
    /// A message the bot showed the user (`send-message`, type `text`).
    Reply {
        text: &'a str,
    },
    /// The bot is waiting for a click/secret/approval (widget, secret-request, *-draft …).
    NeedsInput {
        what: String,
        answered: bool,
    },
    /// Bot-to-bot message (`message`, role `assistant`, with `toAgent`).
    AgentMessage {
        to: Option<&'a str>,
        text: &'a str,
    },
    Attachment {
        name: &'a str,
    },
    Event {
        summary: String,
    },
    Other,
}

impl Entry {
    pub fn decode(raw: &RawEntry) -> Self {
        let body: Value = raw
            .body
            .as_deref()
            .and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(Value::Null);
        Self {
            seq: raw.seq,
            id: raw
                .entry_id
                .clone()
                .or_else(|| body["id"].as_str().map(str::to_owned))
                .unwrap_or_default(),
            kind: raw.entry_kind.clone(),
            timestamp_ms: body["timestampMs"].as_i64().unwrap_or(0),
            request_id: body["requestId"].as_str().map(str::to_owned),
            body,
        }
    }

    pub fn view(&self) -> View<'_> {
        let b = &self.body;
        match self.kind.as_str() {
            "message" if b["role"] == "user" => View::User {
                text: b["content"].as_str().unwrap_or(""),
                nonce: b["clientNonce"].as_str(),
            },
            "message" => View::AgentMessage {
                to: b["toAgent"]["name"].as_str(),
                text: b["content"].as_str().unwrap_or(""),
            },
            "send-message" => {
                let m = &b["message"];
                match m["type"].as_str().unwrap_or("") {
                    "text" => View::Reply {
                        text: m["content"].as_str().unwrap_or(""),
                    },
                    "widget" => View::NeedsInput {
                        what: format!(
                            "{} [{}]",
                            m["widget"]["prompt"].as_str().unwrap_or("choose an option"),
                            m["widget"]["options"]
                                .as_array()
                                .map(|o| o
                                    .iter()
                                    .filter_map(|o| o["label"].as_str())
                                    .collect::<Vec<_>>()
                                    .join(" / "))
                                .unwrap_or_default()
                        ),
                        answered: b.get("respondedValue").is_some(),
                    },
                    "secret-request" => View::NeedsInput {
                        what: format!(
                            "secret: {}",
                            m["secretRequest"]["label"].as_str().unwrap_or("?")
                        ),
                        answered: b.get("secretProvided").is_some(),
                    },
                    t if t.ends_with("-draft") => View::NeedsInput {
                        what: format!("{t} to review"),
                        answered: b.get("draftSendState").is_some(),
                    },
                    t if t.ends_with("-approval") => View::NeedsInput {
                        what: t.to_owned(),
                        answered: b.get("respondedValue").is_some()
                            || b.get("resolution").is_some(),
                    },
                    _ => View::Other,
                }
            }
            "user-attachment" => View::Attachment {
                name: b["file_name"].as_str().unwrap_or("?"),
            },
            "event" => {
                let e = &b["event"];
                let name = e["automationName"]
                    .as_str()
                    .or(e["name"].as_str())
                    .unwrap_or("");
                View::Event {
                    summary: format!(
                        "{} {} {name}",
                        e["type"].as_str().unwrap_or("event"),
                        e["action"].as_str().unwrap_or("")
                    )
                    .trim()
                    .to_owned(),
                }
            }
            _ => View::Other,
        }
    }
}

/// Fetch one page (newest first).
pub fn page(
    client: &Client,
    agent: &str,
    session: &str,
    limit: u32,
    before: Option<i64>,
) -> Result<Vec<Entry>> {
    let resp: ListEntriesResponse = client.call(
        &format!("{SVC}/ListGrokBotTranscriptEntries"),
        &ListEntriesRequest {
            agent_id: agent,
            limit,
            before_seq: before.map(|s| s.to_string()),
            session_id: session,
        },
    )?;
    Ok(resp.entries.iter().map(Entry::decode).collect())
}

/// All entries with `seq > after`, oldest first (pages backwards as needed).
pub fn since(client: &Client, agent: &str, session: &str, after: i64) -> Result<Vec<Entry>> {
    const PAGE: u32 = 50;
    let mut out = Vec::new();
    let mut before = None;
    loop {
        let entries = page(client, agent, session, PAGE, before)?;
        let n = entries.len();
        let mut done = n < PAGE as usize;
        for e in entries {
            if e.seq <= after {
                done = true;
                break;
            }
            before = Some(e.seq);
            out.push(e);
        }
        if done {
            break;
        }
    }
    out.reverse();
    Ok(out)
}

/// The most recent `n` entries, oldest first.
pub fn last(
    client: &Client,
    agent: &str,
    session: &str,
    n: u32,
    before: Option<i64>,
) -> Result<Vec<Entry>> {
    let mut out = Vec::new();
    let mut before = before;
    while out.len() < n as usize {
        let want = (n as usize - out.len()).min(200) as u32;
        let entries = page(client, agent, session, want, before)?;
        let got = entries.len();
        if let Some(e) = entries.last() {
            before = Some(e.seq);
        }
        out.extend(entries);
        if got < want as usize {
            break;
        }
    }
    out.reverse();
    Ok(out)
}

pub fn latest_seq(client: &Client, agent: &str, session: &str) -> Result<i64> {
    Ok(page(client, agent, session, 1, None)?
        .first()
        .map_or(0, |e| e.seq))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: &str, body: serde_json::Value) -> Entry {
        let raw = RawEntry {
            seq: 7,
            entry_kind: kind.into(),
            entry_id: None,
            body: Some(base64::engine::general_purpose::STANDARD.encode(body.to_string())),
            body_omitted: false,
        };
        Entry::decode(&raw)
    }

    #[test]
    fn decodes_common_fields() {
        let e = entry(
            "send-message",
            serde_json::json!({"kind":"send-message","id":"t1s0","timestampMs":5,"requestId":"turn",
                               "message":{"type":"text","content":"hi"}}),
        );
        assert_eq!((e.seq, e.id.as_str(), e.timestamp_ms), (7, "t1s0", 5));
        assert_eq!(e.request_id.as_deref(), Some("turn"));
        assert!(matches!(e.view(), View::Reply { text: "hi" }));
    }

    #[test]
    fn classifies_entry_kinds() {
        let user = entry(
            "message",
            serde_json::json!({"role":"user","content":"q","clientNonce":"n"}),
        );
        assert!(matches!(
            user.view(),
            View::User {
                text: "q",
                nonce: Some("n")
            }
        ));

        let to_agent = entry(
            "message",
            serde_json::json!({"role":"assistant","content":"c","toAgent":{"name":"B"}}),
        );
        assert!(matches!(
            to_agent.view(),
            View::AgentMessage {
                to: Some("B"),
                text: "c"
            }
        ));

        let widget = serde_json::json!({"message":{"type":"widget","widget":{"prompt":"A or B?",
            "options":[{"label":"A"},{"label":"B"}]}}});
        match entry("send-message", widget.clone()).view() {
            View::NeedsInput { what, answered } => {
                assert_eq!(what, "A or B? [A / B]");
                assert!(!answered);
            }
            _ => panic!("widget not classified as needs-input"),
        }
        let mut answered = widget;
        answered["respondedValue"] = "A".into();
        assert!(matches!(
            entry("send-message", answered).view(),
            View::NeedsInput { answered: true, .. }
        ));

        let draft = entry(
            "send-message",
            serde_json::json!({"message":{"type":"email-draft"}}),
        );
        assert!(matches!(
            draft.view(),
            View::NeedsInput {
                answered: false,
                ..
            }
        ));

        let att = entry("user-attachment", serde_json::json!({"file_name":"a.txt"}));
        assert!(matches!(att.view(), View::Attachment { name: "a.txt" }));

        assert!(matches!(
            entry("voice-call", serde_json::json!({})).view(),
            View::Other
        ));
    }

    #[test]
    fn tolerates_missing_or_bad_body() {
        let raw = RawEntry {
            seq: 1,
            entry_kind: "message".into(),
            body: Some("!!".into()),
            ..Default::default()
        };
        let e = Entry::decode(&raw);
        assert!(e.body.is_null());
        assert!(matches!(e.view(), View::AgentMessage { text: "", .. }));
    }
}
