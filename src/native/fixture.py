"""Offline ACP peer for exercising the public native client through real stdio."""
import json
import os
import pathlib
import subprocess
import sys
import time

mode = sys.argv[1]
sessions = 0
configurations = {}
pending = {}
permissions = {}
permission_id = 100
if len(sys.argv) > 2 and sys.argv[2].endswith(".json"):
    child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"])
    pathlib.Path(sys.argv[2]).write_text(json.dumps({"agent": os.getpid(), "descendant": child.pid}))


def send(value):
    print(json.dumps({"jsonrpc": "2.0", **value}), flush=True)


def response(request_id, result):
    send({"id": request_id, "result": result})


def update(session, text):
    send({"method": "session/update", "params": {"sessionId": session,
          "update": {"sessionUpdate": "agent_message_chunk",
                     "content": {"type": "text", "text": text}}}})


# Fake OpenCode control server for driver tests. The Rust driver spawns:
# python3 -u -c <this file> <opencode_ok|opencode_fail|opencode_never>
#   serve --hostname 127.0.0.1 --port <port>
# Only loopback is used; no real network leaves the host.
if "serve" in sys.argv:
    if mode == "opencode_never":
        time.sleep(60)
        sys.exit(0)
    port = 0
    if "--port" in sys.argv:
        port = int(sys.argv[sys.argv.index("--port") + 1])
    from http.server import BaseHTTPRequestHandler, HTTPServer

    class DriverHandler(BaseHTTPRequestHandler):
        def log_message(self, format, *args):
            pass

        def _reply(self, code, body):
            data = body if isinstance(body, bytes) else body.encode()
            self.send_response(code)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self):
            path = self.path.split("?")[0]
            if path == "/global/health":
                self._reply(200, b"{}")
            elif path == "/provider":
                if mode == "opencode_fail":
                    self._reply(200, json.dumps({"connected": []}).encode())
                else:
                    self._reply(200, json.dumps({"connected": ["opencode-go"]}).encode())
            else:
                self._reply(404, b"{}")

        def do_PUT(self):
            path = self.path.split("?")[0]
            if path == "/auth/opencode-go":
                length = int(self.headers.get("Content-Length", "0") or "0")
                if length > 0:
                    self.rfile.read(min(length, 1_048_576))
                if mode == "opencode_fail":
                    self._reply(500, b"{}")
                else:
                    self._reply(200, b"{}")
            else:
                self._reply(404, b"{}")

    HTTPServer(("127.0.0.1", port), DriverHandler).serve_forever()
    sys.exit(0)


for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    params = message.get("params", {})
    request_id = message.get("id")
    if method == "initialize":
        if mode == "slow_init":
            time.sleep(60)
        response(request_id, {"protocolVersion": 1,
                 "agentCapabilities": {"loadSession": True,
                     "sessionCapabilities": {"resume": {}, "close": {}}},
                 "agentInfo": {"name": "offline-fixture", "version": "1"}})
    elif method in ("session/new", "session/load", "session/resume"):
        if mode == "auth_required":
            send({"id": request_id, "error": {"code": -32000,
                  "message": "SECRET-SHOULD-NOT-ESCAPE", "data": {"token": "SECRET"}}})
            continue
        sessions += 1
        session = params.get("sessionId", "fixture-" + str(sessions))
        if method == "session/load":
            update(session, "restored history")
        configurations[session] = [{
            "id": "model", "name": "Model", "category": "model", "type": "select",
            "currentValue": "test-model", "options": [{"value": "test-model", "name": "Test"},
                {"value": "second-model", "name": "Second"}]},
            {"id": "thinking", "name": "Thinking", "type": "boolean", "currentValue": False}]
        response(request_id, {"sessionId": session, "configOptions": configurations[session]})
    elif method == "session/set_config_option":
        configuration = configurations[params["sessionId"]]
        for option in configuration:
            if option["id"] == params["configId"]:
                option["currentValue"] = params["value"]
        if params["configId"] == "model":
            configuration[1]["currentValue"] = params["value"] == "second-model"
        response(request_id, {"configOptions": configuration})
    elif method == "session/prompt":
        session = params["sessionId"]
        pending[session] = request_id
        if mode == "exit":
            sys.exit(2)
        if mode in ("ask", "deny", "unsupported_host_request"):
            permission_id += 1
            permissions[permission_id] = session
            if mode == "unsupported_host_request":
                send({"id": permission_id, "method": "fs/read_text_file",
                      "params": {"sessionId": session, "path": "/never-read-this"}})
            else:
                send({"id": permission_id, "method": "session/request_permission", "params": {
                    "sessionId": session, "toolCall": {"toolCallId": "tool-1", "title": "Test action"},
                    "options": [{"optionId": "allow", "name": "Allow once", "kind": "allow_once"},
                                {"optionId": "reject", "name": "Reject once", "kind": "reject_once"}]}})
        elif mode in ("hang", "ignore_cancel"):
            update(session, "waiting")
        else:
            if mode == "startup_cwd":
                update(session, os.getcwd())
            elif mode == "burst":
                for _ in range(1000):
                    update(session, "x")
            else:
                update(session, "hello ")
                update(session, "world")
            response(pending.pop(session), {"stopReason": "end_turn"})
    elif method == "session/cancel":
        session = params["sessionId"]
        if mode != "ignore_cancel" and session in pending:
            response(pending.pop(session), {"stopReason": "cancelled"})
    elif method == "session/close":
        response(request_id, {})
    elif method == "account/login/start":
        if mode == "codex_bad_url":
            response(request_id, {"loginId": "login-1",
                      "verificationUrl": "http://evil.example.com/x",
                      "userCode": "ABCD-1234"})
        elif mode == "codex_bad_code":
            response(request_id, {"loginId": "login-1",
                      "verificationUrl": "https://auth.openai.com/codex/device",
                      "userCode": "!!!"})
        else:
            response(request_id, {"loginId": "login-1",
                      "verificationUrl": "https://auth.openai.com/codex/device",
                      "userCode": "ABCD-1234"})
            if mode == "codex_ok":
                send({"method": "account/login/completed",
                      "params": {"loginId": "login-1", "success": True}})
            elif mode == "codex_declined":
                send({"method": "account/login/completed",
                      "params": {"loginId": "login-1", "success": False}})
    elif method == "account/read":
        response(request_id, {"account": {"type": "chatgpt",
                  "email": "user@example.com"}})
    elif method is None and request_id in permissions:
        session = permissions.pop(request_id)
        if mode == "unsupported_host_request":
            assert message["error"]["code"] == -32601
            text = "unsupported"
        else:
            outcome = message["result"]["outcome"]
            text = outcome.get("optionId", outcome["outcome"])
        update(session, text)
        response(pending.pop(session), {"stopReason": "end_turn"})
    elif request_id is not None:
        send({"id": request_id, "error": {"code": -32601, "message": "Unsupported fixture request"}})
