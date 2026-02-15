from __future__ import annotations

import json
import os
import socket
from pathlib import Path
from typing import Any

def _default_socket_path() -> str:
    tmpdir = os.getenv("TMPDIR") or os.getenv("TEMP") or os.getenv("TMP") or "/tmp"
    return str(Path(tmpdir) / "opengraphs-ogd.sock")


DEFAULT_SOCKET = os.getenv("OGD_SOCKET", _default_socket_path())


class OGDClientError(RuntimeError):
    pass


def _recv_line(sock: socket.socket) -> bytes:
    buffer = bytearray()
    while True:
        chunk = sock.recv(4096)
        if not chunk:
            break
        buffer.extend(chunk)
        if b"\n" in chunk:
            break
    if b"\n" in buffer:
        line, _ = buffer.split(b"\n", 1)
        return bytes(line)
    return bytes(buffer)


def send_request(payload: dict[str, Any], socket_path: str | Path = DEFAULT_SOCKET) -> dict[str, Any]:
    path = Path(socket_path)
    if not path.exists():
        raise OGDClientError(f"Socket not found: {path}")

    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
        sock.connect(str(path))
        sock.sendall((json.dumps(payload) + "\n").encode("utf-8"))
        data = _recv_line(sock)

    if not data:
        raise OGDClientError("No response from daemon")
    try:
        return json.loads(data.decode("utf-8"))
    except json.JSONDecodeError as exc:  # pragma: no cover
        raise OGDClientError("Invalid response from daemon") from exc


def ping(socket_path: str | Path = DEFAULT_SOCKET) -> dict[str, Any]:
    return send_request({"type": "ping"}, socket_path)


def get_chat_history(socket_path: str | Path = DEFAULT_SOCKET) -> dict[str, Any]:
    return send_request({"type": "get_chat_history"}, socket_path)


def send_chat_message(content: str, socket_path: str | Path = DEFAULT_SOCKET) -> dict[str, Any]:
    return send_request({"type": "chat_message", "content": content}, socket_path)


def send_metric(
    metric: str,
    value: float,
    *,
    step: int | None = None,
    socket_path: str | Path = DEFAULT_SOCKET,
) -> dict[str, Any]:
    payload: dict[str, Any] = {"type": "metrics_update", "metric": metric, "value": value}
    if step is not None:
        payload["step"] = step
    return send_request(payload, socket_path)


def append_log(line: str, socket_path: str | Path = DEFAULT_SOCKET) -> dict[str, Any]:
    return send_request({"type": "log_append", "line": line}, socket_path)


def set_training_file(path: str | Path, socket_path: str | Path = DEFAULT_SOCKET) -> dict[str, Any]:
    return send_request({"type": "set_training_file", "path": str(path)}, socket_path)
