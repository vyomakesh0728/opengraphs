from __future__ import annotations

from og_agent_chat.models import ActionPlan, Alert, RunState


def test_run_state_helpers_track_latest_values_and_tails(tmp_path) -> None:
    state = RunState(training_file=tmp_path / "train.py", codebase_root=tmp_path)

    assert state.latest_alert() is None
    assert state.metric_tail("loss") == []
    assert state.log_tail() == ""

    state.add_metric("loss", 1.5, step=2)
    state.add_metric("loss", 0.75, step=1)
    state.add_metric("loss", 0.5, step="bad-step")
    state.append_log("line 1")
    state.append_log("line 2")

    first = Alert(
        metric="loss",
        threshold=1.0,
        current=1.5,
        message="loss is high",
        timestamp=10.0,
    )
    latest = Alert(
        metric="loss",
        threshold=0.8,
        current=0.75,
        message="loss is improving",
        timestamp=20.0,
    )
    state.add_alert(first)
    state.add_alert(latest)

    assert state.metrics == {"loss": [1.5, 0.75, 0.5]}
    assert state.metric_tail("loss", n=2) == [0.75, 0.5]
    assert state.current_step == 2
    assert state.log_tail(1) == "line 2"
    assert state.latest_alert() == latest


def test_action_plan_is_refactor_matches_action() -> None:
    explain = ActionPlan(
        diagnosis="Need checks",
        action="explain",
        code_changes="",
        raw_output="raw explain",
    )
    refactor = ActionPlan(
        diagnosis="Need patch",
        action="refactor",
        code_changes="--- a/train.py\n+++ b/train.py\n",
        raw_output="raw refactor",
    )

    assert explain.is_refactor() is False
    assert refactor.is_refactor() is True
