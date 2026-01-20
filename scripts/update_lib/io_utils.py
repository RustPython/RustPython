"""
I/O utilities for update_lib.

This module provides functions for:
- Safe file reading with error handling
- Safe AST parsing with error handling
- Iterating over Python files
"""

import ast
import pathlib
from collections.abc import Iterator


def safe_read_text(path: pathlib.Path) -> str | None:
    """
    Read file content with UTF-8 encoding, returning None on error.

    Args:
        path: Path to the file

    Returns:
        File content as string, or None if reading fails
    """
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return None


def safe_parse_ast(content: str) -> ast.Module | None:
    """
    Parse Python content into AST, returning None on syntax error.

    Args:
        content: Python source code

    Returns:
        AST module, or None if parsing fails
    """
    try:
        return ast.parse(content)
    except SyntaxError:
        return None


def iter_python_files(path: pathlib.Path) -> Iterator[pathlib.Path]:
    """
    Yield Python files from a file or directory.

    If path is a file, yields just that file.
    If path is a directory, yields all .py files recursively.

    Args:
        path: Path to a file or directory

    Yields:
        Paths to Python files
    """
    if path.is_file():
        yield path
    else:
        yield from path.glob("**/*.py")


def read_python_files(path: pathlib.Path) -> Iterator[tuple[pathlib.Path, str]]:
    """
    Read all Python files from a path, yielding (path, content) pairs.

    Skips files that cannot be read.

    Args:
        path: Path to a file or directory

    Yields:
        Tuples of (file_path, file_content)
    """
    for py_file in iter_python_files(path):
        content = safe_read_text(py_file)
        if content is not None:
            yield py_file, content
