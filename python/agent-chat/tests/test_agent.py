from __future__ import annotations

import asyncio
import difflib
import json

import pytest

import og_agent_chat.agent as agent_module
from og_agent_chat.agent import (
    ActionPlanner,
    AgentEngine,
    GuardedExecutor,
    _normalize_diff_text,
    _patch_targets_file,
    apply_diff,
    create_checkpoint,
    restore_checkpoint,
)
from og_agent_chat.models import ActionPlan, Alert, RunState


def _make_run_state(tmp_path, filename: str = "train.py", content: str = "value = 1\n") -> RunState:
    training_file = tmp_path / "workspace" / filename
    training_file.parent.mkdir(parents=True, exist_ok=True)
    training_file.write_text(content, encoding="utf-8")
    return RunState(training_file=training_file, codebase_root=tmp_path / "workspace")


def _make_diff(before: str, after: str, fromfile: str, tofile: str) -> str:
    return "".join(
        difflib.unified_diff(
            before.splitlines(keepends=True),
            after.splitlines(keepends=True),
            fromfile=fromfile,
            tofile=tofile,
        )
    ).rstrip("\n")


def test_action_planner_parses_refactor_response_sections() -> None:
    planner = ActionPlanner()
    raw = (
        "DIAGNOSIS: Learning rate is too high.\n"
        "ACTION: refactor\n"
        "CODE_CHANGES:\n"
        "--- a/train.py\n"
        "+++ b/train.py\n"
        "@@ -1 +1 @@\n"
        "-LR = 0.1\n"
        "+LR = 0.001\n"
    )

    plan = planner.parse_response(raw)

    assert plan.diagnosis == "Learning rate is too high."
    assert plan.action == "refactor"
    assert plan.code_changes.startswith("--- a/train.py")
    assert plan.raw_output == raw


def test_action_planner_falls_back_to_explain_and_drops_code_changes() -> None:
    planner = ActionPlanner()

    explain_plan = planner.parse_response(
        "DIAGNOSIS: Check the logs first.\n"
        "ACTION: explain\n"
        "CODE_CHANGES:\n"
        "--- a/train.py\n"
        "+++ b/train.py\n"
    )
    fallback_plan = planner.parse_response("Unstructured answer")

    assert explain_plan.action == "explain"
    assert explain_plan.code_changes == ""
    assert fallback_plan.diagnosis == "Unstructured answer"
    assert fallback_plan.action == "explain"


def test_normalize_diff_text_discards_fences_and_preamble() -> None:
    normalized = _normalize_diff_text(
        "Here is the patch you asked for:\n"
        "```diff\n"
        "--- a/train.py\n"
        "+++ b/train.py\n"
        "@@ -1 +1 @@\n"
        "-value = 1\n"
        "+value = 2\n"
        "```\n"
    )

    assert normalized.startswith("--- a/train.py\n+++ b/train.py\n")
    assert "```" not in normalized
    assert "Here is the patch" not in normalized
    assert normalized.endswith("\n")


@pytest.mark.parametrize(
    "patch_path",
    [
        "train.py",
        "./nested/train.py",
        "a/nested/train.py",
        "b/nested/train.py\t2026-03-01",
    ],
)
def test_patch_targets_file_matches_common_patch_path_formats(tmp_path, patch_path: str) -> None:
    filepath = tmp_path / "nested" / "train.py"
    filepath.parent.mkdir(parents=True, exist_ok=True)
    filepath.write_text("value = 1\n", encoding="utf-8")

    assert _patch_targets_file(filepath, patch_path) is True
    assert _patch_targets_file(filepath, "other.py") is False


def test_apply_diff_updates_file_when_patch_targets_relative_path(tmp_path) -> None:
    run_state = _make_run_state(tmp_path, filename="nested/train.py", content="value = 1\n")
    diff_text = _make_diff(
        "value = 1\n",
        "value = 2\n",
        "a/nested/train.py",
        "b/nested/train.py",
    )

    apply_diff(run_state.training_file, diff_text)

    assert run_state.training_file.read_text(encoding="utf-8") == "value = 2\n"


def test_create_and_restore_checkpoint_round_trip(tmp_path, monkeypatch) -> None:
    run_state = _make_run_state(tmp_path, content="value = 1\n")
    run_state.add_metric("loss", 0.9, step=3)
    run_state.add_metric("loss", 0.7, step=4)
    checkpoint_dir = tmp_path / "checkpoints"
    monkeypatch.setattr(agent_module.time, "time", lambda: 1234.56)

    checkpoint_id = create_checkpoint(run_state, checkpoint_dir)
    checkpoint_path = checkpoint_dir / checkpoint_id

    run_state.training_file.write_text("value = 99\n", encoding="utf-8")
    restore_checkpoint(checkpoint_id, run_state, checkpoint_dir)

    assert checkpoint_id == "ckpt_1234"
    assert run_state.training_file.read_text(encoding="utf-8") == "value = 1\n"
    assert json.loads((checkpoint_path / "state.json").read_text(encoding="utf-8")) == {
        "metrics": {"loss": [0.9, 0.7]},
        "step": 4,
    }


def test_fast_demo_refactor_plan_requires_alert_or_refactor_intent(tmp_path) -> None:
    run_state = _make_run_state(
        tmp_path,
        filename="demo_train.py",
        content='LEARNING_RATE = float(os.getenv("DEMO_LR", "0.01"))\n',
    )
    engine = AgentEngine(run_state=run_state, codebase_root=tmp_path)

    plan = engine._fast_demo_refactor_plan("How are the metrics looking?", alert=None)

    assert plan is None


def test_fast_demo_refactor_plan_builds_stabilizing_patch_for_demo_file(tmp_path) -> None:
    run_state = _make_run_state(
        tmp_path,
        filename="demo_train.py",
        content=(
            'LEARNING_RATE = float(os.getenv("DEMO_LR", "0.01"))\n'
            "WARMUP_STEPS = 5\n"
            "PEAK_LR_MULT = 3.0\n"
            'BATCH_SIZE = int(os.getenv("DEMO_BATCH", "8"))\n'
        ),
    )
    engine = AgentEngine(run_state=run_state, codebase_root=tmp_path)

    plan = engine._fast_demo_refactor_plan(
        "Please stabilize the demo training defaults.",
        alert=None,
    )

    assert plan is not None
    assert plan.action == "refactor"
    assert 'LEARNING_RATE = float(os.getenv("DEMO_LR", "0.001"))' in plan.code_changes
    assert "WARMUP_STEPS = 20" in plan.code_changes
    assert "PEAK_LR_MULT = 1.0" in plan.code_changes
    assert 'BATCH_SIZE = int(os.getenv("DEMO_BATCH", "32"))' in plan.code_changes


def test_summarize_diff_changes_reports_assignment_updates() -> None:
    summary = AgentEngine._summarize_diff_changes(
        "--- a/train.py\n"
        "+++ b/train.py\n"
        "@@ -1,3 +1,3 @@\n"
        "-ALPHA = 1\n"
        "+ALPHA = 2\n"
        "-BETA = 3\n"
        "+GAMMA = 4\n"
    )

    assert summary == "Refactor summary: ALPHA: 1 -> 2 | BETA: removed | GAMMA: set to 4"


def test_guarded_executor_returns_checkpoint_for_explain_plan(tmp_path, monkeypatch) -> None:
    run_state = _make_run_state(tmp_path, content="value = 1\n")
    monkeypatch.setattr(agent_module.time, "time", lambda: 2000.0)
    executor = GuardedExecutor(auto_mode=False, checkpoint_dir=tmp_path / "ckpts")

    result = asyncio.run(
        executor.execute(
            ActionPlan(
                diagnosis="No code change needed",
                action="explain",
                code_changes="",
                raw_output="raw",
            ),
            run_state,
        )
    )

    assert result.success is True
    assert result.checkpoint_id == "ckpt_2000"
    assert run_state.training_file.read_text(encoding="utf-8") == "value = 1\n"


def test_guarded_executor_applies_refactor_and_awaits_restart_callback(tmp_path) -> None:
    run_state = _make_run_state(tmp_path, content="value = 1\n")
    restarted: list[str] = []
    diff_text = _make_diff("value = 1\n", "value = 2\n", "a/train.py", "b/train.py")

    async def restart_callback(state: RunState) -> None:
        restarted.append(state.training_file.read_text(encoding="utf-8"))

    executor = GuardedExecutor(
        auto_mode=True,
        checkpoint_dir=tmp_path / "ckpts",
        restart_callback=restart_callback,
    )

    result = asyncio.run(
        executor.execute(
            ActionPlan(
                diagnosis="Apply patch",
                action="refactor",
                code_changes=diff_text,
                raw_output="raw",
            ),
            run_state,
        )
    )

    assert result.success is True
    assert run_state.training_file.read_text(encoding="utf-8") == "value = 2\n"
    assert restarted == ["value = 2\n"]


def test_guarded_executor_restores_checkpoint_when_apply_fails(tmp_path, monkeypatch) -> None:
    run_state = _make_run_state(tmp_path, content="value = 1\n")
    monkeypatch.setattr(agent_module.time, "time", lambda: 3000.0)

    def broken_apply_diff(filepath, diff_text: str) -> None:
        filepath.write_text("partially changed\n", encoding="utf-8")
        raise RuntimeError("boom")

    monkeypatch.setattr(agent_module, "apply_diff", broken_apply_diff)
    executor = GuardedExecutor(auto_mode=True, checkpoint_dir=tmp_path / "ckpts")

    result = asyncio.run(
        executor.execute(
            ActionPlan(
                diagnosis="Apply patch",
                action="refactor",
                code_changes="--- a/train.py\n+++ b/train.py\n",
                raw_output="raw",
            ),
            run_state,
        )
    )

    assert result.success is False
    assert result.checkpoint_id == "ckpt_3000"
    assert result.error == "boom"
    assert run_state.training_file.read_text(encoding="utf-8") == "value = 1\n"
