//! Portable headless dock state shared by native, Wasm and web consumers.
//!
//! This module only tracks which docks exist, where they are placed, and
//! whether they are open. It performs no I/O, spawns nothing, and renders no
//! UI; applications own product flows, persistence, and presentation.

use serde::{Deserialize, Serialize};

/// What a dock is for. Product-neutral; applications map kinds to views.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DockKind {
    /// Conversation or chat surface.
    Chat,
    /// Configuration or settings surface.
    Config,
    /// Application-defined surface.
    Custom,
}

/// Where a dock is placed relative to the main surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Placement {
    /// Docked to the left side.
    Left,
    /// Docked to the right side.
    Right,
    /// Docked to the bottom.
    Bottom,
    /// Rendered inline with the main content.
    Inline,
}

/// Validated dock identifier: non-empty, at most 64 characters, charset
/// `[a-z0-9-_]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DockId(String);

impl DockId {
    /// Maximum identifier length in characters.
    const MAX_LEN: usize = 64;

    /// Validate an identifier without performing any I/O.
    ///
    /// # Errors
    ///
    /// Returns [`DockError::EmptyId`] for empty input,
    /// [`DockError::IdTooLong`] when longer than 64 characters, or
    /// [`DockError::InvalidChar`] for the first out-of-charset character.
    pub fn new(id: &str) -> Result<Self, DockError> {
        if id.is_empty() {
            return Err(DockError::EmptyId);
        }
        if id.chars().count() > Self::MAX_LEN {
            return Err(DockError::IdTooLong);
        }
        if let Some(invalid) = id
            .chars()
            .find(|c| !matches!(c, 'a'..='z' | '0'..='9' | '-' | '_'))
        {
            return Err(DockError::InvalidChar(invalid));
        }
        Ok(Self(id.to_owned()))
    }

    /// Borrow the identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Dock validation and lookup failure. Messages are fixed strings and never
/// carry focus tokens or other sensitive data.
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum DockError {
    /// The identifier is empty.
    #[error("dock id must not be empty")]
    EmptyId,
    /// The identifier exceeds 64 characters.
    #[error("dock id must be at most 64 characters")]
    IdTooLong,
    /// The identifier contains a character outside `[a-z0-9-_]`.
    #[error("dock id contains invalid character: {0}")]
    InvalidChar(char),
    /// No dock is registered under this identifier.
    #[error("unknown dock: {0}")]
    UnknownDock(String),
    /// A dock is already registered under this identifier.
    #[error("duplicate dock: {0}")]
    DuplicateDock(String),
}

/// Runtime state of a single registered dock.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockState {
    /// Validated dock identifier.
    pub id: DockId,
    /// What the dock is for.
    pub kind: DockKind,
    /// Where the dock is placed.
    pub placement: Placement,
    /// Whether the dock is currently open.
    pub open: bool,
}

/// Headless registry of docks plus a transient focus token.
///
/// The focus token is never serialized; it only travels in memory so the
/// application can restore focus after opening a dock.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockManager {
    /// Registered docks in registration order.
    pub docks: Vec<DockState>,
    /// Transient focus token, replaced by `open_with_focus` and consumed by
    /// `take_focus_token`. Skipped by serde.
    #[serde(skip, default)]
    pub focus_token: Option<String>,
}

impl DockManager {
    /// Empty registry with no focus token.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Find a dock index by identifier text.
    fn index_of(&self, id: &str) -> Option<usize> {
        self.docks.iter().position(|dock| dock.id.as_str() == id)
    }

    /// Register a new closed dock.
    ///
    /// # Errors
    ///
    /// Returns [`DockError::EmptyId`], [`DockError::IdTooLong`] or
    /// [`DockError::InvalidChar`] for invalid identifiers, or
    /// [`DockError::DuplicateDock`] when the identifier is already registered.
    pub fn register(
        &mut self,
        id: &str,
        kind: DockKind,
        placement: Placement,
    ) -> Result<(), DockError> {
        let validated = DockId::new(id)?;
        if self.docks.iter().any(|dock| dock.id == validated) {
            return Err(DockError::DuplicateDock(validated.as_str().to_owned()));
        }
        self.docks.push(DockState {
            id: validated,
            kind,
            placement,
            open: false,
        });
        Ok(())
    }

    /// Open a dock. Idempotent; a plain open never clears a stored focus
    /// token.
    ///
    /// # Errors
    ///
    /// Returns [`DockError::UnknownDock`] when the identifier is not
    /// registered.
    pub fn open(&mut self, id: &str) -> Result<(), DockError> {
        let Some(index) = self.index_of(id) else {
            return Err(DockError::UnknownDock(id.to_owned()));
        };
        self.docks[index].open = true;
        Ok(())
    }

    /// Open a dock and replace the stored focus token.
    ///
    /// # Errors
    ///
    /// Returns [`DockError::UnknownDock`] when the identifier is not
    /// registered.
    pub fn open_with_focus(&mut self, id: &str, token: String) -> Result<(), DockError> {
        let Some(index) = self.index_of(id) else {
            return Err(DockError::UnknownDock(id.to_owned()));
        };
        self.docks[index].open = true;
        self.focus_token = Some(token);
        Ok(())
    }

    /// Close a dock. Idempotent; keeps any stored focus token.
    ///
    /// # Errors
    ///
    /// Returns [`DockError::UnknownDock`] when the identifier is not
    /// registered.
    pub fn close(&mut self, id: &str) -> Result<(), DockError> {
        let Some(index) = self.index_of(id) else {
            return Err(DockError::UnknownDock(id.to_owned()));
        };
        self.docks[index].open = false;
        Ok(())
    }

    /// Toggle a dock between open and closed.
    ///
    /// # Errors
    ///
    /// Returns [`DockError::UnknownDock`] when the identifier is not
    /// registered.
    pub fn toggle(&mut self, id: &str) -> Result<(), DockError> {
        let Some(index) = self.index_of(id) else {
            return Err(DockError::UnknownDock(id.to_owned()));
        };
        self.docks[index].open = !self.docks[index].open;
        Ok(())
    }

    /// Whether a dock is currently open.
    ///
    /// # Errors
    ///
    /// Returns [`DockError::UnknownDock`] when the identifier is not
    /// registered.
    pub fn is_open(&self, id: &str) -> Result<bool, DockError> {
        let Some(index) = self.index_of(id) else {
            return Err(DockError::UnknownDock(id.to_owned()));
        };
        Ok(self.docks[index].open)
    }

    /// Current placement of a dock.
    ///
    /// # Errors
    ///
    /// Returns [`DockError::UnknownDock`] when the identifier is not
    /// registered.
    pub fn placement(&self, id: &str) -> Result<Placement, DockError> {
        let Some(index) = self.index_of(id) else {
            return Err(DockError::UnknownDock(id.to_owned()));
        };
        Ok(self.docks[index].placement)
    }

    /// Move a dock without changing whether it is open.
    ///
    /// # Errors
    ///
    /// Returns [`DockError::UnknownDock`] when the identifier is not
    /// registered.
    pub fn set_placement(&mut self, id: &str, placement: Placement) -> Result<(), DockError> {
        let Some(index) = self.index_of(id) else {
            return Err(DockError::UnknownDock(id.to_owned()));
        };
        self.docks[index].placement = placement;
        Ok(())
    }

    /// Currently open docks in registration order.
    #[must_use]
    pub fn open_docks(&self) -> Vec<&DockState> {
        self.docks.iter().filter(|dock| dock.open).collect()
    }

    /// Take and clear the stored focus token, if any.
    pub fn take_focus_token(&mut self) -> Option<String> {
        self.focus_token.take()
    }

    /// Close every dock. Keeps any stored focus token.
    pub fn close_all(&mut self) {
        for dock in &mut self.docks {
            dock.open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager_with(id: &str, kind: DockKind, placement: Placement) -> DockManager {
        let mut manager = DockManager::new();
        manager.register(id, kind, placement).unwrap();
        manager
    }

    #[test]
    fn id_rejects_empty() {
        assert_eq!(DockId::new("").unwrap_err(), DockError::EmptyId);
    }

    #[test]
    fn id_rejects_too_long() {
        let long: String = std::iter::repeat_n('a', 65).collect();
        assert_eq!(DockId::new(&long).unwrap_err(), DockError::IdTooLong);
    }

    #[test]
    fn id_accepts_exact_64_chars() {
        let exact: String = std::iter::repeat_n('a', 64).collect();
        let id = DockId::new(&exact).unwrap();
        assert_eq!(id.as_str(), exact);
    }

    #[test]
    fn id_accepts_valid_charset() {
        for valid in ["chat", "a-z_0-9", "abc-123_xyz", "0", "-", "_"] {
            let id = DockId::new(valid).unwrap();
            assert_eq!(id.as_str(), valid);
        }
    }

    #[test]
    fn id_rejects_invalid_characters() {
        for invalid in [
            "Chat",
            "CHAT",
            "has space",
            "with.dot",
            "with/slash",
            "UPPER",
        ] {
            assert!(
                matches!(DockId::new(invalid).unwrap_err(), DockError::InvalidChar(_)),
                "expected InvalidChar for {invalid:?}"
            );
        }
        assert_eq!(
            DockId::new("ok Bad").unwrap_err(),
            DockError::InvalidChar(' ')
        );
        assert_eq!(DockId::new("ABC").unwrap_err(), DockError::InvalidChar('A'));
        assert_eq!(DockId::new("a.b").unwrap_err(), DockError::InvalidChar('.'));
    }

    #[test]
    fn register_ok_and_duplicate() {
        let mut manager = DockManager::new();
        manager
            .register("chat", DockKind::Chat, Placement::Right)
            .unwrap();
        assert!(!manager.is_open("chat").unwrap());
        assert_eq!(
            manager.register("chat", DockKind::Chat, Placement::Right),
            Err(DockError::DuplicateDock("chat".into()))
        );
        // Invalid identifiers surface validation errors, not duplicates.
        assert_eq!(
            manager.register("", DockKind::Chat, Placement::Right),
            Err(DockError::EmptyId)
        );
    }

    #[test]
    fn open_close_toggle_idempotency_and_unknown() {
        let mut manager = manager_with("chat", DockKind::Chat, Placement::Right);

        // Open is idempotent.
        manager.open("chat").unwrap();
        assert!(manager.is_open("chat").unwrap());
        manager.open("chat").unwrap();
        assert!(manager.is_open("chat").unwrap());

        // Close is idempotent.
        manager.close("chat").unwrap();
        assert!(!manager.is_open("chat").unwrap());
        manager.close("chat").unwrap();
        assert!(!manager.is_open("chat").unwrap());

        // Toggle flips each time.
        manager.toggle("chat").unwrap();
        assert!(manager.is_open("chat").unwrap());
        manager.toggle("chat").unwrap();
        assert!(!manager.is_open("chat").unwrap());

        // Unknown identifiers report UnknownDock.
        for result in [
            manager.open("missing"),
            manager.close("missing"),
            manager.toggle("missing"),
            manager.open_with_focus("missing", "token".into()),
            manager.set_placement("missing", Placement::Left),
        ] {
            assert_eq!(result, Err(DockError::UnknownDock("missing".into())));
        }
        assert_eq!(
            manager.is_open("missing"),
            Err(DockError::UnknownDock("missing".into()))
        );
        assert_eq!(
            manager.placement("missing"),
            Err(DockError::UnknownDock("missing".into()))
        );
    }

    #[test]
    fn multi_open_docks_are_independent() {
        let mut manager = DockManager::new();
        manager
            .register("chat", DockKind::Chat, Placement::Right)
            .unwrap();
        manager
            .register("config", DockKind::Config, Placement::Left)
            .unwrap();

        manager.open("chat").unwrap();
        assert!(manager.is_open("chat").unwrap());
        assert!(!manager.is_open("config").unwrap());

        manager.open("config").unwrap();
        assert!(manager.is_open("chat").unwrap());
        assert!(manager.is_open("config").unwrap());

        manager.close("chat").unwrap();
        assert!(!manager.is_open("chat").unwrap());
        assert!(manager.is_open("config").unwrap());

        let open = manager.open_docks();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].id.as_str(), "config");
    }

    #[test]
    fn set_placement_updates_only_target() {
        let mut manager = DockManager::new();
        manager
            .register("chat", DockKind::Chat, Placement::Right)
            .unwrap();
        manager
            .register("config", DockKind::Config, Placement::Left)
            .unwrap();

        assert_eq!(manager.placement("chat").unwrap(), Placement::Right);
        manager.set_placement("chat", Placement::Bottom).unwrap();
        assert_eq!(manager.placement("chat").unwrap(), Placement::Bottom);
        // The other dock is untouched.
        assert_eq!(manager.placement("config").unwrap(), Placement::Left);

        assert_eq!(
            manager.set_placement("missing", Placement::Inline),
            Err(DockError::UnknownDock("missing".into()))
        );
    }

    #[test]
    fn focus_token_lifecycle() {
        let mut manager = manager_with("chat", DockKind::Chat, Placement::Right);

        // open_with_focus stores the token.
        manager.open_with_focus("chat", "first".into()).unwrap();
        assert!(manager.is_open("chat").unwrap());

        // Plain open preserves an existing token.
        manager.open("chat").unwrap();
        assert_eq!(manager.take_focus_token().as_deref(), Some("first"));
        // Take clears the token.
        assert_eq!(manager.take_focus_token(), None);

        // open_with_focus replaces the stored token.
        manager.open_with_focus("chat", "second".into()).unwrap();
        manager.open_with_focus("chat", "third".into()).unwrap();
        assert_eq!(manager.take_focus_token().as_deref(), Some("third"));

        // Close keeps the token for a later take.
        manager.open_with_focus("chat", "kept".into()).unwrap();
        manager.close("chat").unwrap();
        assert!(!manager.is_open("chat").unwrap());
        assert_eq!(manager.take_focus_token().as_deref(), Some("kept"));
        assert_eq!(manager.take_focus_token(), None);
    }

    #[test]
    fn close_all_closes_docks_but_keeps_token() {
        let mut manager = DockManager::new();
        manager
            .register("chat", DockKind::Chat, Placement::Right)
            .unwrap();
        manager
            .register("config", DockKind::Config, Placement::Left)
            .unwrap();
        manager.open_with_focus("chat", "token".into()).unwrap();
        manager.open("config").unwrap();
        assert_eq!(manager.open_docks().len(), 2);

        manager.close_all();
        assert!(!manager.is_open("chat").unwrap());
        assert!(!manager.is_open("config").unwrap());
        assert!(manager.open_docks().is_empty());
        assert_eq!(manager.take_focus_token().as_deref(), Some("token"));
    }

    #[test]
    fn serde_roundtrip_preserves_state_without_focus_token() {
        let mut manager = DockManager::new();
        manager
            .register("chat", DockKind::Chat, Placement::Bottom)
            .unwrap();
        manager
            .register("custom-1", DockKind::Custom, Placement::Inline)
            .unwrap();
        manager.open("chat").unwrap();
        manager
            .open_with_focus("custom-1", "transient".into())
            .unwrap();

        let json = serde_json::to_value(&manager).unwrap();
        // The transient token is never serialized.
        assert!(json.get("focus_token").is_none());

        let restored: DockManager = serde_json::from_value(json).unwrap();
        assert!(restored.is_open("chat").unwrap());
        assert!(restored.is_open("custom-1").unwrap());
        assert_eq!(restored.placement("chat").unwrap(), Placement::Bottom);
        assert_eq!(restored.placement("custom-1").unwrap(), Placement::Inline);
        // Deserialized managers start without a focus token.
        let mut restored = restored;
        assert_eq!(restored.take_focus_token(), None);
        assert_eq!(restored.open_docks().len(), 2);
    }

    #[test]
    fn open_docks_content_and_order() {
        let mut manager = DockManager::new();
        manager
            .register("one", DockKind::Chat, Placement::Left)
            .unwrap();
        manager
            .register("two", DockKind::Config, Placement::Right)
            .unwrap();
        manager
            .register("three", DockKind::Custom, Placement::Inline)
            .unwrap();
        assert!(manager.open_docks().is_empty());

        manager.open("two").unwrap();
        manager.open("one").unwrap();
        let open = manager.open_docks();
        // Registration order, not open order.
        let ids: Vec<&str> = open.iter().map(|dock| dock.id.as_str()).collect();
        assert_eq!(ids, vec!["one", "two"]);
        assert_eq!(open[0].kind, DockKind::Chat);
        assert_eq!(open[1].placement, Placement::Right);
    }

    #[test]
    fn error_messages_never_carry_focus_tokens() {
        let token = "focus-secret-token";
        let mut manager = manager_with("chat", DockKind::Chat, Placement::Right);
        manager.open_with_focus("chat", token.into()).unwrap();

        let errors = [
            DockError::EmptyId,
            DockError::IdTooLong,
            DockError::InvalidChar('X'),
            DockError::UnknownDock("missing".into()),
            DockError::DuplicateDock("chat".into()),
        ];
        for error in errors {
            let rendered = format!("{error} {error:?}");
            assert!(!rendered.contains("focus-secret-token"));
        }
    }
}
