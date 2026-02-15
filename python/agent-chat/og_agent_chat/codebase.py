from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

DEFAULT_EXTENSIONS = {
    ".py",
    ".rs",
    ".ts",
    ".tsx",
    ".js",
    ".md",
    ".toml",
    ".yaml",
    ".yml",
    ".json",
}
DEFAULT_IGNORE_DIRS = {
    ".git",
    ".venv",
    "__pycache__",
    "node_modules",
    "dist",
    "build",
    "target",
}


@dataclass
class CodebaseDocument:
    path: Path
    content: str


@dataclass
class CodebaseIndex:
    root: Path
    documents: list[CodebaseDocument]
    truncated: list[Path]

    @classmethod
    def from_root(
        cls,
        root: Path,
        extensions: Iterable[str] | None = None,
        max_files: int = 200,
        max_file_chars: int = 12000,
        max_total_chars: int = 300000,
        ignore_dirs: Iterable[str] | None = None,
    ) -> "CodebaseIndex":
        ext_set = set(extensions or DEFAULT_EXTENSIONS)
        ignore_set = set(ignore_dirs or DEFAULT_IGNORE_DIRS)
        documents: list[CodebaseDocument] = []
        truncated: list[Path] = []
        total_chars = 0

        for path in root.rglob("*"):
            if len(documents) >= max_files:
                break
            if not path.is_file():
                continue
            if any(part in ignore_set for part in path.parts):
                continue
            if path.suffix and path.suffix not in ext_set:
                continue

            try:
                text = path.read_text(encoding="utf-8", errors="ignore")
            except OSError:
                continue

            if len(text) > max_file_chars:
                text = text[:max_file_chars] + "\n... (truncated)\n"
                truncated.append(path)
            if total_chars + len(text) > max_total_chars:
                break
            total_chars += len(text)
            documents.append(CodebaseDocument(path=path, content=text))

        return cls(root=root, documents=documents, truncated=truncated)

    def file_listing(self, limit: int = 100) -> str:
        items = [str(doc.path.relative_to(self.root)) for doc in self.documents[:limit]]
        if len(self.documents) > limit:
            items.append("... (more files omitted)")
        return "\n".join(items)

    def build_context(self) -> str:
        parts: list[str] = []
        for doc in self.documents:
            rel_path = doc.path.relative_to(self.root)
            parts.append(f"FILE: {rel_path}\n{doc.content}")
        if self.truncated:
            truncated_list = "\n".join(
                str(path.relative_to(self.root)) for path in self.truncated
            )
            parts.append("TRUNCATED_FILES:\n" + truncated_list)
        return "\n\n".join(parts)

    def search_regex(self, pattern: str, max_matches: int = 20) -> list[str]:
        import re

        results: list[str] = []
        regex = re.compile(pattern, re.IGNORECASE)
        for doc in self.documents:
            lines = doc.content.splitlines()
            for idx, line in enumerate(lines, start=1):
                if regex.search(line):
                    rel_path = doc.path.relative_to(self.root)
                    results.append(f"{rel_path}:{idx}: {line.strip()}")
                    if len(results) >= max_matches:
                        return results
        return results
