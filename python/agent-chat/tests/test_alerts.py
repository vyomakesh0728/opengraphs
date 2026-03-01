from __future__ import annotations

import json

import og_agent_chat.alerts as alerts_module
from og_agent_chat.alerts import (
    AlertDetector,
    StallRule,
    ThresholdRule,
    default_alert_rules,
    load_alert_rules_from_env,
)
from og_agent_chat.models import RunState


def test_threshold_rule_and_detector_respect_metric_filter_and_cooldown(
    monkeypatch,
    tmp_path,
) -> None:
    state = RunState(training_file=tmp_path / "train.py", codebase_root=tmp_path)
    state.add_metric("train/loss", 0.9, step=3)
    detector = AlertDetector(
        [
            ThresholdRule(
                metric="train/loss",
                threshold=0.5,
                comparison="gt",
                cooldown_secs=60.0,
            )
        ]
    )
    clock = {"now": 100.0}
    monkeypatch.setattr(alerts_module.time, "time", lambda: clock["now"])

    alert = detector.check(state, metric="train/loss")
    assert alert is not None
    assert alert.metric == "train/loss"
    assert alert.current == 0.9
    assert alert.message == "train/loss crossed gt 0.5"

    assert detector.check(state, metric="other-metric") is None
    assert detector.check(state, metric="train/loss") is None

    clock["now"] = 161.0
    assert detector.check(state, metric="train/loss") is not None


def test_stall_rule_detects_insufficient_improvement() -> None:
    rule = StallRule(
        metric="train/loss",
        window=3,
        min_delta=0.5,
        direction="decrease",
    )

    assert rule.evaluate([5.0, 4.9, 4.8]) is True
    assert rule.evaluate([5.0, 4.0, 3.0]) is False


def test_load_alert_rules_from_env_parses_threshold_and_stall_rules(
    monkeypatch,
) -> None:
    monkeypatch.setenv(
        "OG_ALERT_RULES",
        json.dumps(
            [
                {
                    "metric": "loss",
                    "threshold": 0.7,
                    "comparison": "gte",
                    "cooldown_secs": 15,
                    "message": "loss too high",
                },
                {
                    "type": "stall",
                    "metric": "accuracy",
                    "window": 5,
                    "min_delta": 0.02,
                    "direction": "increase",
                },
                {"type": "threshold"},
                "skip-me",
            ]
        ),
    )

    rules = load_alert_rules_from_env()

    assert len(rules) == 2
    assert isinstance(rules[0], ThresholdRule)
    assert rules[0].comparison == "gte"
    assert rules[0].message == "loss too high"
    assert isinstance(rules[1], StallRule)
    assert rules[1].window == 5
    assert rules[1].direction == "increase"


def test_default_alert_rules_only_apply_to_demo_training_file(
    monkeypatch,
) -> None:
    monkeypatch.setenv("OG_DEMO_STALL_WINDOW", "7")
    monkeypatch.setenv("OG_DEMO_STALL_MIN_DELTA", "0.125")
    monkeypatch.setenv("OG_DEMO_ALERT_COOLDOWN_SECS", "9")

    rules = default_alert_rules("demo_train.py")

    assert len(rules) == 1
    assert isinstance(rules[0], StallRule)
    assert rules[0].window == 7
    assert rules[0].min_delta == 0.125
    assert rules[0].cooldown_secs == 9.0
    assert default_alert_rules("other_train.py") == []
