"""Exercise the installed Python package outside its producing checkout."""

import json
from pathlib import Path
import unittest

import ccht
from ccht import Conversation, ConversationError


def event(sequence, payload, conversation="creator", request="turn-1"):
    return {
        "version": 1,
        "conversation_id": conversation,
        "request_id": request,
        "sequence": sequence,
        "event": payload,
    }


def text(value, role="agent_message_chunk"):
    return {"kind": "update", "update": {
        "sessionUpdate": role,
        "content": {"type": "text", "text": value},
    }}


class InstalledConversationTests(unittest.TestCase):
    def test_shared_reduction_replay_and_independent_roles(self):
        creator, critic = Conversation("creator"), Conversation("critic")
        first = event(1, text("Hello"))
        self.assertTrue(creator.apply_event(first))
        self.assertFalse(creator.apply_event(first))
        with self.assertRaises(ConversationError):
            critic.apply_event(first)
        self.assertEqual(critic.snapshot()["turns"], [])
        self.assertTrue(creator.apply_event(json.dumps(event(2, text(" world 👋")))))
        self.assertEqual(creator.id, "creator")
        self.assertEqual(creator.snapshot()["turns"][0]["text"], "Hello world 👋")
        detached = creator.snapshot()
        detached["turns"].clear()
        self.assertEqual(len(creator.snapshot()["turns"]), 1)

    def test_native_model_controls_survive_shared_reduction(self):
        model = Conversation("creator")
        options = [{"id": "model", "name": "Model", "category": "model", "type": "select",
                    "currentValue": "fixture/muse", "options": [{"value": "fixture/muse", "name": "Muse"}]},
                   {"id": "thinking", "name": "Thinking", "type": "boolean", "currentValue": True}]
        model.apply_event(event(1, {"kind": "update", "update": {
            "sessionUpdate": "config_option_update", "configOptions": options,
        }}))
        self.assertEqual(model.snapshot()["configuration"]["options"], options)
        self.assertEqual(model.snapshot()["turns"][0]["text"], "")

    def test_rejected_events_do_not_mutate(self):
        model = Conversation("creator")
        model.apply_event(event(1, text("First")))
        snapshot = model.snapshot_json()
        for invalid in ["{", event(3, text("gap")), event(2, text("wrong"), "critic"),
                        {**event(2, text("version")), "version": 999},
                        event(2, text("x" * ccht.MAX_EVENT_BYTES))]:
            with self.assertRaises(ConversationError):
                model.apply_event(invalid)
            self.assertEqual(model.snapshot_json(), snapshot)
        with self.assertRaises(ValueError):
            model.apply_event({"non_finite": float("nan")})
        self.assertEqual(model.snapshot_json(), snapshot)

    def test_tools_permissions_and_terminal_cancellation(self):
        model = Conversation("creator")
        model.apply_event(event(1, {"kind": "update", "update": {
            "sessionUpdate": "tool_call", "toolCallId": "tool-1",
            "title": "Read document", "status": "pending",
        }}))
        model.apply_event(event(2, {"kind": "permission", "request_id": "permission-1", "request": {
            "sessionId": "agent-session", "toolCall": {"toolCallId": "tool-1"},
            "options": [{"optionId": "deny", "name": "Reject", "kind": "reject_once"}],
        }}))
        self.assertEqual(model.snapshot()["turns"][0]["permissions"][0]["request_id"], "permission-1")
        model.apply_event(event(3, {"kind": "update", "update": {
            "sessionUpdate": "tool_call_update", "toolCallId": "tool-1", "status": "completed",
        }}))
        turn = model.snapshot()["turns"][0]
        self.assertEqual(turn["tools"]["tool-1"]["status"], "completed")
        self.assertEqual(turn["permissions"], [])
        model.apply_event(event(4, {"kind": "completed", "stop_reason": "cancelled"}))
        self.assertEqual(model.snapshot()["turns"][0]["status"], "cancelled")
        with self.assertRaises(ConversationError):
            model.apply_event(event(5, text("late")))

    def test_non_text_content_retains_its_role(self):
        model = Conversation("creator")
        image = {"type": "image", "data": "AQID", "mimeType": "image/png"}
        for sequence, role in enumerate(["user_message_chunk", "agent_thought_chunk", "agent_message_chunk"], 1):
            model.apply_event(event(sequence, {"kind": "update", "update": {
                "sessionUpdate": role, "content": image,
            }}))
        turn = model.snapshot()["turns"][0]
        for field in ["user_content", "thought_content", "content"]:
            self.assertEqual(turn[field], [image])

    def test_terminal_failure_and_new_request(self):
        model = Conversation("creator")
        model.apply_event(event(1, {"kind": "error", "code": "unavailable", "message": "Agent unavailable"}))
        self.assertEqual(model.snapshot()["turns"][0]["status"], "failed")
        model.apply_event(event(1, text("Next"), request="turn-2"))
        self.assertEqual(len(model.snapshot()["turns"]), 2)


if __name__ == "__main__":
    assert "site-packages" in str(Path(ccht.__file__).resolve()), ccht.__file__
    assert ccht.WIRE_VERSION == 1
    assert issubclass(ConversationError, ValueError)
    unittest.main(verbosity=2)
