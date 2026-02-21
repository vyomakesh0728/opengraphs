from __future__ import annotations

import json
import os
from pathlib import Path
from typing import Any, Optional


def _ensure_deno_dir() -> None:
    tmpdir = os.getenv("TMPDIR") or os.getenv("TEMP") or os.getenv("TMP") or "/tmp"
    deno_dir = Path(os.getenv("DENO_DIR") or (Path(tmpdir) / "opengraphs-deno-cache"))
    deno_dir.mkdir(parents=True, exist_ok=True)
    os.environ.setdefault("DENO_DIR", str(deno_dir))


_DEFAULT_MODEL = "openai/gpt-5.2-codex"
_DEFAULT_PRIME_API_BASE = "https://api.pinference.ai/api/v1"
_VALID_INFERENCE_PROVIDERS = {"auto", "openai", "prime"}
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
    inference_provider: str | None = None,
) -> Any:
    global _configured_lm
    if _configured_lm is not None and model is None:
        return _configured_lm
    dspy = get_dspy()

    model_name = model or os.getenv("OG_AGENT_MODEL", _DEFAULT_MODEL)
    api_key = _sanitize_api_key(api_key or os.getenv("OG_AGENT_API_KEY"))
    api_base = api_base or os.getenv("OG_AGENT_API_BASE")
    provider_raw = (
        inference_provider
        or os.getenv("OG_AGENT_INFERENCE_PROVIDER")
        or os.getenv("OG_AGENT_PROVIDER")
        or "auto"
    )
    provider = _normalize_inference_provider(provider_raw)
    model_type = model_type or os.getenv("OG_AGENT_MODEL_TYPE")
    reasoning_effort = os.getenv("OG_AGENT_REASONING_EFFORT")

    api_base = _resolve_provider_api_base(model_name, api_base, provider)
    api_key = _resolve_provider_api_key(
        model_name,
        api_key,
        api_base=api_base,
        provider=provider,
    )
    effective_model_name = _resolve_effective_model_name(
        model_name,
        api_base=api_base,
        provider=provider,
    )

    lm_kwargs: dict[str, object] = {}
    if api_key:
        lm_kwargs["api_key"] = api_key

    if api_base:
        lm_kwargs["api_base"] = api_base
        prime_headers = _resolve_prime_inference_headers(api_base)
        if prime_headers:
            lm_kwargs["extra_headers"] = prime_headers
    if not model_type and model_name.startswith("openai/gpt-5"):
        model_type = "responses"
    if model_type:
        lm_kwargs["model_type"] = model_type
    if not reasoning_effort and model_name == "openai/gpt-5.2-codex":
        reasoning_effort = "high"
    if reasoning_effort:
        lm_kwargs["reasoning_effort"] = reasoning_effort

    lm = dspy.LM(effective_model_name, **lm_kwargs)
    dspy.configure(lm=lm)
    _configured_lm = lm
    return lm


def _normalize_inference_provider(value: str | None) -> str:
    normalized = (value or "auto").strip().lower()
    if normalized not in _VALID_INFERENCE_PROVIDERS:
        return "auto"
    return normalized


def _is_prime_api_base(api_base: str | None) -> bool:
    if not api_base:
        return False
    lower = api_base.lower()
    return "pinference.ai" in lower or "primeintellect.ai" in lower


def _prime_default_api_base() -> str:
    return (
        os.getenv("OG_AGENT_PRIME_API_BASE")
        or os.getenv("PRIME_API_BASE")
        or _DEFAULT_PRIME_API_BASE
    )


def _resolve_provider_api_base(
    model_name: str,
    api_base: str | None,
    provider: str,
) -> str | None:
    if api_base:
        return api_base

    if provider == "prime":
        return _prime_default_api_base()

    if provider == "openai":
        if model_name.startswith("openai/"):
            return os.getenv("OPENAI_API_BASE")
        return None

    # auto: if Prime key is present for an OpenAI-compatible model, prefer Prime inference.
    if model_name.startswith("openai/"):
        prime_key = _sanitize_api_key(
            os.getenv("PRIME_API_KEY") or os.getenv("OG_AGENT_PRIME_API_KEY")
        )
        if prime_key:
            return _prime_default_api_base()
        return os.getenv("OPENAI_API_BASE")
    return None


def _resolve_effective_model_name(
    model_name: str,
    *,
    api_base: str | None,
    provider: str,
) -> str:
    # litellm strips one provider prefix for OpenAI models. Prime's endpoint expects
    # model ids like "openai/gpt-4.1-mini", so for Prime routing we prefix once more.
    is_prime = provider == "prime" or _is_prime_api_base(api_base)
    if is_prime and model_name.startswith("openai/") and not model_name.startswith("openai/openai/"):
        return f"openai/{model_name}"
    return model_name


def _resolve_provider_api_key(
    model_name: str,
    api_key: str | None,
    *,
    api_base: str | None,
    provider: str,
) -> str | None:
    if api_key:
        return _sanitize_api_key(api_key)

    if provider == "prime" or _is_prime_api_base(api_base):
        return _sanitize_api_key(
            os.getenv("PRIME_API_KEY")
            or os.getenv("OG_AGENT_PRIME_API_KEY")
            or os.getenv("OG_AGENT_API_KEY")
            or os.getenv("OPENAI_API_KEY")
            or os.getenv("OG_AGENT_OPENAI_API_KEY")
        )

    if model_name.startswith("openai/"):
        return _sanitize_api_key(
            os.getenv("OPENAI_API_KEY") or os.getenv("OG_AGENT_OPENAI_API_KEY")
        )
    if model_name.startswith("anthropic/"):
        return _sanitize_api_key(
            os.getenv("ANTHROPIC_API_KEY") or os.getenv("OG_AGENT_ANTHROPIC_API_KEY")
        )
    return None


def _resolve_prime_inference_headers(api_base: str) -> dict[str, str] | None:
    if not _is_prime_api_base(api_base):
        return None

    team_id = _sanitize_api_key(os.getenv("PRIME_TEAM_ID"))
    if not team_id:
        config_path = Path.home() / ".prime" / "config.json"
        try:
            raw = json.loads(config_path.read_text(encoding="utf-8"))
        except Exception:
            raw = {}
        team_id = _sanitize_api_key(str(raw.get("team_id") or ""))

    if not team_id:
        return None
    return {"X-Prime-Team-ID": team_id}
