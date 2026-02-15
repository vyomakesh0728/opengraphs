from __future__ import annotations

import asyncio
import json
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Iterable

import unidiff

from .codebase import CodebaseIndex
from .config import dspy, ensure_dspy_configured
from .models import ActionPlan, Alert, ChatMessage, ExecutionResult, RunState

SYSTEM_PROMPT = """
You are an ML training assistant for OpenGraphs.
Role: Diagnose issues and suggest safe code fixes when metrics plateau/degrade.

Operating policy:
1. Prefer small, reversible refactors over large rewrites.
2. Use logs + metric trend direction, not a single noisy point.
3. If recent refactors are not improving the target metric, stop proposing further code edits.
   In that case, set ACTION: explain and provide concrete checks (data, LR schedule, optimizer state,
   grad clipping, batch size, seed, and hardware/resource bottlenecks).
4. Never invent files or paths. Only modify the provided training script unless explicitly asked.
5. For refactors, produce a syntactically valid unified diff that can be applied directly.
   Do not wrap the diff in markdown fences.
6. For demo_train.py stabilization, prefer changing all three knobs together:
   LEARNING_RATE -> 0.001, WARMUP_STEPS -> 20, PEAK_LR_MULT -> 1.0.

Response format:
DIAGNOSIS: <analysis of the problem>
ACTION: <explain|refactor>
CODE_CHANGES: <if refactor, provide unified diff starting with --- and +++ for one file>
""".strip()

EDITOR_QUERY_TEMPLATE = """
You are editing the training script for OpenGraphs.
Provide ONLY a strict unified diff (---/+++ headers) for the requested fix.
If no code change is required, return an empty string.
Rules:
- Output raw diff text only (no markdown fences, no commentary).
- Target exactly one file: the training script path below.
- Keep hunks minimal and preserve surrounding context lines.

Alert:
{alert_block}

Diagnosis:
{diagnosis}

Training script path:
{training_path}
""".strip()


@dataclass
class AgentResponse:
    raw_output: str
    plan: ActionPlan


class ContextBuilder:
    def __init__(self, system_prompt: str = SYSTEM_PROMPT) -> None:
        self.system_prompt = system_prompt

    def build_context(
        self,
        run_state: RunState,
        codebase_index: CodebaseIndex,
        alert: Alert | None = None,
    ) -> str:
        alert = alert or run_state.latest_alert()
        alert_block = "No active alert."
        if alert:
            alert_block = (
                "metric={metric}\nthreshold={threshold}\ncurrent={current}\nmessage={message}"
            ).format(
                metric=alert.metric,
                threshold=alert.threshold,
                current=alert.current,
                message=alert.message,
            )

        metrics_block = []
        for metric_name, values in run_state.metrics.items():
            tail = values[-20:]
            metrics_block.append(f"{metric_name}: {tail}")
        metrics_text = "\n".join(metrics_block) if metrics_block else "No metrics yet."

        logs_text = run_state.log_tail(50) or "No logs yet."

        training_text = ""
        try:
            training_text = run_state.training_file.read_text(encoding="utf-8")
        except OSError:
            training_text = "<unable to read training file>"

        codebase_listing = codebase_index.file_listing(limit=120)

        return (
            f"{self.system_prompt}\n\n"
            f"ALERT:\n{alert_block}\n\n"
            f"RECENT_METRICS:\n{metrics_text}\n\n"
            f"LOG_TAIL:\n{logs_text}\n\n"
            f"TRAINING_SCRIPT ({run_state.training_file}):\n{training_text}\n\n"
            f"CODEBASE_FILES:\n{codebase_listing}\n"
        )


class ActionPlanner:
    def parse_response(self, llm_output: str) -> ActionPlan:
        diagnosis = self._extract_section(llm_output, "DIAGNOSIS")
        action_raw = self._extract_section(llm_output, "ACTION").lower()
        code_changes = self._extract_section(llm_output, "CODE_CHANGES")

        action = "refactor" if "refactor" in action_raw else "explain"
        if action == "explain":
            code_changes = ""

        return ActionPlan(
            diagnosis=diagnosis or llm_output.strip(),
            action=action,
            code_changes=code_changes.strip(),
            raw_output=llm_output,
        )

    @staticmethod
    def _extract_section(text: str, label: str) -> str:
        import re

        pattern = re.compile(
            rf"{label}\s*:\s*(.*?)(?=\n[A-Z_]+\s*:|$)", re.DOTALL
        )
        match = pattern.search(text)
        if not match:
            return ""
        return match.group(1).strip()


class CodebaseExplorer:
    def __init__(
        self,
        codebase_index: CodebaseIndex,
        max_iterations: int = 30,
        max_llm_calls: int = 80,
        max_output_chars: int = 12000,
    ) -> None:
        ensure_dspy_configured()
        self.codebase_index = codebase_index
        self.context = codebase_index.build_context()
        self.rlm = dspy.RLM(
            "context, query -> answer",
            max_iterations=max_iterations,
            max_llm_calls=max_llm_calls,
            max_output_chars=max_output_chars,
        )

    def query(self, query: str) -> str:
        result = self.rlm(context=self.context, query=query)
        return result.answer


class ToolCaller:
    class Signature(dspy.Signature):
        """Decide when to call tools and answer with DIAGNOSIS/ACTION/CODE_CHANGES."""

        context: str = dspy.InputField(desc="System and run-state context.")
        question: str = dspy.InputField(desc="Alert or user question to address.")
        answer: str = dspy.OutputField(
            desc="Answer using DIAGNOSIS/ACTION/CODE_CHANGES sections."
        )

    def __init__(
        self,
        run_state: RunState,
        codebase_index: CodebaseIndex,
        explorer: CodebaseExplorer,
        max_iters: int = 30,
    ) -> None:
        ensure_dspy_configured()
        self.run_state = run_state
        self.codebase_index = codebase_index
        self.explorer = explorer
        self.react = dspy.ReAct(
            signature=ToolCaller.Signature,
            tools=self._build_tools(),
            max_iters=max_iters,
        )

    def _build_tools(self) -> list[Callable]:
        def alert_summary() -> str:
            """Return the latest alert details."""
            alert = self.run_state.latest_alert()
            if not alert:
                return "No active alert."
            return (
                f"metric={alert.metric}, threshold={alert.threshold}, "
                f"current={alert.current}, message={alert.message}"
            )

        def list_metrics() -> str:
            """List available metric names."""
            names = list(self.run_state.metrics.keys())
            return "\n".join(names) if names else "No metrics available."

        def metric_tail(metric: str, n: int = 20) -> str:
            """Return the last n values for a metric."""
            tail = self.run_state.metric_tail(metric, n=n)
            return f"{metric}: {tail}"

        def log_tail(n: int = 50) -> str:
            """Return the last n log lines."""
            return self.run_state.log_tail(n)

        def read_training_file() -> str:
            """Read the training script content."""
            try:
                return self.run_state.training_file.read_text(encoding="utf-8")
            except OSError:
                return "<unable to read training file>"

        def list_codebase_files(limit: int = 120) -> str:
            """Return a listing of codebase files."""
            return self.codebase_index.file_listing(limit=limit)

        def search_codebase(pattern: str, max_matches: int = 20) -> str:
            """Regex search across indexed files."""
            matches = self.codebase_index.search_regex(pattern, max_matches=max_matches)
            return "\n".join(matches) if matches else "No matches found."

        def explore_codebase(query: str) -> str:
            """Use RLM to search and summarize the codebase."""
            return self.explorer.query(query)

        return [
            alert_summary,
            list_metrics,
            metric_tail,
            log_tail,
            read_training_file,
            list_codebase_files,
            search_codebase,
            explore_codebase,
        ]

    def run(self, context: str, question: str) -> str:
        prediction = self.react(context=context, question=question)
        return prediction.answer


class CodeEditor:
    def __init__(
        self,
        codebase_index: CodebaseIndex,
        max_iterations: int = 30,
        max_llm_calls: int = 80,
        max_output_chars: int = 12000,
    ) -> None:
        ensure_dspy_configured()
        self.codebase_index = codebase_index
        self.rlm = dspy.RLM(
            "context, query -> answer",
            max_iterations=max_iterations,
            max_llm_calls=max_llm_calls,
            max_output_chars=max_output_chars,
        )

    def propose_diff(
        self,
        run_state: RunState,
        diagnosis: str,
        alert: Alert | None = None,
    ) -> str:
        alert_block = "No active alert."
        if alert:
            alert_block = (
                "metric={metric}\nthreshold={threshold}\ncurrent={current}\nmessage={message}"
            ).format(
                metric=alert.metric,
                threshold=alert.threshold,
                current=alert.current,
                message=alert.message,
            )

        training_text = ""
        try:
            training_text = run_state.training_file.read_text(encoding="utf-8")
        except OSError:
            training_text = "<unable to read training file>"

        context = (
            f"TRAINING_SCRIPT:\n{training_text}\n\n"
            f"CODEBASE:\n{self.codebase_index.build_context()}"
        )
        query = EDITOR_QUERY_TEMPLATE.format(
            alert_block=alert_block,
            diagnosis=diagnosis,
            training_path=run_state.training_file,
        )
        result = self.rlm(context=context, query=query)
        return result.answer


class GuardedExecutor:
    def __init__(
        self,
        auto_mode: bool,
        checkpoint_dir: Path | None = None,
        restart_callback: Callable[[RunState], object] | None = None,
    ) -> None:
        self.auto_mode = auto_mode
        self.checkpoint_dir = checkpoint_dir or Path(".og_checkpoints")
        self.restart_callback = restart_callback

    async def execute(self, plan: ActionPlan, run_state: RunState) -> ExecutionResult:
        checkpoint_id = create_checkpoint(run_state, self.checkpoint_dir)
        if plan.action != "refactor":
            return ExecutionResult(success=True, checkpoint_id=checkpoint_id)
        if not self.auto_mode:
            return ExecutionResult(
                success=False,
                checkpoint_id=checkpoint_id,
                error="Auto mode disabled.",
            )
        if not plan.code_changes:
            return ExecutionResult(
                success=False,
                checkpoint_id=checkpoint_id,
                error="No code changes provided.",
            )

        try:
            apply_diff(run_state.training_file, plan.code_changes)
            if self.restart_callback:
                result = self.restart_callback(run_state)
                if asyncio.iscoroutine(result):
                    await result
            return ExecutionResult(success=True, checkpoint_id=checkpoint_id)
        except Exception as exc:  # pragma: no cover - defensive guard
            restore_checkpoint(checkpoint_id, run_state, self.checkpoint_dir)
            return ExecutionResult(
                success=False,
                checkpoint_id=checkpoint_id,
                error=str(exc),
            )


class AgentEngine:
    def __init__(
        self,
        run_state: RunState,
        codebase_root: Path,
        auto_mode: bool = False,
        max_tool_iters: int = 30,
        max_rlm_iters: int = 30,
        restart_callback: Callable[[RunState], object] | None = None,
    ) -> None:
        self.run_state = run_state
        self.codebase_index = CodebaseIndex.from_root(codebase_root)
        self.context_builder = ContextBuilder()
        self.action_planner = ActionPlanner()
        self.explorer = CodebaseExplorer(
            self.codebase_index,
            max_iterations=max_rlm_iters,
        )
        self.tool_caller = ToolCaller(
            run_state,
            self.codebase_index,
            self.explorer,
            max_iters=max_tool_iters,
        )
        self.editor = CodeEditor(self.codebase_index, max_iterations=max_rlm_iters)
        self.executor = GuardedExecutor(
            auto_mode=auto_mode,
            restart_callback=restart_callback,
        )
        self.chat_messages: list[ChatMessage] = []

    def add_chat_message(self, sender: str, content: str) -> None:
        self.chat_messages.append(
            ChatMessage(sender=sender, content=content, timestamp=time.time())
        )

    def get_chat_history(self) -> list[ChatMessage]:
        return list(self.chat_messages)

    async def handle_chat_message(self, user_message: str) -> AgentResponse:
        self.add_chat_message("user", user_message)
        response = await self._respond(question=user_message)
        return response

    async def handle_alert(self, alert: Alert | None = None) -> AgentResponse | None:
        alert = alert or self.run_state.latest_alert()
        if not alert:
            return None
        self.add_chat_message(
            "system",
            (
                "Alert triggered: "
                f"metric={alert.metric}, threshold={alert.threshold}, "
                f"current={alert.current}, message={alert.message}"
            ),
        )
        question = (
            "Alert triggered: "
            f"metric={alert.metric}, threshold={alert.threshold}, "
            f"current={alert.current}, message={alert.message}"
        )
        return await self._respond(question=question, alert=alert)

    async def _respond(
        self,
        question: str,
        alert: Alert | None = None,
    ) -> AgentResponse:
        context = self.context_builder.build_context(
            self.run_state,
            self.codebase_index,
            alert=alert,
        )
        raw = ""
        plan: ActionPlan
        try:
            raw = self.tool_caller.run(context=context, question=question)
            plan = self.action_planner.parse_response(raw)
            if plan.action == "refactor" and not plan.code_changes:
                diff = self.editor.propose_diff(self.run_state, plan.diagnosis, alert=alert)
                plan = ActionPlan(
                    diagnosis=plan.diagnosis,
                    action=plan.action,
                    code_changes=diff.strip(),
                    raw_output=raw,
                )
        except Exception as exc:
            raw = f"[fallback] agent unavailable: {exc}"
            plan = ActionPlan(
                diagnosis=f"Agent unavailable: {type(exc).__name__}: {exc}",
                action="explain",
                code_changes="",
                raw_output=raw,
            )
        self.add_chat_message("agent", plan.diagnosis)
        if plan.action == "refactor" and plan.code_changes:
            self.add_chat_message(
                "agent",
                "Proposed refactor diff:\n" + plan.code_changes,
            )
            summary = self._summarize_diff_changes(plan.code_changes)
            if summary:
                self.add_chat_message("system", summary)
        if plan.action == "refactor" and self.executor.auto_mode:
            result = await self.executor.execute(plan, self.run_state)
            if result.success:
                self.add_chat_message(
                    "system",
                    f"Code refactored from checkpoint {result.checkpoint_id}.",
                )
            else:
                self.add_chat_message(
                    "system",
                    f"Refactor failed: {result.error}. Rolled back.",
                )
        return AgentResponse(raw_output=raw, plan=plan)

    @staticmethod
    def _summarize_diff_changes(diff_text: str) -> str:
        assignment_re = re.compile(r"^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.+?)\s*$")
        removed: dict[str, str] = {}
        added: dict[str, str] = {}

        for raw_line in diff_text.splitlines():
            if raw_line.startswith("---") or raw_line.startswith("+++") or raw_line.startswith("@@"):
                continue
            if raw_line.startswith("-"):
                line = raw_line[1:]
                m = assignment_re.match(line)
                if m:
                    removed[m.group(1)] = m.group(2)
            elif raw_line.startswith("+"):
                line = raw_line[1:]
                m = assignment_re.match(line)
                if m:
                    added[m.group(1)] = m.group(2)

        changes: list[str] = []
        for key in sorted(set(removed.keys()) | set(added.keys())):
            before = removed.get(key)
            after = added.get(key)
            if before and after and before != after:
                changes.append(f"{key}: {before} -> {after}")
            elif after and not before:
                changes.append(f"{key}: set to {after}")
            elif before and not after:
                changes.append(f"{key}: removed")

        if not changes:
            return ""
        shown = changes[:5]
        suffix = " ..." if len(changes) > len(shown) else ""
        return "Refactor summary: " + " | ".join(shown) + suffix

    async def execute_plan(self, plan: ActionPlan) -> ExecutionResult:
        """Execute a refactor plan (called from TUI approve, bypasses auto_mode)."""
        if plan.action != "refactor" or not plan.code_changes:
            return ExecutionResult(success=False, error="No refactor to apply.")
        checkpoint_id = create_checkpoint(self.run_state, self.executor.checkpoint_dir)
        try:
            apply_diff(self.run_state.training_file, plan.code_changes)
            if self.executor.restart_callback:
                result = self.executor.restart_callback(self.run_state)
                if asyncio.iscoroutine(result):
                    await result
            self.add_chat_message(
                "system",
                f"Code refactored from checkpoint {checkpoint_id}.",
            )
            return ExecutionResult(success=True, checkpoint_id=checkpoint_id)
        except Exception as exc:
            restore_checkpoint(checkpoint_id, self.run_state, self.executor.checkpoint_dir)
            self.add_chat_message(
                "system",
                f"Refactor failed: {exc}. Rolled back.",
            )
            return ExecutionResult(
                success=False, checkpoint_id=checkpoint_id, error=str(exc)
            )


async def agent_loop(
    run_state: RunState,
    codebase_root: Path,
    auto_mode: bool = False,
    poll_interval: float = 5.0,
) -> None:
    engine = AgentEngine(
        run_state=run_state,
        codebase_root=codebase_root,
        auto_mode=auto_mode,
    )
    last_alert_timestamp = 0.0
    while run_state.is_active:
        alert = run_state.latest_alert()
        if alert and alert.timestamp > last_alert_timestamp:
            last_alert_timestamp = alert.timestamp
            await engine.handle_alert(alert)
        await asyncio.sleep(poll_interval)


def create_checkpoint(run_state: RunState, checkpoint_dir: Path) -> str:
    checkpoint_id = f"ckpt_{int(time.time())}"
    ckpt_path = checkpoint_dir / checkpoint_id
    ckpt_path.mkdir(parents=True, exist_ok=True)
    training_dest = ckpt_path / run_state.training_file.name
    training_dest.write_text(
        run_state.training_file.read_text(encoding="utf-8"), encoding="utf-8"
    )
    state_path = ckpt_path / "state.json"
    state_payload = {
        "metrics": run_state.metrics,
        "step": run_state.current_step,
    }
    state_path.write_text(json.dumps(state_payload, indent=2), encoding="utf-8")
    return checkpoint_id


def restore_checkpoint(checkpoint_id: str, run_state: RunState, checkpoint_dir: Path) -> None:
    ckpt_path = checkpoint_dir / checkpoint_id
    training_source = ckpt_path / run_state.training_file.name
    run_state.training_file.write_text(
        training_source.read_text(encoding="utf-8"), encoding="utf-8"
    )


def apply_diff(filepath: Path, diff_text: str) -> None:
    normalized_diff = _normalize_diff_text(diff_text)
    patch = unidiff.PatchSet(normalized_diff)
    if not patch:
        raise ValueError("Empty diff provided.")

    target_patches = [
        patch_file
        for patch_file in patch
        if _patch_file_targets_filepath(filepath, patch_file)
    ]
    if not target_patches:
        raise ValueError("Diff does not target the training file.")
    if len(target_patches) > 1:
        raise ValueError("Diff targets the training file multiple times.")

    patch_file = target_patches[0]

    original_lines = filepath.read_text(encoding="utf-8").splitlines(keepends=True)
    updated_lines = _apply_patch_hunks(original_lines, patch_file)

    tmp_path = filepath.with_suffix(filepath.suffix + ".tmp")
    tmp_path.write_text("".join(updated_lines), encoding="utf-8")
    tmp_path.replace(filepath)


def _normalize_diff_text(diff_text: str) -> str:
    lines = diff_text.strip().splitlines()
    if not lines:
        return ""

    # LLMs often wrap diffs in fenced code blocks.
    lines = [line for line in lines if not line.strip().startswith("```")]

    start_index = 0
    for idx, line in enumerate(lines):
        if line.startswith("diff --git ") or line.startswith("--- "):
            start_index = idx
            break

    normalized = "\n".join(lines[start_index:]).strip()
    if normalized:
        normalized += "\n"
    return normalized


def _normalize_patch_path(patch_path: str | None) -> str:
    normalized = (patch_path or "").strip().strip('"').strip("'")
    if not normalized:
        return ""
    if normalized.startswith("file://"):
        normalized = normalized[len("file://") :]
    normalized = normalized.replace("\\", "/")
    if "\t" in normalized:
        normalized = normalized.split("\t", 1)[0]
    while normalized.startswith("a/") or normalized.startswith("b/"):
        normalized = normalized[2:]
    while normalized.startswith("./"):
        normalized = normalized[2:]
    if normalized == "/dev/null":
        return ""
    return normalized


def _patch_file_targets_filepath(
    filepath: Path,
    patch_file: unidiff.PatchedFile,
) -> bool:
    for patch_path in [patch_file.path, patch_file.source_file, patch_file.target_file]:
        if _patch_targets_file(filepath, patch_path):
            return True
    return False


def _patch_targets_file(filepath: Path, patch_path: str | None) -> bool:
    normalized = _normalize_patch_path(patch_path)
    if not normalized:
        return False

    file_path = filepath.as_posix().replace("\\", "/")
    resolved_path = filepath.resolve().as_posix().replace("\\", "/")

    if normalized == filepath.name:
        return True
    if normalized == file_path or normalized == resolved_path:
        return True
    if file_path.endswith("/" + normalized):
        return True
    if resolved_path.endswith("/" + normalized):
        return True

    return False


def _apply_patch_hunks(original_lines: list[str], patch_file: unidiff.PatchedFile) -> list[str]:
    result: list[str] = []
    src_index = 0

    for hunk in patch_file:
        hunk_start = max(hunk.source_start - 1, 0)
        if hunk_start < src_index:
            raise ValueError("Overlapping hunks detected.")

        result.extend(original_lines[src_index:hunk_start])
        src_index = hunk_start

        for line in hunk:
            if line.is_context:
                if src_index >= len(original_lines):
                    raise ValueError("Patch context exceeds file length.")
                original = original_lines[src_index]
                if original.rstrip("\n") != line.value.rstrip("\n"):
                    raise ValueError("Patch context does not match file.")
                result.append(original)
                src_index += 1
            elif line.is_removed:
                if src_index >= len(original_lines):
                    raise ValueError("Patch removal exceeds file length.")
                original = original_lines[src_index]
                if original.rstrip("\n") != line.value.rstrip("\n"):
                    raise ValueError("Patch removal does not match file.")
                src_index += 1
            elif line.is_added:
                result.append(line.value)

    result.extend(original_lines[src_index:])
    return result
