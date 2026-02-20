from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Literal

ActionType = Literal["explain", "refactor"]
SenderType = Literal["user", "agent", "system"]
RuntimeType = Literal["local", "prime", "modal"]


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
    runtime: RuntimeType = "local"
    metrics: dict[str, list[float]] = field(default_factory=dict)
    logs: list[str] = field(default_factory=list)
    alerts: list[Alert] = field(default_factory=list)
    current_step: int = 0
    is_active: bool = True
    runtime_status: str = "idle"
    runtime_id: str | None = None
    runtime_failure_reason: str | None = None
    runtime_error_type: str | None = None
    runtime_restarts: int = 0
    runtime_last_heartbeat: float | None = None
    runtime_last_exit_code: int | None = None

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
