from __future__ import annotations

import asyncio
import contextlib
import os
import shlex
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Awaitable, Callable, Literal, Protocol

RuntimeType = Literal["local", "modal"]


@dataclass
class RuntimeFailure:
    status: str
    error_type: str | None = None
    message: str | None = None
    exit_code: int | None = None


@dataclass
class RuntimeStartResult:
    runtime_id: str | None = None
    metadata: dict[str, str] = field(default_factory=dict)


class RuntimeAdapter(Protocol):
    runtime: RuntimeType

    async def start(self) -> RuntimeStartResult: ...

    async def stop(self) -> None: ...

    async def close(self) -> None: ...


def _looks_like_oom_text(text: str) -> bool:
    normalized = (text or "").lower()
    return any(
        token in normalized
        for token in (
            "out of memory",
            "cuda out of memory",
            "cublas_status_alloc_failed",
            "memoryerror",
            "killed process",
            "oom",
        )
    )


class LocalRuntimeAdapter:
    runtime: RuntimeType = "local"

    def __init__(
        self,
        *,
        training_file: Path,
        codebase_root: Path,
        socket_path: Path,
        run_dir: Path | None,
        training_cmd: str | None,
        runtime_env_overrides: dict[str, str] | None,
        on_log: Callable[[str], None],
        on_failure: Callable[[RuntimeFailure], Awaitable[None]],
        on_complete: Callable[[str], Awaitable[None]],
        on_heartbeat: Callable[[], None],
    ) -> None:
        self.training_file = training_file
        self.codebase_root = codebase_root
        self.socket_path = socket_path
        self.run_dir = run_dir
        self.training_cmd = training_cmd
        self.runtime_env_overrides = dict(runtime_env_overrides or {})
        self.on_log = on_log
        self.on_failure = on_failure
        self.on_complete = on_complete
        self.on_heartbeat = on_heartbeat
        self._process: asyncio.subprocess.Process | None = None
        self._log_task: asyncio.Task[None] | None = None
        self._stop_requested = False
        self._recent_lines: list[str] = []

    def _build_env(self) -> dict[str, str]:
        env = dict(os.environ)
        env.setdefault("OGD_SOCKET", str(self.socket_path))
        if self.run_dir is not None:
            self.run_dir.mkdir(parents=True, exist_ok=True)
            env.setdefault("TB_LOG_DIR", str(self.run_dir))
        env.setdefault("ENABLE_TB", "1")
        env.update(self.runtime_env_overrides)
        return env

    def _resolve_command(self) -> list[str]:
        if self.training_cmd and self.training_cmd.strip():
            command = shlex.split(self.training_cmd.strip())
            if not command:
                raise RuntimeError("empty training command")
            return command
        return [sys.executable, str(self.training_file)]

    async def _stream_logs(self, process: asyncio.subprocess.Process) -> None:
        if process.stdout is None:
            return
        while True:
            line = await process.stdout.readline()
            if not line:
                break
            text = line.decode("utf-8", errors="replace").rstrip("\n")
            self._recent_lines.append(text)
            if len(self._recent_lines) > 400:
                self._recent_lines = self._recent_lines[-400:]
            self.on_log(text)
            self.on_heartbeat()
        return_code = await process.wait()
        self.on_log(f"[system] training exited with code {return_code}")
        if self._stop_requested:
            return
        if return_code == 0:
            self.on_log("[system] training completed successfully")
            await self.on_complete("completed")
            return
        recent = "\n".join(self._recent_lines[-200:])
        is_oom = _looks_like_oom_text(recent)
        await self.on_failure(
            RuntimeFailure(
                status="failed",
                error_type="LOCAL_OOM" if is_oom else "LOCAL_EXIT_NONZERO",
                message=(
                    "local training process OOM detected"
                    if is_oom
                    else "local training process exited unexpectedly"
                ),
                exit_code=return_code,
            )
        )

    async def start(self) -> RuntimeStartResult:
        await self.stop()
        self._stop_requested = False
        self._recent_lines = []
        command = self._resolve_command()
        process = await asyncio.create_subprocess_exec(
            *command,
            cwd=str(self.codebase_root),
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.STDOUT,
            env=self._build_env(),
        )
        self._process = process
        self.on_log(f"[system] training restarted (pid={process.pid})")
        self.on_log(
            "[system] launch command: " + " ".join(shlex.quote(part) for part in command)
        )
        if self.run_dir is not None:
            self.on_log(f"[system] TB_LOG_DIR={self.run_dir}")
        self.on_heartbeat()
        self._log_task = asyncio.create_task(self._stream_logs(process))
        return RuntimeStartResult(runtime_id=str(process.pid))

    async def stop(self) -> None:
        self._stop_requested = True

        process = self._process
        self._process = None
        if process is not None and process.returncode is None:
            process.terminate()
            try:
                await asyncio.wait_for(process.wait(), timeout=5)
            except TimeoutError:
                process.kill()
                await process.wait()

        current = asyncio.current_task()
        if self._log_task is not None and not self._log_task.done():
            if self._log_task is current:
                self._log_task = None
            else:
                self._log_task.cancel()
                with contextlib.suppress(asyncio.CancelledError):
                    await self._log_task
        self._log_task = None

    async def close(self) -> None:
        await self.stop()


class ModalRuntimeScaffoldAdapter(LocalRuntimeAdapter):
    runtime: RuntimeType = "modal"

    async def start(self) -> RuntimeStartResult:
        self.on_log(
            "[system] modal runtime scaffold active: running local process while "
            "remote Modal adapter is being finalized."
        )
        result = await super().start()
        result.metadata["mode"] = "scaffold-local"
        return result


def build_runtime_adapter(
    *,
    runtime: RuntimeType,
    training_file: Path,
    codebase_root: Path,
    socket_path: Path,
    run_dir: Path | None,
    training_cmd: str | None,
    runtime_env_overrides: dict[str, str] | None,
    on_log: Callable[[str], None],
    on_failure: Callable[[RuntimeFailure], Awaitable[None]],
    on_complete: Callable[[str], Awaitable[None]],
    on_heartbeat: Callable[[], None],
) -> RuntimeAdapter:
    if runtime == "modal":
        return ModalRuntimeScaffoldAdapter(
            training_file=training_file,
            codebase_root=codebase_root,
            socket_path=socket_path,
            run_dir=run_dir,
            training_cmd=training_cmd,
            runtime_env_overrides=runtime_env_overrides,
            on_log=on_log,
            on_failure=on_failure,
            on_complete=on_complete,
            on_heartbeat=on_heartbeat,
        )
    return LocalRuntimeAdapter(
        training_file=training_file,
        codebase_root=codebase_root,
        socket_path=socket_path,
        run_dir=run_dir,
        training_cmd=training_cmd,
        runtime_env_overrides=runtime_env_overrides,
        on_log=on_log,
        on_failure=on_failure,
        on_complete=on_complete,
        on_heartbeat=on_heartbeat,
    )
