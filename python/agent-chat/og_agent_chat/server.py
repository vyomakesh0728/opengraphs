from __future__ import annotations

import argparse
import asyncio
import json
import logging
import os
import stat
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from .agent import AgentEngine
from .alerts import AlertDetector, AlertRule, load_alert_rules_from_env
from .models import ActionPlan, Alert, ChatMessage, RunState


LOGGER = logging.getLogger(__name__)


@dataclass
class DaemonConfig:
    training_file: Path
    codebase_root: Path
    socket_path: Path
    auto_mode: bool = False
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


async def serve(config: DaemonConfig) -> None:
    run_state = RunState(
        training_file=config.training_file,
        codebase_root=config.codebase_root,
    )
    training_process: asyncio.subprocess.Process | None = None
    training_log_task: asyncio.Task[None] | None = None

    async def _stream_training_logs(process: asyncio.subprocess.Process) -> None:
        if process.stdout is None:
            return
        while True:
            line = await process.stdout.readline()
            if not line:
                break
            run_state.append_log(line.decode("utf-8", errors="replace").rstrip("\n"))
        return_code = await process.wait()
        run_state.append_log(f"[system] training exited with code {return_code}")

    async def _stop_training_process() -> None:
        nonlocal training_process, training_log_task

        if training_process and training_process.returncode is None:
            training_process.terminate()
            try:
                await asyncio.wait_for(training_process.wait(), timeout=5)
            except TimeoutError:
                training_process.kill()
                await training_process.wait()

        if training_log_task:
            if not training_log_task.done():
                training_log_task.cancel()
                try:
                    await training_log_task
                except asyncio.CancelledError:
                    pass
            training_log_task = None

        training_process = None

    async def _restart_training_process(state: RunState) -> None:
        nonlocal training_process, training_log_task

        await _stop_training_process()

        env = dict(os.environ)
        env.setdefault("OGD_SOCKET", str(config.socket_path))
        process = await asyncio.create_subprocess_exec(
            sys.executable,
            str(state.training_file),
            cwd=str(state.codebase_root),
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.STDOUT,
            env=env,
        )
        training_process = process
        run_state.append_log(f"[system] training restarted (pid={process.pid})")
        training_log_task = asyncio.create_task(_stream_training_logs(process))

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
    async with server:
        try:
            await server.serve_forever()
        finally:
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
        "--auto",
        action="store_true",
        default=os.getenv("OG_AGENT_AUTO", "0") == "1",
        help="Enable auto refactor mode",
    )

    args = parser.parse_args()
    if not args.training_file:
        raise SystemExit("--training-file or OG_TRAINING_FILE is required")

    config = DaemonConfig(
        training_file=Path(args.training_file),
        codebase_root=Path(args.codebase_root),
        socket_path=Path(args.socket),
        auto_mode=args.auto,
        alert_rules=load_alert_rules_from_env(),
    )
    asyncio.run(serve(config))


if __name__ == "__main__":
    main()
