from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Literal

ActionType = Literal["explain", "refactor"]
SenderType = Literal["user", "agent", "system"]


@dataclass
class Alert:
    metric: str
    threshold: float
    current: float
    message: str
    timestamp: float


@dataclass
class RunState:
    training_file: Path
    codebase_root: Path
    metrics: dict[str, list[float]] = field(default_factory=dict)
    logs: list[str] = field(default_factory=list)
    alerts: list[Alert] = field(default_factory=list)
    current_step: int = 0
    is_active: bool = True

    def latest_alert(self) -> Alert | None:
        if not self.alerts:
            return None
        return self.alerts[-1]

    def metric_tail(self, metric: str, n: int = 20) -> list[float]:
        values = self.metrics.get(metric, [])
        return values[-n:]

    def log_tail(self, n: int = 50) -> str:
        return "\n".join(self.logs[-n:])

    def add_metric(self, metric: str, value: float, step: int | None = None) -> None:
        self.metrics.setdefault(metric, []).append(float(value))
        if step is not None:
            try:
                self.current_step = max(self.current_step, int(step))
            except (TypeError, ValueError):
                pass

    def append_log(self, line: str) -> None:
        self.logs.append(line)

    def add_alert(self, alert: Alert) -> None:
        self.alerts.append(alert)


@dataclass
class ActionPlan:
    diagnosis: str
    action: ActionType
    code_changes: str
    raw_output: str

    def is_refactor(self) -> bool:
        return self.action == "refactor"


@dataclass
class ExecutionResult:
    success: bool
    checkpoint_id: str | None = None
    error: str | None = None


@dataclass
class ChatMessage:
    sender: SenderType
    content: str
    timestamp: float
