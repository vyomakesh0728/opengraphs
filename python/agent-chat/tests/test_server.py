from __future__ import annotations

import asyncio
from types import SimpleNamespace

import pytest

from og_agent_chat.alerts import AlertDetector, ThresholdRule
from og_agent_chat.models import ActionPlan, Alert, ChatMessage, ExecutionResult, RunState
from og_agent_chat.server import (
    _clear_tfevents_files,
    _handle_payload,
    _normalize_runtime,
    _prepare_socket_path,
    _resolve_run_dir,
)


class StubAgentResponse:
    def __init__(self, plan: ActionPlan) -> None:
        self.plan = plan


class StubAgent:
    def __init__(self, auto_mode: bool = False) -> None:
        self.executor = SimpleNamespace(auto_mode=auto_mode)
        self._history = [ChatMessage(sender="user", content="hello", timestamp=1.0)]
        self.seen_alerts: list[Alert] = []
        self.executed_plans: list[ActionPlan] = []

    def get_chat_history(self) -> list[ChatMessage]:
        return list(self._history)

    async def handle_alert(self, alert: Alert) -> StubAgentResponse:
        self.seen_alerts.append(alert)
        return StubAgentResponse(
            ActionPlan(
                diagnosis="Investigated alert",
                action="explain",
                code_changes="",
                raw_output="raw-output",
            )
        )

    async def execute_plan(self, plan: ActionPlan) -> ExecutionResult:
        self.executed_plans.append(plan)
        return ExecutionResult(success=True, checkpoint_id="ckpt-1")


def test_server_filesystem_helpers_clean_up_expected_paths(tmp_path) -> None:
    socket_path = tmp_path / "sockdir" / "ogd.sock"
    socket_path.parent.mkdir(parents=True, exist_ok=True)
    socket_path.write_text("stale socket placeholder", encoding="utf-8")

    _prepare_socket_path(socket_path)

    assert socket_path.parent.is_dir()
    assert not socket_path.exists()

    run_dir = tmp_path / "runs"
    run_dir.mkdir()
    (run_dir / "events.tfevents.1").write_text("old", encoding="utf-8")
    (run_dir / "keep.txt").write_text("keep", encoding="utf-8")

    assert _clear_tfevents_files(run_dir) == 1
    assert not (run_dir / "events.tfevents.1").exists()
    assert (run_dir / "keep.txt").exists()
    assert _resolve_run_dir(run_dir / "events.tfevents.2") == run_dir
    assert _normalize_runtime(" MODAL ") == "modal"


def test_prepare_socket_path_rejects_existing_directories(tmp_path) -> None:
    socket_path = tmp_path / "occupied"
    socket_path.mkdir()

    with pytest.raises(RuntimeError, match="not a file"):
        _prepare_socket_path(socket_path)


def test_handle_payload_metrics_update_can_trigger_alert_callbacks(tmp_path) -> None:
    run_state = RunState(training_file=tmp_path / "train.py", codebase_root=tmp_path)
    agent = StubAgent()
    detector = AlertDetector(
        [
            ThresholdRule(
                metric="train/loss",
                threshold=0.5,
                comparison="gt",
                cooldown_secs=0.0,
            )
        ]
    )

    response = asyncio.run(
        _handle_payload(
            {"type": "metrics_update", "metric": "train/loss", "value": 0.9, "step": 4},
            run_state,
            agent,
            detector,
        )
    )

    assert response["ok"] is True
    assert response["alert"]["metric"] == "train/loss"
    assert response["agent_response"]["diagnosis"] == "Investigated alert"
    assert run_state.metrics["train/loss"] == [0.9]
    assert run_state.current_step == 4
    assert len(run_state.alerts) == 1
    assert len(agent.seen_alerts) == 1


def test_handle_payload_supports_runtime_controls_and_state_queries(tmp_path) -> None:
    run_state = RunState(training_file=tmp_path / "train.py", codebase_root=tmp_path)
    run_state.add_metric("runtime/health", 1.0, step=2)
    run_state.append_log("runtime started")
    run_state.add_alert(
        Alert(
            metric="runtime/health",
            threshold=0.0,
            current=1.0,
            message="healthy",
            timestamp=10.0,
        )
    )
    agent = StubAgent(auto_mode=False)
    detector = AlertDetector([])
    runtime_ref = {"kind": "local"}
    restarts: list[str] = []

    async def restart_training(_: RunState) -> None:
        restarts.append("called")

    state_response = asyncio.run(
        _handle_payload(
            {"type": "get_run_state", "metric_tail": 1, "log_tail": 1},
            run_state,
            agent,
            detector,
            runtime_ref=runtime_ref,
            runtime_env_overrides={"BATCH_SIZE": "4"},
        )
    )
    assert state_response["ok"] is True
    assert state_response["run_state"]["metrics"] == {"runtime/health": [1.0]}
    assert state_response["run_state"]["logs"] == ["runtime started"]
    assert state_response["run_state"]["runtime_env_overrides"] == {"BATCH_SIZE": "4"}

    auto_response = asyncio.run(
        _handle_payload(
            {"type": "set_auto_mode", "enabled": True},
            run_state,
            agent,
            detector,
        )
    )
    assert auto_response == {"ok": True, "auto_mode": True}
    assert agent.executor.auto_mode is True

    runtime_response = asyncio.run(
        _handle_payload(
            {"type": "set_runtime", "runtime": "modal"},
            run_state,
            agent,
            detector,
            runtime_ref=runtime_ref,
        )
    )
    assert runtime_response == {"ok": True, "runtime": "modal"}
    assert runtime_ref["kind"] == "modal"
    assert run_state.runtime == "modal"

    invalid_runtime = asyncio.run(
        _handle_payload(
            {"type": "set_runtime", "runtime": "remote"},
            run_state,
            agent,
            detector,
            runtime_ref=runtime_ref,
        )
    )
    assert invalid_runtime["ok"] is False
    assert "unsupported runtime" in invalid_runtime["error"]

    start_response = asyncio.run(
        _handle_payload(
            {"type": "start_training"},
            run_state,
            agent,
            detector,
            restart_training_callback=restart_training,
        )
    )
    assert start_response == {"ok": True}
    assert restarts == ["called"]

    apply_response = asyncio.run(
        _handle_payload(
            {
                "type": "apply_refactor",
                "diagnosis": "Need a fix",
                "action": "refactor",
                "code_changes": "--- a/train.py\n+++ b/train.py\n",
                "raw_output": "raw-output",
            },
            run_state,
            agent,
            detector,
        )
    )
    assert apply_response["ok"] is True
    assert apply_response["success"] is True
    assert apply_response["checkpoint_id"] == "ckpt-1"
    assert len(agent.executed_plans) == 1
