from __future__ import annotations

import asyncio
import contextlib
import json
import os
import shlex
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import uuid
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


class PrimeRuntimeAdapter:
    runtime: RuntimeType = "prime"

    def __init__(
        self,
        *,
        training_file: Path,
        codebase_root: Path,
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
        self.run_dir = run_dir
        self.training_cmd = training_cmd
        self.runtime_env_overrides = dict(runtime_env_overrides or {})
        self.on_log = on_log
        self.on_failure = on_failure
        self.on_complete = on_complete
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
        self._prime_team_id: str | None = None
        self._resume_checkpoint_dir_remote: str | None = None
        self._last_sync_at: float = 0.0
        self._last_checkpoint_marker: str | None = None
        self._sandbox_tag: str | None = None

    def _prime_workdir(self) -> str:
        return os.getenv("OG_PRIME_WORKDIR", "/workspace")

    def _prime_name(self) -> str:
        if self._sandbox_tag:
            return f"opengraphs-{self._sandbox_tag[:12]}"
        return f"opengraphs-{uuid.uuid4().hex[:12]}"

    def _prime_cpu_cores(self) -> float:
        return float(os.getenv("OG_PRIME_CPU_CORES", "2"))

    def _prime_memory_gb(self) -> float:
        return float(os.getenv("OG_PRIME_MEMORY_GB", "8"))

    def _prime_timeout_minutes(self) -> int:
        return int(os.getenv("OG_PRIME_TIMEOUT_MINUTES", "60"))

    def _prime_teardown_mode(self) -> str:
        mode = os.getenv("OG_PRIME_SANDBOX_TEARDOWN", "cleanup").strip().lower()
        if mode not in {"cleanup", "kill"}:
            return "cleanup"
        return mode

    def _prime_labels(self) -> list[str]:
        labels = ["opengraphs", "runtime:prime"]
        if self._sandbox_tag:
            labels.append(f"run:{self._sandbox_tag[:12]}")
        return labels

    def _poll_interval_secs(self) -> float:
        return float(os.getenv("OG_PRIME_POLL_INTERVAL_SECS", "2"))

    def _max_wait_attempts(self) -> int:
        return int(os.getenv("OG_PRIME_WAIT_ATTEMPTS", "180"))

    def _sync_interval_secs(self) -> float:
        return max(float(os.getenv("OG_PRIME_SYNC_INTERVAL_SECS", "8")), 1.0)

    def _sync_enabled(self) -> bool:
        value = os.getenv("OG_PRIME_SYNC_ENABLE", "1").strip().lower()
        return value not in {"0", "false", "no", "off"}

    def _checkpoint_dir_name(self) -> str:
        raw = os.getenv("OG_PRIME_CHECKPOINT_DIR_NAME", "checkpoints").strip()
        return raw or "checkpoints"

    def _remote_checkpoint_dir(self, workdir: str) -> str:
        custom = os.getenv("OG_PRIME_CHECKPOINT_DIR", "").strip()
        if custom:
            return custom
        return f"{workdir.rstrip('/')}/{self._checkpoint_dir_name()}"

    def _sync_root(self) -> Path | None:
        if self.run_dir is None:
            return None
        root = self.run_dir / "prime_sync"
        root.mkdir(parents=True, exist_ok=True)
        return root

    def _local_checkpoint_dir(self) -> Path | None:
        root = self._sync_root()
        if root is None:
            return None
        path = root / "checkpoints"
        path.mkdir(parents=True, exist_ok=True)
        return path

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
        env.update(self.runtime_env_overrides)
        if self._resume_checkpoint_dir_remote:
            env.setdefault("OG_RESUME_CHECKPOINT_DIR", self._resume_checkpoint_dir_remote)
        return env

    def _prime_auth_config_path(self) -> Path:
        home = Path(os.getenv("HOME") or "~").expanduser()
        return home / ".prime" / "config.json"

    def _resolve_prime_team_id(self) -> str | None:
        env_team = os.getenv("PRIME_TEAM_ID", "").strip()
        if env_team:
            return env_team

        config_path = self._prime_auth_config_path()
        if not config_path.exists():
            return None
        try:
            raw = json.loads(config_path.read_text(encoding="utf-8"))
        except Exception:
            return None

        value = raw.get("team_id")
        if value is None:
            return None
        team_id = str(value).strip()
        return team_id or None

    def _has_prime_auth(self) -> bool:
        api_key = os.getenv("PRIME_API_KEY", "").strip()
        if api_key:
            return True
        return self._prime_auth_config_path().exists()

    def _can_interactive_login(self) -> bool:
        try:
            return sys.stdin.isatty() and sys.stderr.isatty()
        except Exception:
            return False

    def _ensure_prime_auth(self) -> None:
        if self._has_prime_auth():
            return

        message = (
            "Prime auth missing. Run `uvx prime login` (or `uv run prime login`) "
            "or set PRIME_API_KEY."
        )
        if not self._can_interactive_login():
            raise RuntimeError(message)

        attempts = [
            (["uvx", "prime", "login"], "uvx prime login"),
            (["uv", "run", "prime", "login"], "uv run prime login"),
        ]
        for command, label in attempts:
            try:
                self.on_log(f"[system] prime auth missing; running `{label}`")
                result = subprocess.run(command, check=False)
            except FileNotFoundError:
                self.on_log(f"[system] `{label}` unavailable")
                continue
            if result.returncode == 0 and self._has_prime_auth():
                self.on_log(f"[system] prime auth configured via `{label}`")
                return
            self.on_log(
                f"[error] `{label}` exited with code {result.returncode}; auth still missing"
            )

        raise RuntimeError(message)

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
        self._prime_team_id = self._resolve_prime_team_id()

        if self._prime_team_id:
            # Prime team accounts may require explicit team scoping at transport level.
            transport = getattr(getattr(self._client, "client", None), "client", None)
            headers = getattr(transport, "headers", None)
            if headers is not None:
                headers["X-Prime-Team-ID"] = self._prime_team_id
            self.on_log(f"[system] prime team context: {self._prime_team_id}")

    async def _sync_job_logs_to_local(self, sandbox_id: str) -> None:
        if self._client is None or self._job is None:
            return
        root = self._sync_root()
        if root is None:
            return
        logs_dir = root / "logs"
        logs_dir.mkdir(parents=True, exist_ok=True)

        targets = [
            (self._job.stdout_log_file, logs_dir / "stdout.log"),
            (self._job.stderr_log_file, logs_dir / "stderr.log"),
        ]
        for remote_path, local_path in targets:
            try:
                await self._client.download_file(
                    sandbox_id,
                    remote_path,
                    str(local_path),
                )
            except Exception:
                continue

    async def _remote_checkpoint_marker(self, sandbox_id: str, remote_dir: str) -> str | None:
        if self._client is None:
            return None
        quoted = shlex.quote(remote_dir)
        command = (
            "if [ -d {dir} ]; then "
            "find {dir} -type f -printf '%T@ %p\\n' | sort -n | tail -1; "
            "fi"
        ).format(dir=quoted)
        try:
            result = await self._client.execute_command(sandbox_id, command, timeout=20)
        except Exception:
            return None
        marker = (result.stdout or "").strip()
        return marker or None

    async def _sync_remote_checkpoints_to_local(self, sandbox_id: str, remote_dir: str) -> None:
        if self._client is None:
            return
        local_dir = self._local_checkpoint_dir()
        root = self._sync_root()
        if local_dir is None or root is None:
            return

        marker = await self._remote_checkpoint_marker(sandbox_id, remote_dir)
        if marker is None:
            return
        if marker == self._last_checkpoint_marker:
            return

        archive_remote = f"/tmp/og_ckpt_sync_{int(time.time())}.tgz"
        quoted_dir = shlex.quote(remote_dir)
        quoted_archive = shlex.quote(archive_remote)
        create_cmd = (
            "set -e; "
            "[ -d {dir} ] && tar -czf {archive} -C {dir} ."
        ).format(dir=quoted_dir, archive=quoted_archive)
        await self._client.execute_command(sandbox_id, create_cmd, timeout=60)

        local_archive = root / "checkpoints_latest.tgz"
        await self._client.download_file(
            sandbox_id,
            archive_remote,
            str(local_archive),
        )
        try:
            if local_dir.exists():
                shutil.rmtree(local_dir, ignore_errors=True)
                local_dir.mkdir(parents=True, exist_ok=True)
            with tarfile.open(local_archive, "r:gz") as tar:
                tar.extractall(local_dir)
            self._last_checkpoint_marker = marker
            self.on_log(
                f"[system] prime checkpoint sync complete ({remote_dir} -> {local_dir})"
            )
        finally:
            with contextlib.suppress(Exception):
                await self._client.execute_command(
                    sandbox_id,
                    f"rm -f {quoted_archive}",
                    timeout=20,
                )

    async def _maybe_sync_artifacts(self, *, force: bool = False) -> None:
        if not self._sync_enabled():
            return
        if self._client is None or self._sandbox_id is None:
            return
        now = time.time()
        if not force and (now - self._last_sync_at) < self._sync_interval_secs():
            return
        sandbox_id = self._sandbox_id
        await self._sync_job_logs_to_local(sandbox_id)
        if self._resume_checkpoint_dir_remote:
            with contextlib.suppress(Exception):
                await self._sync_remote_checkpoints_to_local(
                    sandbox_id,
                    self._resume_checkpoint_dir_remote,
                )
        self._last_sync_at = now

    async def _restore_checkpoints_to_remote_if_available(
        self,
        sandbox_id: str,
        remote_checkpoint_dir: str,
    ) -> bool:
        if self._client is None or not self._sync_enabled():
            return False
        local_dir = self._local_checkpoint_dir()
        if local_dir is None or not local_dir.exists():
            return False
        has_files = any(p.is_file() for p in local_dir.rglob("*"))
        if not has_files:
            return False

        with tempfile.NamedTemporaryFile(prefix="og_resume_", suffix=".tgz", delete=False) as tmp:
            archive_path = Path(tmp.name)
        try:
            with tarfile.open(archive_path, "w:gz") as tar:
                tar.add(local_dir, arcname=".")
            remote_archive = f"/tmp/{archive_path.name}"
            await self._client.upload_file(
                sandbox_id,
                remote_archive,
                str(archive_path),
            )
            cmd = (
                "set -e; mkdir -p {dir}; tar -xzf {archive} -C {dir}; rm -f {archive}"
            ).format(
                dir=shlex.quote(remote_checkpoint_dir),
                archive=shlex.quote(remote_archive),
            )
            await self._client.execute_command(sandbox_id, cmd, timeout=60)
            self.on_log(
                f"[system] restored checkpoints to prime sandbox: {remote_checkpoint_dir}"
            )
            return True
        finally:
            archive_path.unlink(missing_ok=True)

    def _apply_resume_template_if_configured(
        self,
        command: str,
        remote_checkpoint_dir: str,
        restored: bool,
    ) -> str:
        template = os.getenv("OG_PRIME_RESUME_ARG_TEMPLATE", "").strip()
        if not template or not restored:
            return command
        resume_arg = template.format(checkpoint_dir=remote_checkpoint_dir)
        merged = f"{command} {resume_arg}".strip()
        self.on_log(f"[system] prime resume arg applied: {resume_arg}")
        return merged

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

    async def _delete_sandbox(self, sandbox_id: str, reason: str) -> None:
        if self._client is None:
            return
        try:
            await self._client.delete(sandbox_id)
            self.on_log(f"[system] prime sandbox deleted: {sandbox_id} ({reason})")
        except Exception as exc:
            self.on_log(
                f"[error] failed to delete prime sandbox {sandbox_id}: {exc}"
            )
        finally:
            if self._sandbox_id == sandbox_id:
                self._sandbox_id = None
                self._job = None

    async def _teardown_sandbox(self, sandbox_id: str, reason: str) -> None:
        mode = self._prime_teardown_mode()
        if mode == "cleanup":
            with contextlib.suppress(Exception):
                await self._maybe_sync_artifacts(force=True)
        else:
            self.on_log(
                "[system] prime sandbox teardown mode=kill "
                "(skipping artifact sync before delete)"
            )
        await self._delete_sandbox(sandbox_id, reason)

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
                await self._maybe_sync_artifacts(force=False)
                job_status = await self._client.get_background_job(
                    self._sandbox_id, self._job
                )
                if job_status.completed:
                    await self._append_tail_lines("stdout", job_status.stdout or "")
                    await self._append_tail_lines("stderr", job_status.stderr or "")
                    exit_code = int(job_status.exit_code or 0)
                    self.on_log(f"[system] prime job exited with code {exit_code}")
                    current_sandbox = self._sandbox_id
                    if current_sandbox:
                        await self._teardown_sandbox(current_sandbox, "job completed")
                    if exit_code == 0:
                        self.on_log("[system] prime job completed successfully")
                        await self.on_complete("completed")
                        return
                    combined = f"{job_status.stderr or ''}\n{job_status.stdout or ''}"
                    is_oom = _looks_like_oom_text(combined)
                    await self._notify_failure(
                        status="failed",
                        error_type="PRIME_JOB_OOM" if is_oom else "PRIME_JOB_EXIT_NONZERO",
                        message=(
                            "prime background job OOM detected"
                            if is_oom
                            else "prime background job exited unexpectedly"
                        ),
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
        self._last_sync_at = 0.0
        self._sandbox_tag = uuid.uuid4().hex
        self._ensure_prime_auth()
        self._require_prime()
        assert self._client is not None
        assert self._create_sandbox_request is not None

        request = self._create_sandbox_request(
            name=self._prime_name(),
            docker_image=os.getenv("OG_PRIME_DOCKER_IMAGE", "python:3.11-slim"),
            cpu_cores=self._prime_cpu_cores(),
            memory_gb=self._prime_memory_gb(),
            timeout_minutes=self._prime_timeout_minutes(),
            labels=self._prime_labels(),
            team_id=self._prime_team_id,
        )

        sandbox = await self._client.create(request)
        self._sandbox_id = sandbox.id
        self.on_log(
            f"[system] prime sandbox created: {sandbox.id} "
            f"(tag={self._sandbox_tag[:12] if self._sandbox_tag else 'n/a'})"
        )

        await self._client.wait_for_creation(
            sandbox.id, max_attempts=self._max_wait_attempts()
        )
        self.on_log(f"[system] prime sandbox ready: {sandbox.id}")

        workdir = self._prime_workdir()
        self._resume_checkpoint_dir_remote = self._remote_checkpoint_dir(workdir)
        await self._client.execute_command(
            sandbox.id,
            f"mkdir -p {shlex.quote(workdir)}",
            timeout=20,
        )
        await self._client.execute_command(
            sandbox.id,
            f"mkdir -p {shlex.quote(self._resume_checkpoint_dir_remote)}",
            timeout=20,
        )
        restored = await self._restore_checkpoints_to_remote_if_available(
            sandbox.id,
            self._resume_checkpoint_dir_remote,
        )

        remote_training_path = f"{workdir.rstrip('/')}/{self.training_file.name}"
        await self._client.upload_file(
            sandbox.id, remote_training_path, str(self.training_file)
        )
        self.on_log(
            f"[system] prime uploaded training file to {remote_training_path}"
        )

        command = self._resolve_command(remote_training_path)
        command = self._apply_resume_template_if_configured(
            command,
            self._resume_checkpoint_dir_remote,
            restored=restored,
        )
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

        if self._sandbox_id is not None:
            await self._teardown_sandbox(self._sandbox_id, "runtime stop")

        self._sandbox_id = None
        self._job = None
        self._resume_checkpoint_dir_remote = None
        self._stdout_tail = []
        self._stderr_tail = []
        self._sandbox_tag = None

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
    runtime_env_overrides: dict[str, str] | None,
    on_log: Callable[[str], None],
    on_failure: Callable[[RuntimeFailure], Awaitable[None]],
    on_complete: Callable[[str], Awaitable[None]],
    on_heartbeat: Callable[[], None],
) -> RuntimeAdapter:
    if runtime == "prime":
        return PrimeRuntimeAdapter(
            training_file=training_file,
            codebase_root=codebase_root,
            run_dir=run_dir,
            training_cmd=training_cmd,
            runtime_env_overrides=runtime_env_overrides,
            on_log=on_log,
            on_failure=on_failure,
            on_complete=on_complete,
            on_heartbeat=on_heartbeat,
        )
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
