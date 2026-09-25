#!/usr/bin/env python3
"""Disposable ACP v1 peer for the live Podman containment gate."""

import json
import os
import socket
import sys
from pathlib import Path


WORKSPACE = Path("/workspace")
SESSION_ID = "swem-live-container-fixture"


def send(message):
    sys.stdout.write(json.dumps(message, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def observe(prompt):
    challenge = (WORKSPACE / "host-challenge.txt").read_text(encoding="utf-8")
    process_status = dict(
        line.split(":", 1)
        for line in Path("/proc/self/status").read_text(encoding="utf-8").splitlines()
        if ":" in line
    )
    try:
        with socket.create_connection(("1.1.1.1", 53), timeout=0.25):
            network = "unexpectedly_reachable"
    except OSError as error:
        network = f"denied:{type(error).__name__}"
    result = {
        "uid": os.getuid(),
        "gid": os.getgid(),
        "cwd": os.getcwd(),
        "environment_names": sorted(os.environ),
        "host_challenge": challenge,
        "prompt": prompt,
        "network": network,
        "interfaces": sorted(path.name for path in Path("/sys/class/net").iterdir()),
        "cap_eff": process_status["CapEff"].strip(),
        "cap_bnd": process_status["CapBnd"].strip(),
    }
    (WORKSPACE / "runtime-observation.json").write_text(
        json.dumps(result, indent=2, sort_keys=True), encoding="utf-8"
    )
    (WORKSPACE / "container-write.txt").write_text(
        "container-observed:" + challenge, encoding="utf-8"
    )
    return result


for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    request_id = request.get("id")
    params = request.get("params") or {}
    if method == "initialize":
        send(
            {
                "jsonrpc": "2.0",
                "id": request_id,
                "result": {
                    "protocolVersion": params["protocolVersion"],
                    "agentCapabilities": {},
                    "authMethods": [],
                    "agentInfo": {"name": "swem-live-fixture", "version": "0.1"},
                },
            }
        )
    elif method == "session/new":
        send(
            {
                "jsonrpc": "2.0",
                "id": request_id,
                "result": {"sessionId": SESSION_ID},
            }
        )
    elif method == "session/prompt":
        prompt = next(
            (
                block.get("text", "")
                for block in params.get("prompt", [])
                if block.get("type") == "text"
            ),
            "",
        )
        result = observe(prompt)
        send(
            {
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {
                    "sessionId": params["sessionId"],
                    "update": {
                        "sessionUpdate": "agent_message_chunk",
                        "content": {
                            "type": "text",
                            "text": json.dumps(result, sort_keys=True),
                        },
                    },
                },
            }
        )
        send(
            {
                "jsonrpc": "2.0",
                "id": request_id,
                "result": {"stopReason": "end_turn"},
            }
        )
    elif request_id is not None:
        send(
            {
                "jsonrpc": "2.0",
                "id": request_id,
                "error": {"code": -32601, "message": f"unsupported method {method}"},
            }
        )
