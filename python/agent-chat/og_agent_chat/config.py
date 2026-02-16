from __future__ import annotations

import os
from pathlib import Path
from typing import Any, Optional


def _ensure_deno_dir() -> None:
    tmpdir = os.getenv("TMPDIR") or os.getenv("TEMP") or os.getenv("TMP") or "/tmp"
    deno_dir = Path(os.getenv("DENO_DIR") or (Path(tmpdir) / "opengraphs-deno-cache"))
    deno_dir.mkdir(parents=True, exist_ok=True)
    os.environ.setdefault("DENO_DIR", str(deno_dir))


_DEFAULT_MODEL = "openai/gpt-5.2-codex"
_configured_lm: Optional[Any] = None
_dspy_module: Any | None = None


def get_dspy() -> Any:
    global _dspy_module
    if _dspy_module is None:
        _ensure_deno_dir()
        import dspy as _dspy

        _dspy_module = _dspy
    return _dspy_module


def _sanitize_api_key(value: str | None) -> str | None:
    if value is None:
        return None
    cleaned = value.strip().strip('"').strip("'")
    if cleaned.lower().startswith("bearer "):
        cleaned = cleaned[7:].strip()
    cleaned = cleaned.replace("\r", "").replace("\n", "")
    if not cleaned:
        return None
    return cleaned


def ensure_dspy_configured(
    model: str | None = None,
    *,
    api_key: str | None = None,
    api_base: str | None = None,
    model_type: str | None = None,
) -> Any:
    global _configured_lm
    if _configured_lm is not None and model is None:
        return _configured_lm
    dspy = get_dspy()

    model_name = model or os.getenv("OG_AGENT_MODEL", _DEFAULT_MODEL)
    api_key = _sanitize_api_key(api_key or os.getenv("OG_AGENT_API_KEY"))
    api_base = api_base or os.getenv("OG_AGENT_API_BASE")
    model_type = model_type or os.getenv("OG_AGENT_MODEL_TYPE")
    reasoning_effort = os.getenv("OG_AGENT_REASONING_EFFORT")

    api_key = _resolve_provider_api_key(model_name, api_key)

    lm_kwargs: dict[str, object] = {}
    if api_key:
        lm_kwargs["api_key"] = api_key
    if not api_base and model_name.startswith("openai/"):
        api_base = os.getenv("OPENAI_API_BASE")

    if api_base:
        lm_kwargs["api_base"] = api_base
    if not model_type and model_name.startswith("openai/gpt-5"):
        model_type = "responses"
    if model_type:
        lm_kwargs["model_type"] = model_type
    if not reasoning_effort and model_name == "openai/gpt-5.2-codex":
        reasoning_effort = "high"
    if reasoning_effort:
        lm_kwargs["reasoning_effort"] = reasoning_effort

    lm = dspy.LM(model_name, **lm_kwargs)
    dspy.configure(lm=lm)
    _configured_lm = lm
    return lm


def _resolve_provider_api_key(model_name: str, api_key: str | None) -> str | None:
    if api_key:
        return _sanitize_api_key(api_key)
    if model_name.startswith("openai/"):
        return _sanitize_api_key(
            os.getenv("OPENAI_API_KEY") or os.getenv("OG_AGENT_OPENAI_API_KEY")
        )
    if model_name.startswith("anthropic/"):
        return _sanitize_api_key(
            os.getenv("ANTHROPIC_API_KEY") or os.getenv("OG_AGENT_ANTHROPIC_API_KEY")
        )
    return None
