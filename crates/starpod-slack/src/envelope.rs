//! Slack Socket Mode envelope types.
//!
//! Socket Mode wraps every server-to-client message in a JSON envelope with a
//! top-level `type` field. Per the Slack docs
//! (<https://docs.slack.dev/apis/events-api/using-socket-mode/>) the envelope
//! types we care about are:
//!
//! - `hello` — sent immediately after connect with connection metadata.
//! - `events_api` — wraps a normal Events API payload (e.g. `app_mention`).
//! - `slash_commands` — slash command invocations.
//! - `interactive` — block actions, modal submissions, shortcuts.
//! - `disconnect` — Slack is closing this connection; client must reconnect.
//!
//! For Milestone 1 we only **route** on the type. Field-level handling of
//! `events_api` payloads happens in Milestone 2. We deliberately keep the
//! payload as a raw `serde_json::Value` so unknown shapes do not crash the
//! parser — Slack adds new event types regularly and a Socket Mode bot must
//! survive that.
//!
//! ## Forward compatibility
//!
//! Any envelope type Slack adds in the future is captured by
//! [`Envelope::Unknown`] and logged at debug level rather than dropped. This
//! mirrors how `teloxide` and the official Slack SDKs handle protocol
//! evolution.

use serde::{Deserialize, Serialize};

/// A message received from Slack over the Socket Mode WebSocket.
///
/// The variant is selected by the top-level `type` field. Unknown types fall
/// through to [`Envelope::Unknown`] so the receive loop never panics on a
/// new server-side envelope type.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Envelope {
    /// Sent by Slack immediately after the WebSocket handshake completes.
    ///
    /// Contains connection metadata including the number of currently active
    /// connections for this app-level token. We log a warning if
    /// `num_connections > 1` because that almost always indicates a duplicate
    /// process holding the same `xapp-...` token, which causes events to be
    /// distributed unpredictably across processes.
    Hello(Hello),

    /// An Events API event wrapped in a Socket Mode envelope.
    ///
    /// The `payload` field contains the same JSON shape Slack would POST to
    /// an HTTP `request_url`. We keep it as raw `Value` for Milestone 1 and
    /// model the typed inner `event` in Milestone 2.
    EventsApi(EventsApi),

    /// A slash command invocation. Not handled in phase 1; the client still
    /// acks it so Slack stops retrying.
    SlashCommands(SlashCommands),

    /// An interactive component (block action, modal submission, shortcut).
    /// Not handled in phase 1.
    Interactive(Interactive),

    /// Slack is asking us to reconnect.
    ///
    /// Sent ~10 seconds before Slack tears down the WebSocket so that
    /// well-behaved clients can establish a new connection without dropping
    /// events. The receive loop returns `Ok(())` on this variant and the
    /// outer reconnect loop opens a fresh connection.
    Disconnect(Disconnect),

    /// Any envelope type the crate does not know about yet.
    ///
    /// Forward-compatible with future Slack protocol additions. The receive
    /// loop logs these at debug level and continues.
    #[serde(other, deserialize_with = "deserialize_unknown")]
    Unknown,
}

/// `hello` envelope payload.
#[derive(Debug, Clone, Deserialize)]
pub struct Hello {
    /// Number of active WebSocket connections for the app-level token Slack
    /// used to mint the connection URL.
    ///
    /// Should always be `1` for Spawner — multiple connections sharing one
    /// token cause events to be load-balanced unpredictably across processes.
    #[serde(default)]
    pub num_connections: u32,

    /// Connection-level metadata Slack passes through. We don't act on it
    /// in Milestone 1 but logging it is useful for triaging connection
    /// issues.
    #[serde(default)]
    pub connection_info: Option<ConnectionInfo>,

    /// Build/version info Slack reports for the Socket Mode endpoint.
    #[serde(default)]
    pub debug_info: Option<serde_json::Value>,
}

/// Slack-side connection identifier returned in `hello.connection_info`.
#[derive(Debug, Clone, Deserialize)]
pub struct ConnectionInfo {
    pub app_id: String,
}

/// `events_api` envelope payload.
#[derive(Debug, Clone, Deserialize)]
pub struct EventsApi {
    /// Per-message identifier used to acknowledge receipt over the WS.
    ///
    /// Every `events_api` (and `slash_commands`, `interactive`) envelope
    /// must be acked within ~3 seconds by sending `{"envelope_id": "..."}`
    /// back over the same WS or Slack will retry delivery.
    pub envelope_id: String,

    /// The wrapped Events API payload — same shape as a normal HTTP delivery.
    /// Modeled as `Value` in Milestone 1; typed in Milestone 2.
    pub payload: serde_json::Value,

    /// Whether the ack message can include a response payload (for
    /// slash-command-style inline replies). Always `false` for `app_mention`
    /// and `message.*` events.
    #[serde(default)]
    pub accepts_response_payload: bool,

    /// Slack's redelivery counter — non-zero if this is a retry of an
    /// event we previously failed to ack.
    #[serde(default)]
    pub retry_attempt: u32,

    /// Reason Slack retried, e.g. `"timeout"`.
    #[serde(default)]
    pub retry_reason: Option<String>,
}

/// `slash_commands` envelope payload (placeholder; not handled in phase 1).
#[derive(Debug, Clone, Deserialize)]
pub struct SlashCommands {
    pub envelope_id: String,
    pub payload: serde_json::Value,
    #[serde(default)]
    pub accepts_response_payload: bool,
}

/// `interactive` envelope payload (placeholder; not handled in phase 1).
#[derive(Debug, Clone, Deserialize)]
pub struct Interactive {
    pub envelope_id: String,
    pub payload: serde_json::Value,
    #[serde(default)]
    pub accepts_response_payload: bool,
}

/// `disconnect` envelope payload.
#[derive(Debug, Clone, Deserialize)]
pub struct Disconnect {
    /// Why Slack is asking us to disconnect. Common values:
    /// - `"warning"` — generic notice; reconnect at leisure.
    /// - `"refresh_requested"` — Slack is recycling the WebSocket container.
    /// - `"link_disabled"` — the WS URL has been invalidated.
    #[serde(default)]
    pub reason: Option<String>,

    /// Slack debug payload — include in logs when surfacing connection
    /// problems.
    #[serde(default)]
    pub debug_info: Option<serde_json::Value>,
}

impl Envelope {
    /// Short, log-friendly name for the envelope variant.
    ///
    /// Used by the receive loop to emit metrics and structured log entries
    /// without exposing payload content.
    pub fn variant_name(&self) -> &'static str {
        match self {
            Envelope::Hello(_) => "hello",
            Envelope::EventsApi(_) => "events_api",
            Envelope::SlashCommands(_) => "slash_commands",
            Envelope::Interactive(_) => "interactive",
            Envelope::Disconnect(_) => "disconnect",
            Envelope::Unknown => "unknown",
        }
    }

    /// Returns the `envelope_id` for variants that require an ack, otherwise
    /// `None`.
    ///
    /// `hello`, `disconnect`, and `unknown` envelopes are not acked.
    pub fn envelope_id(&self) -> Option<&str> {
        match self {
            Envelope::EventsApi(e) => Some(&e.envelope_id),
            Envelope::SlashCommands(e) => Some(&e.envelope_id),
            Envelope::Interactive(e) => Some(&e.envelope_id),
            Envelope::Hello(_) | Envelope::Disconnect(_) | Envelope::Unknown => None,
        }
    }
}

// `serde(other)` requires a unit variant and a deserializer that just
// consumes the input and returns nothing — this helper does that.
fn deserialize_unknown<'de, D>(deserializer: D) -> std::result::Result<(), D::Error>
where
    D: serde::Deserializer<'de>,
{
    serde::de::IgnoredAny::deserialize(deserializer).map(|_| ())
}

/// Acknowledgement message sent from client to Slack over the WebSocket.
///
/// Slack expects this for every `events_api`, `slash_commands`, and
/// `interactive` envelope within ~3 seconds. The shape is `{"envelope_id":
/// "<id>"}` with an optional `payload` field for envelopes whose
/// `accepts_response_payload` is true (only used by slash commands in
/// phase 1; we never set it).
#[derive(Debug, Clone, Serialize)]
pub struct Ack<'a> {
    pub envelope_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

impl<'a> Ack<'a> {
    /// Build a bare ack for an envelope ID.
    pub fn new(envelope_id: &'a str) -> Self {
        Self {
            envelope_id,
            payload: None,
        }
    }

    /// Serialize the ack to a JSON string ready to send over the WebSocket.
    ///
    /// The serialization is infallible in practice because the inputs are
    /// owned-string references, but we propagate the [`serde_json::Error`]
    /// just in case a future field is added that can fail.
    pub fn to_json(&self) -> std::result::Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(value: serde_json::Value) -> Envelope {
        serde_json::from_value(value).expect("envelope must parse")
    }

    #[test]
    fn parses_hello() {
        let env = parse(json!({
            "type": "hello",
            "num_connections": 1,
            "connection_info": { "app_id": "A0KRD7HC3" },
            "debug_info": { "host": "applink-1" }
        }));
        match env {
            Envelope::Hello(h) => {
                assert_eq!(h.num_connections, 1);
                assert_eq!(h.connection_info.unwrap().app_id, "A0KRD7HC3");
            }
            other => panic!("expected Hello, got {:?}", other),
        }
    }

    #[test]
    fn hello_tolerates_missing_optional_fields() {
        let env = parse(json!({ "type": "hello" }));
        match env {
            Envelope::Hello(h) => {
                assert_eq!(h.num_connections, 0);
                assert!(h.connection_info.is_none());
                assert!(h.debug_info.is_none());
            }
            other => panic!("expected Hello, got {:?}", other),
        }
    }

    #[test]
    fn parses_events_api_app_mention() {
        let env = parse(json!({
            "type": "events_api",
            "envelope_id": "f0e0a0a0-0000-4000-8000-000000000000",
            "payload": {
                "team_id": "T0001",
                "event": {
                    "type": "app_mention",
                    "user": "U061F7AUR",
                    "text": "<@U0LAN0Z89> is it everything?",
                    "ts": "1515449522.000016",
                    "channel": "C0LAN2Q65",
                    "event_ts": "1515449522000016"
                },
                "event_id": "Ev0LAN670R"
            },
            "accepts_response_payload": false
        }));
        match env {
            Envelope::EventsApi(e) => {
                assert_eq!(e.envelope_id, "f0e0a0a0-0000-4000-8000-000000000000");
                assert!(!e.accepts_response_payload);
                assert_eq!(e.retry_attempt, 0);
                assert!(e.retry_reason.is_none());
                assert_eq!(
                    e.payload
                        .get("event")
                        .and_then(|v| v.get("type"))
                        .and_then(|v| v.as_str()),
                    Some("app_mention")
                );
            }
            other => panic!("expected EventsApi, got {:?}", other),
        }
    }

    #[test]
    fn parses_events_api_with_retry_metadata() {
        let env = parse(json!({
            "type": "events_api",
            "envelope_id": "abc",
            "payload": {},
            "retry_attempt": 2,
            "retry_reason": "timeout"
        }));
        match env {
            Envelope::EventsApi(e) => {
                assert_eq!(e.retry_attempt, 2);
                assert_eq!(e.retry_reason.as_deref(), Some("timeout"));
            }
            other => panic!("expected EventsApi, got {:?}", other),
        }
    }

    #[test]
    fn parses_slash_commands() {
        let env = parse(json!({
            "type": "slash_commands",
            "envelope_id": "id-1",
            "payload": { "command": "/starpod" },
            "accepts_response_payload": true
        }));
        match env {
            Envelope::SlashCommands(s) => {
                assert_eq!(s.envelope_id, "id-1");
                assert!(s.accepts_response_payload);
            }
            other => panic!("expected SlashCommands, got {:?}", other),
        }
    }

    #[test]
    fn parses_interactive() {
        let env = parse(json!({
            "type": "interactive",
            "envelope_id": "id-2",
            "payload": { "type": "block_actions" }
        }));
        match env {
            Envelope::Interactive(i) => {
                assert_eq!(i.envelope_id, "id-2");
            }
            other => panic!("expected Interactive, got {:?}", other),
        }
    }

    #[test]
    fn parses_disconnect_with_reason() {
        let env = parse(json!({
            "type": "disconnect",
            "reason": "refresh_requested",
            "debug_info": { "host": "applink-3" }
        }));
        match env {
            Envelope::Disconnect(d) => {
                assert_eq!(d.reason.as_deref(), Some("refresh_requested"));
                assert!(d.debug_info.is_some());
            }
            other => panic!("expected Disconnect, got {:?}", other),
        }
    }

    #[test]
    fn parses_disconnect_without_reason() {
        let env = parse(json!({ "type": "disconnect" }));
        match env {
            Envelope::Disconnect(d) => assert!(d.reason.is_none()),
            other => panic!("expected Disconnect, got {:?}", other),
        }
    }

    #[test]
    fn unknown_type_falls_through_to_unknown() {
        // Future Slack envelope type that doesn't exist yet — must not crash.
        let env = parse(json!({
            "type": "future_unknown_envelope",
            "weird_field": [1, 2, 3]
        }));
        assert!(matches!(env, Envelope::Unknown));
    }

    #[test]
    fn variant_name_is_stable_for_metrics() {
        assert_eq!(parse(json!({"type": "hello"})).variant_name(), "hello");
        assert_eq!(
            parse(json!({"type": "disconnect"})).variant_name(),
            "disconnect"
        );
        assert_eq!(
            parse(json!({"type": "unknown_xyz"})).variant_name(),
            "unknown"
        );
    }

    #[test]
    fn envelope_id_is_present_only_for_ackable_variants() {
        let events_api = parse(json!({
            "type": "events_api",
            "envelope_id": "e1",
            "payload": {}
        }));
        assert_eq!(events_api.envelope_id(), Some("e1"));

        let hello = parse(json!({"type": "hello"}));
        assert_eq!(hello.envelope_id(), None);

        let disconnect = parse(json!({"type": "disconnect"}));
        assert_eq!(disconnect.envelope_id(), None);
    }

    #[test]
    fn ack_serializes_to_minimal_json() {
        let ack = Ack::new("envelope-123");
        let json = ack.to_json().unwrap();
        // Must round-trip to the exact shape Slack expects.
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, json!({ "envelope_id": "envelope-123" }));
    }

    #[test]
    fn ack_with_payload_includes_payload_field() {
        let ack = Ack {
            envelope_id: "e",
            payload: Some(json!({"text": "hello"})),
        };
        let parsed: serde_json::Value = serde_json::from_str(&ack.to_json().unwrap()).unwrap();
        assert_eq!(
            parsed,
            json!({ "envelope_id": "e", "payload": { "text": "hello" } })
        );
    }
}
