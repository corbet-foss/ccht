//! Python bindings delegate all event reduction to the published ccht Rust crate.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

pyo3::create_exception!(_native, ConversationError, PyValueError);

/// Native conversation state; Python applications own transport and authority.
#[pyclass(name = "ConversationModel", module = "ccht._native")]
struct ConversationModel {
    inner: ccht::Conversation,
}

#[pymethods]
impl ConversationModel {
    #[new]
    fn new(conversation_id: String) -> Self {
        Self {
            inner: ccht::Conversation::new(conversation_id),
        }
    }

    #[getter]
    fn id(&self) -> &str {
        self.inner.id()
    }

    fn apply_json(&mut self, event_json: &str) -> PyResult<bool> {
        self.inner
            .apply_json(event_json)
            .map_err(|error| ConversationError::new_err(error.to_string()))
    }

    fn snapshot_json(&self) -> PyResult<String> {
        self.inner
            .snapshot_json()
            .map_err(|error| ConversationError::new_err(error.to_string()))
    }
}

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<ConversationModel>()?;
    module.add(
        "ConversationError",
        module.py().get_type::<ConversationError>(),
    )?;
    module.add("WIRE_VERSION", ccht::WIRE_VERSION)?;
    module.add("MAX_EVENT_BYTES", ccht::MAX_EVENT_BYTES)?;
    Ok(())
}
