//! Reusable conversations for native applications and the web.
//!
//! The conversation model and event contract compile to native Rust and Wasm.
//! Native connections use the official ACP SDK; applications own their UI,
//! persistence, prompts and authorization decisions.
//!
//! ```
//! use ccht::{Conversation, Event, WireEvent, acp};
//! let mut conversation = Conversation::new("creator");
//! let event = WireEvent::new("creator", "turn-1", 1,
//!     Event::Update { update: acp::SessionUpdate::AgentMessageChunk(
//!         acp::ContentChunk::new("Hello".into())) });
//! conversation.apply(event).unwrap();
//! assert_eq!(conversation.turns()[0].text, "Hello");
//! ```

/// The official platform-independent ACP version 1 schema.
pub use agent_client_protocol_schema::v1 as acp;

mod configuration;
pub use configuration::*;

mod auth;
pub use auth::*;

mod conversation;
pub use conversation::*;

mod transport;
pub use transport::*;

mod dock;
pub use dock::*;

#[cfg(all(feature = "native", not(target_family = "wasm")))]
pub mod native;

#[cfg(feature = "web")]
mod web;
