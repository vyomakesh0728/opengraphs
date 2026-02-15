from __future__ import annotations

import json
import os
import time
from dataclasses import dataclass
from typing import Literal, Sequence

from .models import Alert, RunState

Comparison = Literal["gt", "gte", "lt", "lte"]
Direction = Literal["decrease", "increase"]


@dataclass
class ThresholdRule:
    metric: str
    threshold: float
    comparison: Comparison = "gt"
    cooldown_secs: float = 60.0
    message: str | None = None

    def evaluate(self, values: Sequence[float]) -> bool:
        if not values:
            return False
        current = values[-1]
        if self.comparison == "gt":
            return current > self.threshold
        if self.comparison == "gte":
            return current >= self.threshold
        if self.comparison == "lt":
            return current < self.threshold
        if self.comparison == "lte":
            return current <= self.threshold
        return False

    def to_alert(self, current: float) -> Alert:
        return Alert(
            metric=self.metric,
            threshold=self.threshold,
            current=current,
            message=self.message or f"{self.metric} crossed {self.comparison} {self.threshold}",
            timestamp=time.time(),
        )


@dataclass
class StallRule:
    metric: str
    window: int = 20
    min_delta: float = 0.0
    direction: Direction = "decrease"
    cooldown_secs: float = 60.0
    message: str | None = None

    def evaluate(self, values: Sequence[float]) -> bool:
        if len(values) < self.window:
            return False
        start = values[-self.window]
        end = values[-1]
        if self.direction == "decrease":
            return (start - end) < self.min_delta
        return (end - start) < self.min_delta

    def to_alert(self, current: float) -> Alert:
        return Alert(
            metric=self.metric,
            threshold=self.min_delta,
            current=current,
            message=self.message
            or f"{self.metric} stalled (delta < {self.min_delta})",
            timestamp=time.time(),
        )


AlertRule = ThresholdRule | StallRule


class AlertDetector:
    def __init__(self, rules: Sequence[AlertRule] | None = None) -> None:
        self.rules = list(rules or [])
        self.last_triggered: dict[str, float] = {}

    def set_rules(self, rules: Sequence[AlertRule]) -> None:
        self.rules = list(rules)

    def check(self, run_state: RunState, metric: str | None = None) -> Alert | None:
        now = time.time()
        for rule in self.rules:
            if metric and rule.metric != metric:
                continue
            last = self.last_triggered.get(rule.metric, 0.0)
            if now - last < rule.cooldown_secs:
                continue
            values = run_state.metrics.get(rule.metric, [])
            if not values:
                continue
            if rule.evaluate(values):
                current = values[-1]
                alert = rule.to_alert(current)
                self.last_triggered[rule.metric] = now
                return alert
        return None


def load_alert_rules_from_env(env_var: str = "OG_ALERT_RULES") -> list[AlertRule]:
    payload = os.getenv(env_var)
    if not payload:
        return []
    try:
        raw = json.loads(payload)
    except json.JSONDecodeError:
        return []
    if not isinstance(raw, list):
        return []

    rules: list[AlertRule] = []
    for item in raw:
        if not isinstance(item, dict):
            continue
        rule_type = item.get("type", "threshold")
        metric = item.get("metric")
        if not metric:
            continue
        if rule_type == "stall":
            rules.append(
                StallRule(
                    metric=metric,
                    window=int(item.get("window", 20)),
                    min_delta=float(item.get("min_delta", 0.0)),
                    direction=item.get("direction", "decrease"),
                    cooldown_secs=float(item.get("cooldown_secs", 60.0)),
                    message=item.get("message"),
                )
            )
        else:
            rules.append(
                ThresholdRule(
                    metric=metric,
                    threshold=float(item.get("threshold", 0.0)),
                    comparison=item.get("comparison", "gt"),
                    cooldown_secs=float(item.get("cooldown_secs", 60.0)),
                    message=item.get("message"),
                )
            )
    return rules
