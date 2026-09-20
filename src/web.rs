//! Browser bindings for the same conversation state used by native consumers.

use wasm_bindgen::prelude::*;

/// A Rust conversation model exported to JavaScript. The application owns transport.
#[wasm_bindgen(js_name = ConversationModel)]
pub struct WebConversation {
    inner: crate::Conversation,
}

#[wasm_bindgen(js_class = ConversationModel)]
impl WebConversation {
    /// Create an isolated conversation state.
    #[wasm_bindgen(constructor)]
    pub fn new(conversation_id: String) -> Self {
        Self {
            inner: crate::Conversation::new(conversation_id),
        }
    }

    /// Apply an event and report whether it changed the state.
    #[wasm_bindgen(js_name = applyEvent)]
    pub fn apply_event(&mut self, event_json: &str) -> Result<bool, JsValue> {
        self.inner
            .apply_json(event_json)
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    /// Return the current render state as JSON.
    pub fn snapshot(&self) -> Result<String, JsValue> {
        self.inner
            .snapshot_json()
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }
}
