from __future__ import annotations

import asyncio
import os
import threading
import time
from collections.abc import Callable, Iterator
from contextlib import contextmanager, suppress
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from og_agent_chat.client import OGDClientError, ping, send_request
from og_agent_chat.server import DaemonConfig, serve


def _wait_for(
    description: str,
    predicate: Callable[[], Any],
    *,
    timeout: float = 5.0,
    interval: float = 0.05,
) -> Any:
    deadline = time.monotonic() + timeout
    last_error: BaseException | None = None
    while time.monotonic() < deadline:
        try:
            result = predicate()
        except BaseException as exc:
            last_error = exc
        else:
            if result:
                return result
        time.sleep(interval)
    if last_error is not None:
        raise AssertionError(f"Timed out waiting for {description}: {last_error}") from last_error
    raise AssertionError(f"Timed out waiting for {description}")


def _log_contains(logs: list[str], text: str) -> bool:
    return any(text in line for line in logs)


def _short_socket_path() -> Path:
    return Path("/tmp") / f"ogd-{os.getpid()}-{time.time_ns()}.sock"


@dataclass
class _DaemonHarness:
    config: DaemonConfig
    ready: threading.Event = field(default_factory=threading.Event)
    loop: asyncio.AbstractEventLoop | None = None
    task: asyncio.Task[None] | None = None
    thread: threading.Thread | None = None
    error: BaseException | None = None

    def start(self) -> None:
        def runner() -> None:
            loop = asyncio.new_event_loop()
            asyncio.set_event_loop(loop)
            self.loop = loop
            self.task = loop.create_task(serve(self.config))
            self.ready.set()
            try:
                loop.run_until_complete(self.task)
            except asyncio.CancelledError:
                pass
            except BaseException as exc:
                self.error = exc
            finally:
                loop.close()

        self.thread = threading.Thread(target=runner, name="og-agent-chat-test-daemon", daemon=True)
        self.thread.start()
        assert self.ready.wait(timeout=5), "daemon thread did not initialize"

        def ping_ready() -> bool:
            self.assert_healthy()
            return ping(self.config.socket_path)["ok"]

        _wait_for("daemon ping", ping_ready, timeout=10.0)

    def assert_healthy(self) -> None:
        if self.error is not None:
            raise self.error
        if self.thread is not None and not self.thread.is_alive():
            raise AssertionError("daemon exited unexpectedly")

    def request(self, payload: dict[str, Any]) -> dict[str, Any]:
        self.assert_healthy()
        return send_request(payload, self.config.socket_path)

    def get_run_state(
        self,
        *,
        log_tail: int = 200,
        metric_tail: int = 20,
    ) -> dict[str, Any]:
        response = self.request(
            {
                "type": "get_run_state",
                "log_tail": log_tail,
                "metric_tail": metric_tail,
            }
        )
        assert response["ok"] is True
        return response["run_state"]

    def wait_for_run_state(
        self,
        predicate: Callable[[dict[str, Any]], bool],
        *,
        description: str,
        timeout: float = 10.0,
        log_tail: int = 200,
        metric_tail: int = 20,
    ) -> dict[str, Any]:
        def fetch() -> dict[str, Any] | None:
            try:
                state = self.get_run_state(log_tail=log_tail, metric_tail=metric_tail)
            except (AssertionError, ConnectionError, FileNotFoundError, OGDClientError):
                self.assert_healthy()
                return None
            return state if predicate(state) else None

        return _wait_for(description, fetch, timeout=timeout)

    def stop(self) -> None:
        if self.loop is not None and self.task is not None and not self.task.done():
            self.loop.call_soon_threadsafe(self.task.cancel)
        if self.thread is not None:
            self.thread.join(timeout=10)
            if self.thread.is_alive():
                raise AssertionError("daemon thread did not stop")
        if self.error is not None:
            raise self.error


@contextmanager
def _running_daemon(config: DaemonConfig) -> Iterator[_DaemonHarness]:
    harness = _DaemonHarness(config)
    harness.start()
    try:
        yield harness
    finally:
        harness.stop()
        with suppress(OSError):
            config.socket_path.unlink()


def test_serve_supports_real_socket_round_trips_for_runtime_controls(tmp_path: Path) -> None:
    training_file = tmp_path / "train.py"
    training_file.write_text("print('integration smoke', flush=True)\n", encoding="utf-8")

    socket_path = _short_socket_path()
    config = DaemonConfig(
        training_file=training_file,
        codebase_root=tmp_path,
        socket_path=socket_path,
    )

    with _running_daemon(config) as daemon:
        assert ping(socket_path) == {"ok": True, "type": "pong"}

        initial_state = daemon.get_run_state(log_tail=20)
        assert initial_state["runtime_status"] == "idle"
        assert initial_state["runtime"] == "local"
        assert initial_state["auto_mode"] is False

        assert daemon.request({"type": "set_auto_mode", "enabled": True}) == {
            "ok": True,
            "auto_mode": True,
        }
        assert daemon.request({"type": "set_runtime", "runtime": "modal"}) == {
            "ok": True,
            "runtime": "modal",
        }

        updated_state = daemon.get_run_state(log_tail=20)
        assert updated_state["auto_mode"] is True
        assert updated_state["runtime"] == "modal"
        assert updated_state["runtime_status"] == "idle"
        assert _log_contains(updated_state["logs"], "[system] runtime backend set to modal")


def test_serve_auto_recovers_from_oom_and_reports_policy_state(
    tmp_path: Path,
    monkeypatch,
) -> None:
    monkeypatch.setenv("TEST_BATCH", "8")
    monkeypatch.setenv("TEST_GRAD_ACCUM", "1")
    monkeypatch.setenv("TEST_SEQ_LEN", "1000")

    training_file = tmp_path / "train_oom_then_recover.py"
    training_file.write_text(
        "\n".join(
            [
                "import os",
                "",
                "batch = int(os.environ['TEST_BATCH'])",
                "accum = int(os.environ['TEST_GRAD_ACCUM'])",
                "seq = int(os.environ['TEST_SEQ_LEN'])",
                "print(f'runtime env batch={batch} accum={accum} seq={seq}', flush=True)",
                "if batch > 4:",
                "    print('CUDA out of memory while allocating tensor', flush=True)",
                "    raise SystemExit(1)",
                "print('training recovered', flush=True)",
            ]
        )
        + "\n",
        encoding="utf-8",
    )

    socket_path = _short_socket_path()
    config = DaemonConfig(
        training_file=training_file,
        codebase_root=tmp_path,
        socket_path=socket_path,
        start_training=True,
        auto_mode=True,
        max_runtime_retries=2,
        runtime_retry_backoff_secs=0.05,
        runtime_retry_backoff_max_secs=0.05,
        runtime_heartbeat_timeout_secs=5.0,
        runtime_heartbeat_check_secs=0.5,
        oom_batch_env_keys=["TEST_BATCH"],
        oom_accum_env_keys=["TEST_GRAD_ACCUM"],
        oom_seq_env_keys=["TEST_SEQ_LEN"],
    )

    with _running_daemon(config) as daemon:
        completed_state = daemon.wait_for_run_state(
            lambda state: state["runtime_status"] == "completed",
            description="oom recovery completion",
            timeout=10.0,
            log_tail=200,
            metric_tail=20,
        )

        assert completed_state["runtime_status"] == "completed"
        assert completed_state["runtime_restarts"] == 1
        assert completed_state["runtime_last_exit_code"] == 0
        assert completed_state["runtime_failure_reason"] is None
        assert completed_state["runtime_failure_class"] is None
        assert completed_state["runtime_error_type"] is None
        assert completed_state["rollout_observed_state"] == "completed"
        assert completed_state["rollout_desired_state"] == "completed"
        assert completed_state["runtime_env_overrides"] == {
            "TEST_BATCH": "4",
            "TEST_GRAD_ACCUM": "2",
            "TEST_SEQ_LEN": "800",
        }
        assert completed_state["metrics"]["runtime/oom_policy_applied"][-1] == 1.0

        logs = completed_state["logs"]
        assert _log_contains(logs, "runtime env batch=8 accum=1 seq=1000")
        assert _log_contains(logs, "CUDA out of memory while allocating tensor")
        assert _log_contains(logs, "oom policy applied: TEST_BATCH: 8 -> 4")
        assert _log_contains(logs, "TEST_GRAD_ACCUM: 1 -> 2")
        assert _log_contains(logs, "TEST_SEQ_LEN: 1000 -> 800")
        assert _log_contains(logs, "runtime recovery scheduled in")
        assert _log_contains(logs, "runtime recovery restarted training")
        assert _log_contains(logs, "runtime env batch=4 accum=2 seq=800")
        assert _log_contains(logs, "training recovered")
        assert _log_contains(logs, "[system] training job completed")
