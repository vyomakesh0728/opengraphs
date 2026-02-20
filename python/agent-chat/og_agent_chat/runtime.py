from __future__ import annotations

import asyncio
import contextlib
import os
import shlex
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Awaitable, Callable, Literal, Protocol

RuntimeType = Literal["local", "prime", "modal"]


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


def _tail_overlap(previous: list[str], current: list[str]) -> int:
    max_overlap = min(len(previous), len(current))
    for overlap in range(max_overlap, -1, -1):
        if previous[len(previous) - overlap :] == current[:overlap]:
            return overlap
    return 0


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
        on_log: Callable[[str], None],
        on_failure: Callable[[RuntimeFailure], Awaitable[None]],
        on_heartbeat: Callable[[], None],
    ) -> None:
        self.training_file = training_file
        self.codebase_root = codebase_root
        self.socket_path = socket_path
        self.run_dir = run_dir
        self.training_cmd = training_cmd
        self.on_log = on_log
        self.on_failure = on_failure
        self.on_heartbeat = on_heartbeat
        self._process: asyncio.subprocess.Process | None = None
        self._log_task: asyncio.Task[None] | None = None
        self._stop_requested = False

    def _build_env(self) -> dict[str, str]:
        env = dict(os.environ)
        env.setdefault("OGD_SOCKET", str(self.socket_path))
        if self.run_dir is not None:
            self.run_dir.mkdir(parents=True, exist_ok=True)
            env.setdefault("TB_LOG_DIR", str(self.run_dir))
        env.setdefault("ENABLE_TB", "1")
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
            self.on_log(line.decode("utf-8", errors="replace").rstrip("\n"))
            self.on_heartbeat()
        return_code = await process.wait()
        self.on_log(f"[system] training exited with code {return_code}")
        if self._stop_requested:
            return
        if return_code == 0:
            self.on_log("[system] training completed successfully")
            return
        await self.on_failure(
            RuntimeFailure(
                status="failed",
                error_type="LOCAL_EXIT_NONZERO",
                message="local training process exited unexpectedly",
                exit_code=return_code,
            )
        )

    async def start(self) -> RuntimeStartResult:
        await self.stop()
        self._stop_requested = False
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


class PrimeRuntimeAdapter:
    runtime: RuntimeType = "prime"

    def __init__(
        self,
        *,
        training_file: Path,
        codebase_root: Path,
        training_cmd: str | None,
        on_log: Callable[[str], None],
        on_failure: Callable[[RuntimeFailure], Awaitable[None]],
        on_heartbeat: Callable[[], None],
    ) -> None:
        self.training_file = training_file
        self.codebase_root = codebase_root
        self.training_cmd = training_cmd
        self.on_log = on_log
        self.on_failure = on_failure
        self.on_heartbeat = on_heartbeat

        self._client = None
        self._create_sandbox_request = None
        self._sandbox_id: str | None = None
        self._job = None
        self._monitor_task: asyncio.Task[None] | None = None
        self._stop_requested = False
        self._stdout_tail: list[str] = []
        self._stderr_tail: list[str] = []
        self._monitor_errors = 0

    def _prime_workdir(self) -> str:
        return os.getenv("OG_PRIME_WORKDIR", "/workspace")

    def _prime_name(self) -> str:
        return f"opengraphs-{int(time.time())}"

    def _prime_cpu_cores(self) -> float:
        return float(os.getenv("OG_PRIME_CPU_CORES", "2"))

    def _prime_memory_gb(self) -> float:
        return float(os.getenv("OG_PRIME_MEMORY_GB", "8"))

    def _prime_timeout_minutes(self) -> int:
        return int(os.getenv("OG_PRIME_TIMEOUT_MINUTES", "180"))

    def _poll_interval_secs(self) -> float:
        return float(os.getenv("OG_PRIME_POLL_INTERVAL_SECS", "2"))

    def _max_wait_attempts(self) -> int:
        return int(os.getenv("OG_PRIME_WAIT_ATTEMPTS", "180"))

    def _resolve_command(self, remote_training_path: str) -> str:
        if self.training_cmd and self.training_cmd.strip():
            command = self.training_cmd.strip()
            local_path = str(self.training_file)
            if local_path in command:
                command = command.replace(local_path, self.training_file.name)
            return command
        python_bin = os.getenv("OG_PRIME_PYTHON_BIN", "python")
        return f"{python_bin} {shlex.quote(Path(remote_training_path).name)}"

    def _runtime_env(self) -> dict[str, str]:
        env: dict[str, str] = {}
        passthrough = os.getenv("OG_PRIME_ENV_PASSTHROUGH", "")
        for key in [k.strip() for k in passthrough.split(",") if k.strip()]:
            value = os.getenv(key)
            if value is not None:
                env[key] = value
        return env

    def _require_prime(self) -> None:
        if self._client is not None:
            return
        try:
            from prime_sandboxes import AsyncSandboxClient, CreateSandboxRequest
        except Exception as exc:
            raise RuntimeError(
                "Prime runtime requires 'prime-sandboxes'. Install with: "
                "uv pip install prime-sandboxes"
            ) from exc

        self._client = AsyncSandboxClient()
        self._create_sandbox_request = CreateSandboxRequest

    async def _append_tail_lines(self, stream_name: str, text: str) -> None:
        lines = [line for line in text.splitlines() if line.strip()]
        if stream_name == "stdout":
            overlap = _tail_overlap(self._stdout_tail, lines)
            fresh = lines[overlap:]
            self._stdout_tail = lines
        else:
            overlap = _tail_overlap(self._stderr_tail, lines)
            fresh = lines[overlap:]
            self._stderr_tail = lines

        for line in fresh:
            if stream_name == "stdout":
                self.on_log(f"[prime] {line}")
            else:
                self.on_log(f"[prime][stderr] {line}")
            self.on_heartbeat()

    async def _tail_job_logs(self) -> None:
        if self._client is None or self._sandbox_id is None or self._job is None:
            return
        stdout_path = shlex.quote(self._job.stdout_log_file)
        stderr_path = shlex.quote(self._job.stderr_log_file)
        stdout_resp = await self._client.execute_command(
            self._sandbox_id,
            f"tail -n 200 {stdout_path} 2>/dev/null || true",
            timeout=20,
        )
        stderr_resp = await self._client.execute_command(
            self._sandbox_id,
            f"tail -n 200 {stderr_path} 2>/dev/null || true",
            timeout=20,
        )
        await self._append_tail_lines("stdout", stdout_resp.stdout)
        await self._append_tail_lines("stderr", stderr_resp.stdout)

    async def _notify_failure(
        self,
        *,
        status: str,
        error_type: str | None = None,
        message: str | None = None,
        exit_code: int | None = None,
    ) -> None:
        if self._stop_requested:
            return
        await self.on_failure(
            RuntimeFailure(
                status=status,
                error_type=error_type,
                message=message,
                exit_code=exit_code,
            )
        )

    async def _monitor_loop(self) -> None:
        assert self._client is not None
        while not self._stop_requested:
            try:
                if self._sandbox_id is None or self._job is None:
                    return

                sandbox = await self._client.get(self._sandbox_id)
                self.on_heartbeat()
                status = (sandbox.status or "").upper()
                if status in {"ERROR", "TERMINATED", "TIMEOUT", "STOPPED"}:
                    await self._notify_failure(
                        status=status.lower(),
                        error_type=sandbox.error_type,
                        message=sandbox.error_message
                        or f"prime sandbox status changed to {status}",
                        exit_code=sandbox.exit_code,
                    )
                    return

                await self._tail_job_logs()
                job_status = await self._client.get_background_job(
                    self._sandbox_id, self._job
                )
                if job_status.completed:
                    await self._append_tail_lines("stdout", job_status.stdout or "")
                    await self._append_tail_lines("stderr", job_status.stderr or "")
                    exit_code = int(job_status.exit_code or 0)
                    self.on_log(f"[system] prime job exited with code {exit_code}")
                    if exit_code == 0:
                        self.on_log("[system] prime job completed successfully")
                        return
                    await self._notify_failure(
                        status="failed",
                        error_type="PRIME_JOB_EXIT_NONZERO",
                        message="prime background job exited unexpectedly",
                        exit_code=exit_code,
                    )
                    return

                self._monitor_errors = 0
                await asyncio.sleep(self._poll_interval_secs())
            except asyncio.CancelledError:
                raise
            except Exception as exc:
                if self._stop_requested:
                    return
                self._monitor_errors += 1
                self.on_log(f"[error] prime monitor error: {exc}")
                if self._monitor_errors >= 3:
                    await self._notify_failure(
                        status="error",
                        error_type=type(exc).__name__,
                        message=str(exc),
                    )
                    return
                await asyncio.sleep(min(self._poll_interval_secs() * self._monitor_errors, 10.0))

    async def start(self) -> RuntimeStartResult:
        await self.stop()
        self._stop_requested = False
        self._monitor_errors = 0
        self._stdout_tail = []
        self._stderr_tail = []
        self._require_prime()
        assert self._client is not None
        assert self._create_sandbox_request is not None

        request = self._create_sandbox_request(
            name=self._prime_name(),
            docker_image=os.getenv("OG_PRIME_DOCKER_IMAGE", "python:3.11-slim"),
            cpu_cores=self._prime_cpu_cores(),
            memory_gb=self._prime_memory_gb(),
            timeout_minutes=self._prime_timeout_minutes(),
            labels=["opengraphs", "runtime:prime"],
        )

        sandbox = await self._client.create(request)
        self._sandbox_id = sandbox.id
        self.on_log(f"[system] prime sandbox created: {sandbox.id}")

        await self._client.wait_for_creation(
            sandbox.id, max_attempts=self._max_wait_attempts()
        )
        self.on_log(f"[system] prime sandbox ready: {sandbox.id}")

        workdir = self._prime_workdir()
        await self._client.execute_command(
            sandbox.id,
            f"mkdir -p {shlex.quote(workdir)}",
            timeout=20,
        )

        remote_training_path = f"{workdir.rstrip('/')}/{self.training_file.name}"
        await self._client.upload_file(
            sandbox.id, remote_training_path, str(self.training_file)
        )
        self.on_log(
            f"[system] prime uploaded training file to {remote_training_path}"
        )

        command = self._resolve_command(remote_training_path)
        self.on_log(f"[system] prime launch command: {command}")
        self._job = await self._client.start_background_job(
            sandbox.id,
            command,
            working_dir=workdir,
            env=self._runtime_env(),
        )
        self.on_log(
            f"[system] prime background job started: {self._job.job_id} (sandbox={sandbox.id})"
        )
        self.on_heartbeat()
        self._monitor_task = asyncio.create_task(self._monitor_loop())
        return RuntimeStartResult(runtime_id=sandbox.id)

    async def stop(self) -> None:
        self._stop_requested = True

        current = asyncio.current_task()
        if self._monitor_task is not None and not self._monitor_task.done():
            if self._monitor_task is current:
                self._monitor_task = None
            else:
                self._monitor_task.cancel()
                with contextlib.suppress(asyncio.CancelledError):
                    await self._monitor_task
        self._monitor_task = None

        if self._client is not None and self._sandbox_id is not None:
            sandbox_id = self._sandbox_id
            try:
                await self._client.delete(sandbox_id)
                self.on_log(f"[system] prime sandbox deleted: {sandbox_id}")
            except Exception as exc:
                self.on_log(
                    f"[error] failed to delete prime sandbox {sandbox_id}: {exc}"
                )

        self._sandbox_id = None
        self._job = None
        self._stdout_tail = []
        self._stderr_tail = []

    async def close(self) -> None:
        await self.stop()
        if self._client is not None:
            with contextlib.suppress(Exception):
                await self._client.aclose()
        self._client = None


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
    on_log: Callable[[str], None],
    on_failure: Callable[[RuntimeFailure], Awaitable[None]],
    on_heartbeat: Callable[[], None],
) -> RuntimeAdapter:
    if runtime == "prime":
        return PrimeRuntimeAdapter(
            training_file=training_file,
            codebase_root=codebase_root,
            training_cmd=training_cmd,
            on_log=on_log,
            on_failure=on_failure,
            on_heartbeat=on_heartbeat,
        )
    if runtime == "modal":
        return ModalRuntimeScaffoldAdapter(
            training_file=training_file,
            codebase_root=codebase_root,
            socket_path=socket_path,
            run_dir=run_dir,
            training_cmd=training_cmd,
            on_log=on_log,
            on_failure=on_failure,
            on_heartbeat=on_heartbeat,
        )
    return LocalRuntimeAdapter(
        training_file=training_file,
        codebase_root=codebase_root,
        socket_path=socket_path,
        run_dir=run_dir,
        training_cmd=training_cmd,
        on_log=on_log,
        on_failure=on_failure,
        on_heartbeat=on_heartbeat,
    )
