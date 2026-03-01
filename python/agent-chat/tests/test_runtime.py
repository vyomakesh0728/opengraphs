from __future__ import annotations

import asyncio
from pathlib import Path

from og_agent_chat.runtime import LocalRuntimeAdapter, RuntimeFailure, _looks_like_oom_text


def _build_adapter(
    *,
    training_file: Path,
    codebase_root: Path,
    socket_path: Path,
    run_dir: Path | None = None,
    training_cmd: str | None = None,
    runtime_env_overrides: dict[str, str] | None = None,
    on_log=None,
    on_failure=None,
    on_complete=None,
    on_heartbeat=None,
) -> LocalRuntimeAdapter:
    async def noop_failure(_: RuntimeFailure) -> None:
        return None

    async def noop_complete(_: str) -> None:
        return None

    return LocalRuntimeAdapter(
        training_file=training_file,
        codebase_root=codebase_root,
        socket_path=socket_path,
        run_dir=run_dir,
        training_cmd=training_cmd,
        runtime_env_overrides=runtime_env_overrides,
        on_log=on_log or (lambda _: None),
        on_failure=on_failure or noop_failure,
        on_complete=on_complete or noop_complete,
        on_heartbeat=on_heartbeat or (lambda: None),
    )


def test_looks_like_oom_text_detects_common_failure_patterns() -> None:
    assert _looks_like_oom_text("CUDA out of memory while allocating") is True
    assert _looks_like_oom_text("Killed process after memory spike") is True
    assert _looks_like_oom_text("completed successfully") is False


def test_local_runtime_adapter_builds_env_and_command(tmp_path) -> None:
    training_file = tmp_path / "train.py"
    training_file.write_text("print('ok')\n", encoding="utf-8")
    run_dir = tmp_path / "runs"
    socket_path = tmp_path / "ogd.sock"
    adapter = _build_adapter(
        training_file=training_file,
        codebase_root=tmp_path,
        socket_path=socket_path,
        run_dir=run_dir,
        training_cmd="python -m demo.train --epochs 3",
        runtime_env_overrides={"BATCH_SIZE": "8"},
    )

    env = adapter._build_env()

    assert env["OGD_SOCKET"] == str(socket_path)
    assert env["TB_LOG_DIR"] == str(run_dir)
    assert env["ENABLE_TB"] == "1"
    assert env["BATCH_SIZE"] == "8"
    assert run_dir.is_dir()
    assert adapter._resolve_command() == ["python", "-m", "demo.train", "--epochs", "3"]


def test_local_runtime_adapter_reports_completion_and_streams_logs(tmp_path) -> None:
    training_file = tmp_path / "train_ok.py"
    training_file.write_text("print('hello from runtime', flush=True)\n", encoding="utf-8")

    logs: list[str] = []
    failures: list[RuntimeFailure] = []
    completions: list[str] = []
    heartbeats = {"count": 0}

    async def on_failure(failure: RuntimeFailure) -> None:
        failures.append(failure)

    async def on_complete(status: str) -> None:
        completions.append(status)

    async def scenario() -> str | None:
        adapter = _build_adapter(
            training_file=training_file,
            codebase_root=tmp_path,
            socket_path=tmp_path / "ogd.sock",
            on_log=logs.append,
            on_failure=on_failure,
            on_complete=on_complete,
            on_heartbeat=lambda: heartbeats.__setitem__("count", heartbeats["count"] + 1),
        )
        result = await adapter.start()
        assert adapter._log_task is not None
        await asyncio.wait_for(adapter._log_task, timeout=10)
        await adapter.close()
        return result.runtime_id

    runtime_id = asyncio.run(scenario())

    assert runtime_id is not None
    assert completions == ["completed"]
    assert failures == []
    assert any("hello from runtime" in line for line in logs)
    assert any("training completed successfully" in line for line in logs)
    assert heartbeats["count"] >= 1


def test_local_runtime_adapter_reports_oom_failures(tmp_path) -> None:
    training_file = tmp_path / "train_oom.py"
    training_file.write_text(
        "\n".join(
            [
                "import sys",
                "print('CUDA out of memory', flush=True)",
                "sys.exit(1)",
            ]
        )
        + "\n",
        encoding="utf-8",
    )

    failures: list[RuntimeFailure] = []

    async def on_failure(failure: RuntimeFailure) -> None:
        failures.append(failure)

    async def scenario() -> None:
        adapter = _build_adapter(
            training_file=training_file,
            codebase_root=tmp_path,
            socket_path=tmp_path / "ogd.sock",
            on_failure=on_failure,
        )
        await adapter.start()
        assert adapter._log_task is not None
        await asyncio.wait_for(adapter._log_task, timeout=10)
        await adapter.close()

    asyncio.run(scenario())

    assert len(failures) == 1
    assert failures[0].error_type == "LOCAL_OOM"
    assert failures[0].exit_code == 1
