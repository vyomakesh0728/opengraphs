from __future__ import annotations

import argparse
import asyncio
import contextlib
import json
import logging
import os
import stat
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Awaitable, Callable

from .agent import AgentEngine
from .alerts import AlertDetector, AlertRule, default_alert_rules, load_alert_rules_from_env
from .models import ActionPlan, Alert, ChatMessage, RunState
from .runtime import RuntimeFailure, RuntimeType, build_runtime_adapter


LOGGER = logging.getLogger(__name__)


@dataclass
class DaemonConfig:
    training_file: Path
    codebase_root: Path
    socket_path: Path
    run_dir: Path | None = None
    training_cmd: str | None = None
    start_training: bool = False
    fresh_run: bool = False
    auto_mode: bool = False
    runtime: RuntimeType = "local"
    max_runtime_retries: int = 2
    runtime_retry_backoff_secs: float = 2.0
    runtime_retry_backoff_max_secs: float = 20.0
    runtime_heartbeat_timeout_secs: float = 30.0
    runtime_heartbeat_check_secs: float = 2.0
    alert_rules: list[AlertRule] = field(default_factory=list)


def _serialize_message(message: ChatMessage) -> dict[str, Any]:
    return {
        "sender": message.sender,
        "content": message.content,
        "timestamp": message.timestamp,
    }


def _serialize_plan(plan: ActionPlan) -> dict[str, Any]:
    return {
        "diagnosis": plan.diagnosis,
        "action": plan.action,
        "code_changes": plan.code_changes,
        "raw_output": plan.raw_output,
    }


def _serialize_alert(alert: Alert) -> dict[str, Any]:
    return {
        "metric": alert.metric,
        "threshold": alert.threshold,
        "current": alert.current,
        "message": alert.message,
        "timestamp": alert.timestamp,
    }


def _prepare_socket_path(socket_path: Path) -> None:
    if socket_path.exists():
        mode = socket_path.stat().st_mode
        if stat.S_ISSOCK(mode) or socket_path.is_file():
            socket_path.unlink()
        else:
            raise RuntimeError(f"Socket path exists and is not a file: {socket_path}")
    socket_path.parent.mkdir(parents=True, exist_ok=True)


def _default_socket_path() -> str:
    tmpdir = os.getenv("TMPDIR") or os.getenv("TEMP") or os.getenv("TMP") or "/tmp"
    return str(Path(tmpdir) / "opengraphs-ogd.sock")


def _clear_tfevents_files(run_dir: Path) -> int:
    removed = 0
    if not run_dir.exists():
        return removed
    for path in run_dir.rglob("*"):
        if path.is_file() and "tfevents" in path.name:
            try:
                path.unlink()
                removed += 1
            except OSError as exc:
                LOGGER.warning("Failed to remove %s: %s", path, exc)
    return removed


def _resolve_run_dir(path: Path | None) -> Path | None:
    if path is None:
        return None
    # If user passed a single event file path, use its parent directory for TB output.
    if "tfevents" in path.name:
        return path.parent
    return path


def _normalize_runtime(value: str | None) -> RuntimeType:
    raw = (value or "local").strip().lower()
    if raw not in {"local", "prime", "modal"}:
        raise ValueError(f"unsupported runtime '{value}' (expected local|prime|modal)")
    return raw  # type: ignore[return-value]


async def serve(config: DaemonConfig) -> None:
    runtime_ref: dict[str, RuntimeType] = {"kind": config.runtime}
    run_state = RunState(
        training_file=config.training_file,
        codebase_root=config.codebase_root,
        runtime=runtime_ref["kind"],
    )
    runtime_adapter = None
    runtime_retry_count = 0
    runtime_failure_lock = asyncio.Lock()

    def _runtime_status_health(status: str) -> float:
        normalized = status.strip().lower()
        if normalized == "running":
            return 1.0
        if normalized == "starting":
            return 0.7
        if normalized == "recovering":
            return 0.5
        return 0.0

    def _runtime_status_code(status: str) -> float:
        normalized = status.strip().lower()
        codes = {
            "idle": 0.0,
            "starting": 1.0,
            "running": 2.0,
            "recovering": 3.0,
            "failed": 4.0,
            "stopped": 5.0,
            "error": 6.0,
        }
        return codes.get(normalized, 99.0)

    def _record_runtime_metrics() -> None:
        run_state.add_metric(
            "runtime/health",
            _runtime_status_health(run_state.runtime_status),
            step=run_state.current_step,
        )
        run_state.add_metric(
            "runtime/state_code",
            _runtime_status_code(run_state.runtime_status),
            step=run_state.current_step,
        )
        run_state.add_metric(
            "runtime/restarts",
            float(run_state.runtime_restarts),
            step=run_state.current_step,
        )

    def _mark_runtime_heartbeat() -> None:
        run_state.runtime_last_heartbeat = time.time()

    def _append_runtime_log(line: str) -> None:
        run_state.append_log(line)
        _mark_runtime_heartbeat()

    def _set_runtime_state(
        *,
        status: str,
        runtime_id: str | None = None,
        reason: str | None = None,
        error_type: str | None = None,
        failure_class: str | None = None,
        exit_code: int | None = None,
    ) -> None:
        previous_status = run_state.runtime_status
        run_state.runtime_status = status
        if runtime_id is not None:
            run_state.runtime_id = runtime_id
        if reason is not None:
            run_state.runtime_failure_reason = reason
        if error_type is not None:
            run_state.runtime_error_type = error_type
        if failure_class is not None:
            run_state.runtime_failure_class = failure_class
        if exit_code is not None:
            run_state.runtime_last_exit_code = exit_code
        _mark_runtime_heartbeat()
        _record_runtime_metrics()
        if previous_status != status:
            details = f" ({reason})" if reason else ""
            run_state.append_log(f"[system] runtime status -> {status}{details}")

    def _runtime_retry_backoff_secs(attempt: int) -> float:
        base = max(config.runtime_retry_backoff_secs, 0.1)
        ceiling = max(config.runtime_retry_backoff_max_secs, base)
        return min(base * (2 ** max(attempt - 1, 0)), ceiling)

    def _runtime_failure_message(failure: RuntimeFailure) -> str:
        failure_class = _classify_runtime_failure(failure)
        details: list[str] = []
        details.append(f"class={failure_class}")
        if failure.status:
            details.append(f"status={failure.status}")
        if failure.error_type:
            details.append(f"type={failure.error_type}")
        if failure.exit_code is not None:
            details.append(f"exit={failure.exit_code}")
        if failure.message:
            details.append(failure.message)
        return " | ".join(details) if details else "runtime failure"

    def _classify_runtime_failure(failure: RuntimeFailure) -> str:
        text = " ".join(
            [
                failure.status or "",
                failure.error_type or "",
                failure.message or "",
            ]
        ).lower()
        if any(
            token in text
            for token in (
                "oom",
                "out of memory",
                "cuda out of memory",
                "memoryerror",
                "killed",
            )
        ):
            return "oom"
        if any(
            token in text
            for token in (
                "timeout",
                "timed out",
                "deadline exceeded",
                "heartbeat stale",
            )
        ):
            return "timeout"
        if any(
            token in text
            for token in (
                "terminated",
                "stopped",
                "not running",
                "not found",
                "deleted",
                "gone",
            )
        ):
            return "terminated"
        if any(
            token in text
            for token in (
                "insufficient balance",
                "insufficient quota",
                "insufficient_funds",
                "quota",
            )
        ):
            return "quota"
        if any(
            token in text
            for token in (
                "unauthorized",
                "forbidden",
                "invalid api key",
                "authentication",
                "401",
                "403",
            )
        ):
            return "auth"
        if any(
            token in text
            for token in (
                "apierror",
                "http",
                "rate limit",
                "429",
                "gateway",
                "dns",
                "connection",
            )
        ):
            return "api"
        return "unknown"

    async def _stop_training_process() -> None:
        nonlocal runtime_adapter
        if runtime_adapter is None:
            return
        try:
            await runtime_adapter.close()
        finally:
            runtime_adapter = None
            if run_state.runtime_status not in {"failed", "error"}:
                _set_runtime_state(status="stopped")

    async def _restart_training_process(
        state: RunState,
        *,
        reset_retry_budget: bool = True,
    ) -> None:
        nonlocal runtime_adapter, runtime_retry_count

        await _stop_training_process()

        runtime_adapter = build_runtime_adapter(
            runtime=runtime_ref["kind"],
            training_file=state.training_file,
            codebase_root=state.codebase_root,
            socket_path=config.socket_path,
            run_dir=_resolve_run_dir(config.run_dir),
            training_cmd=config.training_cmd,
            on_log=_append_runtime_log,
            on_failure=_handle_runtime_failure,
            on_heartbeat=_mark_runtime_heartbeat,
        )

        run_state.runtime_id = None
        _set_runtime_state(status="starting")
        start = await runtime_adapter.start()
        if reset_retry_budget:
            runtime_retry_count = 0
            run_state.runtime_restarts = 0
        _set_runtime_state(
            status="running",
            runtime_id=start.runtime_id,
            exit_code=0,
        )
        run_state.runtime = runtime_ref["kind"]
        run_state.runtime_failure_reason = None
        run_state.runtime_error_type = None

    async def _maybe_recover_runtime_failure(failure: RuntimeFailure) -> None:
        nonlocal runtime_retry_count
        if not config.auto_mode:
            _append_runtime_log(
                "[system] auto mode disabled; runtime recovery requires manual restart"
            )
            return
        if runtime_retry_count >= config.max_runtime_retries:
            _append_runtime_log(
                f"[error] runtime retry budget exhausted ({config.max_runtime_retries})"
            )
            return
        attempt = runtime_retry_count + 1
        runtime_retry_count = attempt
        run_state.runtime_restarts = attempt
        _record_runtime_metrics()
        backoff = _runtime_retry_backoff_secs(attempt)
        _set_runtime_state(
            status="recovering",
            reason=f"retrying after failure in {backoff:.1f}s (attempt {attempt})",
        )
        _append_runtime_log(
            f"[system] runtime recovery scheduled in {backoff:.1f}s (attempt {attempt}/{config.max_runtime_retries})"
        )
        await asyncio.sleep(backoff)
        try:
            await _restart_training_process(run_state, reset_retry_budget=False)
            _append_runtime_log("[system] runtime recovery restarted training")
        except Exception as exc:
            _append_runtime_log(f"[error] runtime recovery failed: {exc}")

    async def _handle_runtime_failure(failure: RuntimeFailure) -> None:
        async with runtime_failure_lock:
            failure_class = _classify_runtime_failure(failure)
            message = _runtime_failure_message(failure)
            _set_runtime_state(
                status="failed",
                reason=message,
                error_type=failure.error_type,
                failure_class=failure_class,
                exit_code=failure.exit_code,
            )
            _append_runtime_log(f"[error] runtime failure: {message}")
            alert = Alert(
                metric="runtime/health",
                threshold=0.0,
                current=1.0,
                message=message,
                timestamp=time.time(),
            )
            run_state.add_alert(alert)
            run_state.add_metric(
                "runtime/failures",
                float(run_state.runtime_restarts + 1),
                step=run_state.current_step,
            )

            response = await agent.handle_alert(alert)
            agent_handled_with_restart = (
                config.auto_mode
                and response is not None
                and response.plan.action == "refactor"
                and bool(response.plan.code_changes)
            )
            if agent_handled_with_restart:
                _append_runtime_log(
                    "[system] agent proposed refactor; relying on refactor restart path"
                )
                return

            await _maybe_recover_runtime_failure(failure)

    agent = AgentEngine(
        run_state=run_state,
        codebase_root=config.codebase_root,
        auto_mode=config.auto_mode,
        restart_callback=_restart_training_process,
    )
    alert_detector = AlertDetector(config.alert_rules)

    async def handle_client(
        reader: asyncio.StreamReader,
        writer: asyncio.StreamWriter,
    ) -> None:
        try:
            while True:
                try:
                    line = await reader.readline()
                except (ConnectionResetError, BrokenPipeError, OSError):
                    break
                if not line:
                    break
                try:
                    payload = json.loads(line.decode("utf-8"))
                except json.JSONDecodeError:
                    await _write_response(writer, {"ok": False, "error": "invalid_json"})
                    continue

                try:
                    response = await _handle_payload(
                        payload,
                        run_state,
                        agent,
                        alert_detector,
                        _restart_training_process,
                        runtime_ref,
                    )
                except asyncio.CancelledError:
                    raise
                except Exception:
                    LOGGER.exception(
                        "Payload handler failed for type=%s",
                        payload.get("type"),
                    )
                    response = {"ok": False, "error": "internal_error"}
                try:
                    await _write_response(writer, response)
                except (ConnectionResetError, BrokenPipeError, OSError):
                    break
        except Exception as exc:
            # Keep daemon alive if a single client connection fails unexpectedly.
            if isinstance(exc, (ConnectionResetError, BrokenPipeError, OSError)):
                return
            LOGGER.exception("Client handler failed")
        finally:
            try:
                writer.close()
                await writer.wait_closed()
            except (ConnectionResetError, BrokenPipeError, OSError):
                pass

    _prepare_socket_path(config.socket_path)
    server = await asyncio.start_unix_server(handle_client, path=str(config.socket_path))
    async def _runtime_watchdog_loop() -> None:
        interval = max(config.runtime_heartbeat_check_secs, 0.5)
        timeout = max(config.runtime_heartbeat_timeout_secs, interval)
        while True:
            await asyncio.sleep(interval)
            if runtime_adapter is None:
                continue
            if run_state.runtime_status != "running":
                continue
            last = run_state.runtime_last_heartbeat
            if last is None:
                continue
            age = time.time() - last
            if age <= timeout:
                continue
            await _handle_runtime_failure(
                RuntimeFailure(
                    status="timeout",
                    error_type="RUNTIME_HEARTBEAT_TIMEOUT",
                    message=(
                        f"runtime heartbeat stale for {age:.1f}s "
                        f"(limit {timeout:.1f}s)"
                    ),
                )
            )

    runtime_watchdog_task = asyncio.create_task(_runtime_watchdog_loop())
    async with server:
        try:
            if config.start_training:
                run_dir = _resolve_run_dir(config.run_dir)
                if config.fresh_run and run_dir is not None:
                    removed = _clear_tfevents_files(run_dir)
                    run_state.append_log(
                        f"[system] fresh run enabled; removed {removed} existing tfevents file(s)"
                    )
                try:
                    await _restart_training_process(run_state)
                except Exception as exc:
                    run_state.append_log(f"[error] failed to start training: {exc}")
            await server.serve_forever()
        finally:
            runtime_watchdog_task.cancel()
            with contextlib.suppress(asyncio.CancelledError):
                await runtime_watchdog_task
            await _stop_training_process()


async def _write_response(writer: asyncio.StreamWriter, response: dict[str, Any]) -> None:
    try:
        writer.write((json.dumps(response) + "\n").encode("utf-8"))
        await writer.drain()
    except (ConnectionResetError, BrokenPipeError, OSError):
        return


async def _handle_payload(
    payload: dict[str, Any],
    run_state: RunState,
    agent: AgentEngine,
    alert_detector: AlertDetector,
    restart_training_callback: Callable[[RunState], Awaitable[None]] | None = None,
    runtime_ref: dict[str, RuntimeType] | None = None,
) -> dict[str, Any]:
    msg_type = payload.get("type")

    if msg_type == "ping":
        return {"ok": True, "type": "pong"}

    if msg_type == "get_chat_history":
        history = [_serialize_message(msg) for msg in agent.get_chat_history()]
        return {"ok": True, "chat_history": history}

    if msg_type == "get_run_state":
        log_tail = int(payload.get("log_tail", 200))
        metric_tail = int(payload.get("metric_tail", 1))
        metrics_payload: dict[str, list[float]] = {}
        for metric, values in run_state.metrics.items():
            if not values:
                continue
            metrics_payload[metric] = values[-metric_tail:]
        logs_payload = run_state.logs[-log_tail:]
        alerts_payload = [_serialize_alert(alert) for alert in run_state.alerts]
        return {
            "ok": True,
            "run_state": {
                "metrics": metrics_payload,
                "logs": logs_payload,
                "alerts": alerts_payload,
                "current_step": run_state.current_step,
                "auto_mode": agent.executor.auto_mode,
                "runtime": run_state.runtime,
                "runtime_status": run_state.runtime_status,
                "runtime_id": run_state.runtime_id,
                "runtime_failure_reason": run_state.runtime_failure_reason,
                "runtime_error_type": run_state.runtime_error_type,
                "runtime_failure_class": run_state.runtime_failure_class,
                "runtime_restarts": run_state.runtime_restarts,
                "runtime_last_heartbeat": run_state.runtime_last_heartbeat,
                "runtime_last_exit_code": run_state.runtime_last_exit_code,
            },
        }

    if msg_type == "chat_message":
        content = payload.get("content", "")
        if not content:
            return {"ok": False, "error": "missing_content"}
        response = await agent.handle_chat_message(content)
        return {
            "ok": True,
            "response": _serialize_plan(response.plan),
            "chat_history": [_serialize_message(msg) for msg in agent.get_chat_history()],
        }

    if msg_type == "metrics_update":
        metric = payload.get("metric")
        value = payload.get("value")
        step = payload.get("step")
        if metric is None or value is None:
            return {"ok": False, "error": "missing_metric_or_value"}
        try:
            value = float(value)
        except (TypeError, ValueError):
            return {"ok": False, "error": "invalid_value"}
        run_state.add_metric(metric, value, step=step)

        alert = alert_detector.check(run_state, metric=metric)
        response: dict[str, Any] = {"ok": True}
        if alert:
            run_state.add_alert(alert)
            agent_response = await agent.handle_alert(alert)
            response["alert"] = _serialize_alert(alert)
            if agent_response:
                response["agent_response"] = _serialize_plan(agent_response.plan)
        return response

    if msg_type == "log_append":
        line = payload.get("line")
        if not line:
            return {"ok": False, "error": "missing_line"}
        run_state.append_log(str(line))
        return {"ok": True}

    if msg_type == "set_training_file":
        path = payload.get("path")
        if not path:
            return {"ok": False, "error": "missing_path"}
        run_state.training_file = Path(path)
        return {"ok": True}

    if msg_type == "set_auto_mode":
        enabled = bool(payload.get("enabled", False))
        agent.executor.auto_mode = enabled
        return {"ok": True, "auto_mode": enabled}

    if msg_type == "set_runtime":
        if runtime_ref is None:
            return {"ok": False, "error": "runtime_control_unavailable"}
        runtime_raw = payload.get("runtime")
        try:
            runtime = _normalize_runtime(str(runtime_raw))
        except ValueError as exc:
            return {"ok": False, "error": str(exc)}
        runtime_ref["kind"] = runtime
        run_state.runtime = runtime
        run_state.append_log(f"[system] runtime backend set to {runtime}")
        return {"ok": True, "runtime": runtime}

    if msg_type == "start_training":
        if restart_training_callback is None:
            return {"ok": False, "error": "training_control_unavailable"}
        try:
            await restart_training_callback(run_state)
        except Exception as exc:
            return {"ok": False, "error": f"failed_to_start_training: {exc}"}
        return {"ok": True}

    if msg_type == "apply_refactor":
        diagnosis = payload.get("diagnosis", "")
        action = payload.get("action", "refactor")
        code_changes = payload.get("code_changes", "")
        raw_output = payload.get("raw_output", "")
        if not code_changes:
            return {"ok": False, "error": "missing_code_changes"}
        plan = ActionPlan(
            diagnosis=diagnosis,
            action=action,
            code_changes=code_changes,
            raw_output=raw_output,
        )
        result = await agent.execute_plan(plan)
        return {
            "ok": True,
            "success": result.success,
            "checkpoint_id": result.checkpoint_id,
            "error": result.error,
            "chat_history": [_serialize_message(msg) for msg in agent.get_chat_history()],
        }

    return {"ok": False, "error": "unknown_type"}


def main() -> None:
    parser = argparse.ArgumentParser(description="OpenGraphs agent chat daemon")
    parser.add_argument(
        "--socket",
        default=os.getenv("OGD_SOCKET", _default_socket_path()),
        help="Unix socket path for ogd communication",
    )
    parser.add_argument(
        "--training-file",
        default=os.getenv("OG_TRAINING_FILE"),
        help="Path to the training script",
    )
    parser.add_argument(
        "--codebase-root",
        default=os.getenv("OG_CODEBASE_ROOT", "."),
        help="Root directory for codebase indexing",
    )
    parser.add_argument(
        "--run-dir",
        default=os.getenv("OG_RUN_DIR"),
        help="Run directory for TensorBoard event output (used for TB_LOG_DIR if unset)",
    )
    parser.add_argument(
        "--training-cmd",
        default=os.getenv("OG_TRAINING_CMD"),
        help='Training command to run (e.g. "torchrun --standalone --nproc_per_node=1 train_gpt.py")',
    )
    parser.add_argument(
        "--start-training",
        action="store_true",
        default=os.getenv("OG_START_TRAINING", "0") == "1",
        help="Start training process automatically on daemon startup",
    )
    parser.add_argument(
        "--fresh-run",
        action="store_true",
        default=os.getenv("OG_FRESH_RUN", "0") == "1",
        help="Delete existing tfevents files under --run-dir before auto-start",
    )
    parser.add_argument(
        "--auto",
        action="store_true",
        default=os.getenv("OG_AGENT_AUTO", "0") == "1",
        help="Enable auto refactor mode",
    )
    parser.add_argument(
        "--runtime",
        default=os.getenv("OG_RUNTIME", "local"),
        help="Training runtime backend: local|prime|modal",
    )
    parser.add_argument(
        "--max-runtime-retries",
        type=int,
        default=int(os.getenv("OG_MAX_RUNTIME_RETRIES", "2")),
        help="Maximum auto-recovery retries for runtime failures",
    )
    parser.add_argument(
        "--runtime-retry-backoff-secs",
        type=float,
        default=float(os.getenv("OG_RUNTIME_RETRY_BACKOFF_SECS", "2")),
        help="Base backoff seconds for runtime recovery retries",
    )
    parser.add_argument(
        "--runtime-retry-backoff-max-secs",
        type=float,
        default=float(os.getenv("OG_RUNTIME_RETRY_BACKOFF_MAX_SECS", "20")),
        help="Maximum backoff seconds for runtime recovery retries",
    )
    parser.add_argument(
        "--runtime-heartbeat-timeout-secs",
        type=float,
        default=float(os.getenv("OG_RUNTIME_HEARTBEAT_TIMEOUT_SECS", "30")),
        help="Runtime heartbeat stale timeout (seconds) before fail-fast recovery",
    )
    parser.add_argument(
        "--runtime-heartbeat-check-secs",
        type=float,
        default=float(os.getenv("OG_RUNTIME_HEARTBEAT_CHECK_SECS", "2")),
        help="Runtime heartbeat watchdog polling interval in seconds",
    )

    args = parser.parse_args()
    if not args.training_file:
        raise SystemExit("--training-file or OG_TRAINING_FILE is required")

    alert_rules = load_alert_rules_from_env()
    if not alert_rules:
        alert_rules = default_alert_rules(args.training_file)

    runtime = _normalize_runtime(args.runtime)
    max_runtime_retries = max(int(args.max_runtime_retries), 0)
    retry_backoff = max(float(args.runtime_retry_backoff_secs), 0.1)
    retry_backoff_max = max(float(args.runtime_retry_backoff_max_secs), retry_backoff)
    heartbeat_check = max(float(args.runtime_heartbeat_check_secs), 0.5)
    heartbeat_timeout = max(float(args.runtime_heartbeat_timeout_secs), heartbeat_check)

    config = DaemonConfig(
        training_file=Path(args.training_file),
        codebase_root=Path(args.codebase_root),
        socket_path=Path(args.socket),
        run_dir=Path(args.run_dir) if args.run_dir else None,
        training_cmd=args.training_cmd,
        start_training=args.start_training,
        fresh_run=args.fresh_run,
        auto_mode=args.auto,
        runtime=runtime,
        max_runtime_retries=max_runtime_retries,
        runtime_retry_backoff_secs=retry_backoff,
        runtime_retry_backoff_max_secs=retry_backoff_max,
        runtime_heartbeat_timeout_secs=heartbeat_timeout,
        runtime_heartbeat_check_secs=heartbeat_check,
        alert_rules=alert_rules,
    )
    asyncio.run(serve(config))


if __name__ == "__main__":
    main()
