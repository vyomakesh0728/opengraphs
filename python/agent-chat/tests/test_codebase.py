from __future__ import annotations

from og_agent_chat.codebase import CodebaseIndex


def test_from_root_ignores_dirs_and_tracks_truncated_files(tmp_path) -> None:
    (tmp_path / "keep.py").write_text("print('ok')\n", encoding="utf-8")
    (tmp_path / "notes.txt").write_text("ignore me\n", encoding="utf-8")
    (tmp_path / "node_modules").mkdir()
    (tmp_path / "node_modules" / "skip.py").write_text("print('skip')\n", encoding="utf-8")
    (tmp_path / "big.md").write_text("x" * 30, encoding="utf-8")

    index = CodebaseIndex.from_root(
        tmp_path,
        max_file_chars=20,
        max_total_chars=500,
    )

    doc_names = {doc.path.name for doc in index.documents}
    assert doc_names == {"big.md", "keep.py"}
    assert (tmp_path / "big.md") in index.truncated

    big_doc = next(doc for doc in index.documents if doc.path.name == "big.md")
    assert big_doc.content.endswith("\n... (truncated)\n")

    context = index.build_context()
    assert "FILE: keep.py" in context
    assert "TRUNCATED_FILES:\nbig.md" in context


def test_file_listing_marks_omitted_entries(tmp_path) -> None:
    for idx in range(3):
        (tmp_path / f"file_{idx}.py").write_text(f"print({idx})\n", encoding="utf-8")

    index = CodebaseIndex.from_root(tmp_path, max_total_chars=500)
    listing = index.file_listing(limit=2)

    assert "... (more files omitted)" in listing


def test_search_regex_is_case_insensitive_and_honors_max_matches(tmp_path) -> None:
    (tmp_path / "search.py").write_text(
        "alpha\nALPHA\nbeta\nalpha\n",
        encoding="utf-8",
    )

    index = CodebaseIndex.from_root(tmp_path, max_total_chars=500)
    results = index.search_regex("alpha", max_matches=2)

    assert results == [
        "search.py:1: alpha",
        "search.py:2: ALPHA",
    ]
