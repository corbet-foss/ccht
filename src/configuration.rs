//! Agent-owned session controls shared by native, Wasm and transport consumers.

use serde::{Deserialize, Serialize};

use crate::acp::{
    SessionConfigKind, SessionConfigOption, SessionConfigOptionCategory, SessionConfigOptionValue,
    SessionConfigSelectOptions, SessionModeState, SessionUpdate,
};

/// An agent's current advertised configuration; option order is significant.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionConfiguration {
    /// Model, mode, reasoning and other controls supplied by the agent.
    pub options: Vec<SessionConfigOption>,
    /// Legacy modes for agents that have not adopted configuration options.
    pub modes: Option<SessionModeState>,
}

impl SessionConfiguration {
    /// The first model selector, following the agent's priority ordering.
    pub fn model_option(&self) -> Option<&SessionConfigOption> {
        self.options.iter().find(|option| {
            option.category == Some(SessionConfigOptionCategory::Model)
                || option.id.0.as_ref() == "model"
        })
    }

    /// Human-readable current model: the advertised option label when the
    /// agent names the current value, else the raw value id, else nothing.
    /// Pure display helper; selection stays with [`Self::accepts`] and the
    /// session, so applications never parse labels back into values.
    pub fn model_display_name(&self) -> Option<String> {
        let option = self.model_option()?;
        let SessionConfigKind::Select(select) = &option.kind else {
            return None;
        };
        let current = select.current_value.to_string();
        let label = match &select.options {
            SessionConfigSelectOptions::Ungrouped(options) => options
                .iter()
                .find(|option| option.value.to_string() == current)
                .map(|option| option.name.clone()),
            SessionConfigSelectOptions::Grouped(groups) => groups
                .iter()
                .flat_map(|group| &group.options)
                .find(|option| option.value.to_string() == current)
                .map(|option| option.name.clone()),
            _ => None,
        };
        Some(label.unwrap_or(current))
    }

    /// Whether an exact value is offered, including grouped selectors and booleans.
    pub fn accepts(&self, id: &str, value: &SessionConfigOptionValue) -> bool {
        let Some(option) = self
            .options
            .iter()
            .find(|option| option.id.0.as_ref() == id)
        else {
            return false;
        };
        match (&option.kind, value) {
            (SessionConfigKind::Boolean(_), SessionConfigOptionValue::Boolean { .. }) => true,
            (SessionConfigKind::Select(select), SessionConfigOptionValue::ValueId { value }) => {
                match &select.options {
                    SessionConfigSelectOptions::Ungrouped(options) => {
                        options.iter().any(|option| option.value == *value)
                    }
                    SessionConfigSelectOptions::Grouped(groups) => groups
                        .iter()
                        .any(|group| group.options.iter().any(|option| option.value == *value)),
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// Apply an agent update. Configuration updates replace the complete option list.
    pub fn apply(&mut self, update: &SessionUpdate) {
        match update {
            SessionUpdate::ConfigOptionUpdate(update) => {
                self.options.clone_from(&update.config_options);
            }
            SessionUpdate::CurrentModeUpdate(update) => {
                if let Some(modes) = &mut self.modes {
                    modes.current_mode_id.clone_from(&update.current_mode_id);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grouped_selectors_and_boolean_controls_preserve_exact_types() {
        let settings: SessionConfiguration = serde_json::from_value(serde_json::json!({
            "options": [{"id":"model","name":"Model","category":"model","type":"select",
                "currentValue":"one","options":[{"group":"provider","name":"Provider","options":[
                    {"value":"one","name":"One"},{"value":"two","name":"Two"}]}]},
                {"id":"thinking","name":"Thinking","type":"boolean","currentValue":false}],
            "modes":null
        }))
        .unwrap();
        assert!(settings.accepts("model", &"two".into()));
        assert!(!settings.accepts("model", &true.into()));
        assert!(settings.accepts("thinking", &true.into()));
        assert!(!settings.accepts("thinking", &"true".into()));
        assert!(!settings.accepts("missing", &true.into()));
    }

    #[test]
    fn model_option_is_none_without_advertised_model() {
        assert!(SessionConfiguration::default().model_option().is_none());
        assert!(
            SessionConfiguration::default()
                .model_display_name()
                .is_none()
        );
    }

    #[test]
    fn model_display_name_prefers_label_over_value() {
        let settings: SessionConfiguration = serde_json::from_value(serde_json::json!({
            "options": [{"id":"model","name":"Model","category":"model","type":"select",
                "currentValue":"muse-spark","options":[{"value":"muse-spark","name":"Muse Spark 1.3"}]}],
            "modes":null
        }))
        .unwrap();
        assert_eq!(
            settings.model_display_name(),
            Some("Muse Spark 1.3".to_string())
        );
    }

    #[test]
    fn model_display_name_falls_back_to_raw_value() {
        let settings: SessionConfiguration = serde_json::from_value(serde_json::json!({
            "options": [{"id":"model","name":"Model","category":"model","type":"select",
                "currentValue":"mystery-9",
                "options":[{"value":"other-1","name":"Other"}]}],
            "modes":null
        }))
        .unwrap();
        assert_eq!(settings.model_display_name(), Some("mystery-9".to_string()));
    }

    #[test]
    fn model_display_name_is_none_without_select_model() {
        let settings: SessionConfiguration = serde_json::from_value(serde_json::json!({
            "options": [{"id":"thinking","name":"Thinking","type":"boolean","currentValue":false}],
            "modes":null
        }))
        .unwrap();
        assert_eq!(settings.model_display_name(), None);
    }

    #[test]
    fn apply_replaces_options_and_tracks_current_mode() {
        use crate::acp::{
            ConfigOptionUpdate, ContentChunk, CurrentModeUpdate, SessionModeId, SessionModeState,
        };
        let mut settings = SessionConfiguration::default();
        // Other updates leave the configuration unchanged.
        settings.apply(&SessionUpdate::AgentMessageChunk(ContentChunk::new(
            "hi".into(),
        )));
        assert!(settings.options.is_empty());

        let options: SessionConfiguration = serde_json::from_value(serde_json::json!({
            "options": [{"id":"model","name":"Model","type":"select",
                "currentValue":"one","options":[{"value":"one","name":"One"}]}],
            "modes": null
        }))
        .unwrap();
        settings.apply(&SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(
            options.options.clone(),
        )));
        assert_eq!(settings.options, options.options);
        assert!(settings.model_option().is_some());

        // Without legacy modes, a mode update is a no-op.
        settings.apply(&SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new(
            SessionModeId::new("second"),
        )));
        assert!(settings.modes.is_none());

        settings.modes = Some(SessionModeState::new(
            SessionModeId::new("first"),
            Vec::new(),
        ));
        settings.apply(&SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new(
            SessionModeId::new("second"),
        )));
        assert_eq!(
            settings
                .modes
                .as_ref()
                .map(|modes| modes.current_mode_id.0.as_ref()),
            Some("second")
        );
    }
}
