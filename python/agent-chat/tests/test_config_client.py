from __future__ import annotations

import socket

import pytest

import og_agent_chat.client as client_module
from og_agent_chat.client import OGDClientError, _recv_line, send_metric, send_request
from og_agent_chat.config import (
    _normalize_inference_provider,
    _resolve_provider_api_base,
    _resolve_provider_api_key,
    _sanitize_api_key,
)


def test_sanitize_api_key_normalizes_common_wrapper_formats() -> None:
    assert _sanitize_api_key(' "Bearer sk-test\n" ') == "sk-test"
    assert _sanitize_api_key("'plain-key'") == "plain-key"
    assert _sanitize_api_key("   ") is None
    assert _sanitize_api_key(None) is None


def test_provider_resolution_prefers_expected_environment_variables(
    monkeypatch,
) -> None:
    monkeypatch.setenv("OPENAI_API_BASE", "https://openai.example/v1")
    monkeypatch.setenv("OPENAI_API_KEY", "Bearer openai-key")
    monkeypatch.setenv("ANTHROPIC_API_KEY", "anthropic-key")

    assert _normalize_inference_provider("OPENAI") == "openai"
    assert _normalize_inference_provider("unsupported") == "auto"
    assert (
        _resolve_provider_api_base("openai/gpt-5.2-codex", None, "auto")
        == "https://openai.example/v1"
    )
    assert _resolve_provider_api_key("openai/gpt-5.2-codex", None, api_base=None, provider="auto") == "openai-key"
    assert _resolve_provider_api_key("anthropic/claude", None, api_base=None, provider="auto") == "anthropic-key"


def test_recv_line_stops_at_first_newline() -> None:
    left, right = socket.socketpair()
    try:
        right.sendall(b'{"ok": true}\ntrailing-bytes')
        assert _recv_line(left) == b'{"ok": true}'
    finally:
        left.close()
        right.close()


def test_send_request_requires_existing_socket(tmp_path) -> None:
    missing = tmp_path / "missing.sock"
    with pytest.raises(OGDClientError, match="Socket not found"):
        send_request({"type": "ping"}, missing)


def test_send_metric_builds_expected_payload(monkeypatch) -> None:
    captured: dict[str, object] = {}

    def fake_send_request(payload: dict[str, object], socket_path: str) -> dict[str, bool]:
        captured["payload"] = payload
        captured["socket_path"] = socket_path
        return {"ok": True}

    monkeypatch.setattr(client_module, "send_request", fake_send_request)

    response = send_metric("train/loss", 0.25, step=7, socket_path="/tmp/demo.sock")

    assert response == {"ok": True}
    assert captured == {
        "payload": {
            "type": "metrics_update",
            "metric": "train/loss",
            "value": 0.25,
            "step": 7,
        },
        "socket_path": "/tmp/demo.sock",
    }
