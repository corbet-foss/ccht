"""Conversation state shared with ccht's Rust and Wasm consumers."""

from collections.abc import Mapping
import json
from typing import Any

from ._native import (
    MAX_EVENT_BYTES as MAX_EVENT_BYTES,
    WIRE_VERSION as WIRE_VERSION,
    ConversationError as ConversationError,
    ConversationModel,
)

__all__ = ["Conversation", "ConversationError", "MAX_EVENT_BYTES", "WIRE_VERSION"]


class Conversation:
    """Reduce authorized ccht wire events using the shared Rust implementation.

    The application owns authentication, event delivery, storage, and UI.
    This model never launches an agent or makes network requests.
    """

    def __init__(self, conversation_id: str) -> None:
        self._model = ConversationModel(conversation_id)

    @property
    def id(self) -> str:
        """Application-owned conversation identity."""
        return self._model.id

    def apply_event(self, event: Mapping[str, Any] | str) -> bool:
        """Apply a JSON event or mapping; return False for an ignored replay.

        Invalid envelopes raise ConversationError, a ValueError subclass.
        Serialization failures raise TypeError or ValueError. Rejected events
        leave the conversation unchanged.
        """
        if not isinstance(event, str):
            event = json.dumps(dict(event), ensure_ascii=False, allow_nan=False)
        return self._model.apply_json(event)

    def snapshot_json(self) -> str:
        """Return the current render state as JSON."""
        return self._model.snapshot_json()

    def snapshot(self) -> dict[str, Any]:
        """Return a detached snapshot; editing it never changes the model."""
        return json.loads(self.snapshot_json())
