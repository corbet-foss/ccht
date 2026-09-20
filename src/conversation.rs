//! Platform-independent conversation events and rendering state.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::acp;

/// Version of the application-facing event envelope.
pub const WIRE_VERSION: u16 = 1;
/// Maximum accepted serialized event size.
pub const MAX_EVENT_BYTES: usize = 1024 * 1024;
const MAX_TURNS: usize = 256;
const MAX_TURN_BYTES: usize = 8 * 1024 * 1024;

/// A prompt whose correlation identity is assigned by the application.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Prompt {
    /// Unique identity for this delivery attempt. Never reuse it to retry an uncertain delivery.
    pub request_id: String,
    /// Official ACP content blocks, including any application-owned context.
    pub content: Vec<acp::ContentBlock>,
}

impl Prompt {
    /// Construct a plain-text prompt.
    pub fn text(request_id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            request_id: request_id.into(),
            content: vec![acp::ContentBlock::from(text.into())],
        }
    }
}

/// A native session event. Events during history loading have no active request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionEvent {
    /// Application request identity, when a prompt is active.
    pub request_id: Option<String>,
    /// Structured event payload.
    pub event: Event,
}

/// Shared event payload. Upstream ACP types preserve tool and capability details.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// Agent output, tools, plans, usage, or session configuration.
    Update {
        /// Original typed ACP update.
        update: acp::SessionUpdate,
    },
    /// A permission decision is required from the application's authority.
    Permission {
        /// Identity of the pending permission request, not the prompt identity.
        request_id: String,
        /// The options and tool activity reported by the agent.
        request: acp::RequestPermissionRequest,
    },
    /// The prompt finished. Cancellation and refusal remain distinct reasons.
    Completed {
        /// Agent-reported terminal reason.
        stop_reason: acp::StopReason,
    },
    /// A terminal failure with a safe, application-facing description.
    Error {
        /// Stable failure category.
        code: String,
        /// Sanitized description; never raw credentials or process output.
        message: String,
    },
}

/// An application-authorized event suitable for persistence, SSE or other transports.
///
/// Sequence numbers start at one and increase within each request. They are not
/// a database cursor. Applications authenticate/authorize the conversation before
/// handing events to a consumer; this envelope is not an authorization mechanism.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WireEvent {
    /// Protocol version, currently one.
    pub version: u16,
    /// Application-owned conversation identity, including role isolation where needed.
    pub conversation_id: String,
    /// Application-owned prompt/delivery identity.
    pub request_id: String,
    /// Contiguous sequence number within the request, starting at one.
    pub sequence: u64,
    /// Shared structured event.
    pub event: Event,
}

impl WireEvent {
    /// Wrap an event with its application-owned routing and ordering identity.
    pub fn new(
        conversation_id: impl Into<String>,
        request_id: impl Into<String>,
        sequence: u64,
        event: Event,
    ) -> Self {
        Self {
            version: WIRE_VERSION,
            conversation_id: conversation_id.into(),
            request_id: request_id.into(),
            sequence,
            event,
        }
    }
}

/// An unresolved permission prompt. Rendering this does not authorize the action.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingPermission {
    /// Native permission-request identity.
    pub request_id: String,
    /// Original request and the allowed response options.
    pub request: acp::RequestPermissionRequest,
}

/// Conversation processing failure. Rejected events do not mutate the snapshot.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ConversationError {
    /// The producer speaks an unsupported envelope version.
    #[error("unsupported ccht event version")]
    Version,
    /// The event belongs to a different conversation.
    #[error("event belongs to another conversation")]
    Scope,
    /// A request identifier or sequence is invalid.
    #[error("request identity and positive sequence are required")]
    Identity,
    /// A cursor skipped an event. The application must replay the missing event.
    #[error("event sequence has a gap; replay the missing events")]
    Gap,
    /// An event arrived after the request was terminal.
    #[error("request is already terminal")]
    Terminal,
    /// A bounded event or conversation exceeded its capacity.
    #[error("conversation capacity exceeded")]
    Capacity,
    /// The event is not valid ccht JSON.
    #[error("invalid ccht event JSON")]
    Json,
}

/// Renderable state for one prompt, shared between web and native consumers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TurnState {
    /// Application-owned request identity.
    pub request_id: String,
    /// Last applied event sequence.
    pub last_sequence: u64,
    /// Accumulated assistant text.
    pub text: String,
    /// User text supplied by upstream history replay, if any.
    pub user_text: String,
    /// Non-text user content from history replay, preserved separately by role.
    pub user_content: Vec<acp::ContentBlock>,
    /// Separately reported thought text; applications choose whether to display it.
    pub thought_text: String,
    /// Non-text thought content, preserved separately from visible assistant output.
    pub thought_content: Vec<acp::ContentBlock>,
    /// Non-text assistant content, preserved without flattening.
    pub content: Vec<acp::ContentBlock>,
    /// Current tool calls, including updates to their content and status.
    pub tools: BTreeMap<String, acp::ToolCall>,
    /// Unresolved permission requests.
    pub permissions: Vec<PendingPermission>,
    /// Latest other update of each ACP type, preserving plans/configuration/usage.
    pub updates: BTreeMap<String, serde_json::Value>,
    /// `streaming`, `awaiting_permission`, `completed`, `cancelled`, `refused`, or `failed`.
    pub status: String,
    /// Exact upstream terminal reason.
    pub stop_reason: Option<acp::StopReason>,
    /// Safe terminal error details.
    pub error: Option<serde_json::Value>,
}

impl TurnState {
    fn new(request_id: String) -> Self {
        Self {
            request_id,
            last_sequence: 0,
            text: String::new(),
            user_text: String::new(),
            user_content: Vec::new(),
            thought_text: String::new(),
            thought_content: Vec::new(),
            content: Vec::new(),
            tools: BTreeMap::new(),
            permissions: Vec::new(),
            updates: BTreeMap::new(),
            status: "streaming".into(),
            stop_reason: None,
            error: None,
        }
    }

    fn update(&mut self, update: acp::SessionUpdate) -> Result<(), ConversationError> {
        match update {
            acp::SessionUpdate::AgentMessageChunk(chunk) => match chunk.content {
                acp::ContentBlock::Text(text) => self.text.push_str(&text.text),
                other => self.content.push(other),
            },
            acp::SessionUpdate::UserMessageChunk(chunk) => match chunk.content {
                acp::ContentBlock::Text(text) => self.user_text.push_str(&text.text),
                other => self.user_content.push(other),
            },
            acp::SessionUpdate::AgentThoughtChunk(chunk) => match chunk.content {
                acp::ContentBlock::Text(text) => self.thought_text.push_str(&text.text),
                other => self.thought_content.push(other),
            },
            acp::SessionUpdate::ToolCall(call) => {
                self.tools.insert(call.tool_call_id.to_string(), call);
            }
            acp::SessionUpdate::ToolCallUpdate(update) => {
                let id = update.tool_call_id.to_string();
                let call = self.tools.entry(id).or_insert_with(|| {
                    acp::ToolCall::new(update.tool_call_id.clone(), "Tool activity")
                });
                let fields = update.fields;
                if let Some(v) = fields.title {
                    call.title = v;
                }
                if let Some(v) = fields.kind {
                    call.kind = v;
                }
                if let Some(v) = fields.status {
                    call.status = v;
                }
                if let Some(v) = fields.content {
                    call.content = v;
                }
                if let Some(v) = fields.locations {
                    call.locations = v;
                }
                if let Some(v) = fields.raw_input {
                    call.raw_input = Some(v);
                }
                if let Some(v) = fields.raw_output {
                    call.raw_output = Some(v);
                }
                if matches!(
                    call.status,
                    acp::ToolCallStatus::Completed | acp::ToolCallStatus::Failed
                ) {
                    self.permissions
                        .retain(|p| p.request.tool_call.tool_call_id != call.tool_call_id);
                }
            }
            other => {
                let value = serde_json::to_value(other).map_err(|_| ConversationError::Json)?;
                let key = value
                    .get("sessionUpdate")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_owned();
                self.updates.insert(key, value);
            }
        }
        self.status = if self.permissions.is_empty() {
            "streaming"
        } else {
            "awaiting_permission"
        }
        .into();
        Ok(())
    }
}

/// Bounded, deterministic conversation state. It owns no network, database or UI.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Conversation {
    conversation_id: String,
    turns: Vec<TurnState>,
    configuration: crate::SessionConfiguration,
}

impl Conversation {
    /// Create an empty conversation for one application-owned identity.
    pub fn new(conversation_id: impl Into<String>) -> Self {
        Self {
            conversation_id: conversation_id.into(),
            turns: Vec::new(),
            configuration: crate::SessionConfiguration::default(),
        }
    }

    /// Application-owned identity of this conversation.
    pub fn id(&self) -> &str {
        &self.conversation_id
    }

    /// Ordered render state, independent of any frontend framework.
    pub fn turns(&self) -> &[TurnState] {
        &self.turns
    }

    /// Latest session controls observed in the ordered event stream.
    pub fn configuration(&self) -> &crate::SessionConfiguration {
        &self.configuration
    }

    /// Serialize the render state for a browser or another transport.
    pub fn snapshot_json(&self) -> Result<String, ConversationError> {
        serde_json::to_string(self).map_err(|_| ConversationError::Json)
    }

    /// Parse and apply a bounded JSON envelope. Duplicate events return false.
    pub fn apply_json(&mut self, json: &str) -> Result<bool, ConversationError> {
        if json.len() > MAX_EVENT_BYTES {
            return Err(ConversationError::Capacity);
        }
        self.apply(serde_json::from_str(json).map_err(|_| ConversationError::Json)?)
    }

    /// Apply exactly one ordered event. Duplicate/replayed events return false.
    ///
    /// Gaps, scope mismatches and new events after completion fail explicitly.
    /// Applications retain the source event log and replay it on reconnection.
    pub fn apply(&mut self, wire: WireEvent) -> Result<bool, ConversationError> {
        if wire.version != WIRE_VERSION {
            return Err(ConversationError::Version);
        }
        if self.conversation_id.is_empty() || wire.conversation_id != self.conversation_id {
            return Err(ConversationError::Scope);
        }
        if wire.request_id.is_empty() || wire.sequence == 0 {
            return Err(ConversationError::Identity);
        }
        let index = self
            .turns
            .iter()
            .position(|t| t.request_id == wire.request_id);
        if let Some(i) = index {
            if wire.sequence <= self.turns[i].last_sequence {
                return Ok(false);
            }
            if self.turns[i].stop_reason.is_some() || self.turns[i].error.is_some() {
                return Err(ConversationError::Terminal);
            }
        } else if self.turns.len() >= MAX_TURNS {
            return Err(ConversationError::Capacity);
        }
        let mut turn = index
            .map(|i| self.turns[i].clone())
            .unwrap_or_else(|| TurnState::new(wire.request_id));
        if wire.sequence != turn.last_sequence + 1 {
            return Err(ConversationError::Gap);
        }
        if serde_json::to_vec(&wire.event)
            .map_err(|_| ConversationError::Json)?
            .len()
            > MAX_EVENT_BYTES
        {
            return Err(ConversationError::Capacity);
        }
        let mut configuration = self.configuration.clone();
        if let Event::Update { update } = &wire.event {
            configuration.apply(update);
        }
        match wire.event {
            Event::Update { update } => turn.update(update)?,
            Event::Permission {
                request_id,
                request,
            } => {
                if !turn.permissions.iter().any(|p| p.request_id == request_id) {
                    turn.permissions.push(PendingPermission {
                        request_id,
                        request,
                    });
                }
                turn.status = "awaiting_permission".into();
            }
            Event::Completed { stop_reason } => {
                turn.status = match stop_reason {
                    acp::StopReason::Cancelled => "cancelled",
                    acp::StopReason::Refusal => "refused",
                    _ => "completed",
                }
                .into();
                turn.stop_reason = Some(stop_reason);
                turn.permissions.clear();
            }
            Event::Error { code, message } => {
                turn.status = "failed".into();
                turn.error = Some(serde_json::json!({"code":code,"message":message}));
                turn.permissions.clear();
            }
        }
        turn.last_sequence = wire.sequence;
        if serde_json::to_vec(&turn)
            .map_err(|_| ConversationError::Json)?
            .len()
            > MAX_TURN_BYTES
        {
            return Err(ConversationError::Capacity);
        }
        self.configuration = configuration;
        match index {
            Some(i) => self.turns[i] = turn,
            None => self.turns.push(turn),
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delta(conversation: &str, request: &str, sequence: u64, text: &str) -> WireEvent {
        WireEvent::new(
            conversation,
            request,
            sequence,
            Event::Update {
                update: acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(text.into())),
            },
        )
    }

    #[test]
    fn isolated_conversations_and_idempotent_replay() {
        let mut creator = Conversation::new("creator");
        let mut critic = Conversation::new("critic");
        let first = delta("creator", "r1", 1, "Hello");
        assert!(creator.apply(first.clone()).unwrap());
        assert!(!creator.apply(first.clone()).unwrap());
        assert_eq!(critic.apply(first), Err(ConversationError::Scope));
        assert!(critic.turns().is_empty());
        creator.apply(delta("creator", "r1", 2, " world")).unwrap();
        assert_eq!(creator.turns()[0].text, "Hello world");
    }

    #[test]
    fn gaps_and_late_updates_do_not_mutate_state() {
        let mut conversation = Conversation::new("c");
        assert_eq!(
            conversation.apply(delta("c", "r", 2, "lost")),
            Err(ConversationError::Gap)
        );
        assert!(conversation.turns().is_empty());
        conversation.apply(delta("c", "r", 1, "partial")).unwrap();
        conversation
            .apply(WireEvent::new(
                "c",
                "r",
                2,
                Event::Completed {
                    stop_reason: acp::StopReason::Cancelled,
                },
            ))
            .unwrap();
        let before = conversation.snapshot_json().unwrap();
        assert_eq!(
            conversation.apply(delta("c", "r", 3, "late")),
            Err(ConversationError::Terminal)
        );
        assert_eq!(before, conversation.snapshot_json().unwrap());
        assert_eq!(conversation.turns()[0].status, "cancelled");
    }

    #[test]
    fn non_text_content_preserves_user_assistant_and_thought_roles() {
        let mut conversation = Conversation::new("c");
        let image: acp::ContentBlock = serde_json::from_value(serde_json::json!({
            "type": "image", "data": "aW1hZ2U=", "mimeType": "image/png"
        }))
        .unwrap();
        for (sequence, update) in [
            (
                1,
                acp::SessionUpdate::UserMessageChunk(acp::ContentChunk::new(image.clone())),
            ),
            (
                2,
                acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(image.clone())),
            ),
            (
                3,
                acp::SessionUpdate::AgentThoughtChunk(acp::ContentChunk::new(image.clone())),
            ),
        ] {
            conversation
                .apply(WireEvent::new("c", "r", sequence, Event::Update { update }))
                .unwrap();
        }
        let turn = &conversation.turns()[0];
        assert_eq!(turn.user_content.as_slice(), std::slice::from_ref(&image));
        assert_eq!(turn.content.as_slice(), std::slice::from_ref(&image));
        assert_eq!(turn.thought_content, [image]);
        assert!(turn.text.is_empty());
        assert!(turn.user_text.is_empty());
        assert!(turn.thought_text.is_empty());
    }

    #[test]
    fn tool_updates_preserve_structure_and_resolve_permissions() {
        let mut conversation = Conversation::new("c");
        let call = acp::ToolCall::new("tool-1", "Edit document");
        conversation
            .apply(WireEvent::new(
                "c",
                "r",
                1,
                Event::Update {
                    update: acp::SessionUpdate::ToolCall(call),
                },
            ))
            .unwrap();
        let permission = acp::RequestPermissionRequest::new(
            "native",
            acp::ToolCallUpdate::new("tool-1", acp::ToolCallUpdateFields::new()),
            vec![acp::PermissionOption::new(
                "deny",
                "Deny",
                acp::PermissionOptionKind::RejectOnce,
            )],
        );
        conversation
            .apply(WireEvent::new(
                "c",
                "r",
                2,
                Event::Permission {
                    request_id: "p1".into(),
                    request: permission,
                },
            ))
            .unwrap();
        assert_eq!(conversation.turns()[0].status, "awaiting_permission");
        let fields = acp::ToolCallUpdateFields::new().status(acp::ToolCallStatus::Failed);
        conversation
            .apply(WireEvent::new(
                "c",
                "r",
                3,
                Event::Update {
                    update: acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
                        "tool-1", fields,
                    )),
                },
            ))
            .unwrap();
        assert!(conversation.turns()[0].permissions.is_empty());
        assert_eq!(
            conversation.turns()[0].tools["tool-1"].status,
            acp::ToolCallStatus::Failed
        );
    }

    #[test]
    fn invalid_json_versions_and_capacity_fail_without_mutation() {
        let mut conversation = Conversation::new("c");
        assert_eq!(conversation.apply_json("{"), Err(ConversationError::Json));
        let mut event = delta("c", "r", 1, "text");
        event.version = 2;
        assert_eq!(conversation.apply(event), Err(ConversationError::Version));
        let oversized = "x".repeat(MAX_EVENT_BYTES + 1);
        assert_eq!(
            conversation.apply_json(&oversized),
            Err(ConversationError::Capacity)
        );
        assert!(conversation.turns().is_empty());
    }

    #[test]
    fn prompt_text_and_error_events_shape_turn_status() {
        let prompt = Prompt::text("request-1", "hello");
        assert_eq!(prompt.request_id, "request-1");
        assert_eq!(prompt.content.len(), 1);

        let mut conversation = Conversation::new("c");
        conversation.apply(delta("c", "r", 1, "partial")).unwrap();
        conversation
            .apply(WireEvent::new(
                "c",
                "r",
                2,
                Event::Error {
                    code: "closed".into(),
                    message: "connection closed".into(),
                },
            ))
            .unwrap();
        let turn = &conversation.turns()[0];
        assert_eq!(turn.status, "failed");
        assert!(turn.permissions.is_empty());
        assert!(turn.error.is_some());
    }
}
