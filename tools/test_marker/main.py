#!/usr/bin/env python
import ast
import pathlib
import tomllib
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from collections.abc import Iterator

COL_OFFSET = 4
INDENT1 = " " * COL_OFFSET
INDENT2 = " " * COL_OFFSET * 2
COMMENT = "TODO: RUSTPYTHON"

ROOT_DIR = pathlib.Path(__file__).parents[2]
CONFS = ROOT_DIR / "tools" / "test_marker" / "confs"


type Patch = dict[str, dict[str, str]]
type Conf = dict[str, Patch]


def format_patch(patch_conf: Patch) -> str:
    """
    Transforms a patch definition to a raw python code.

    Parameters
    ----------
    patch_conf : Patch
        Conf of the patch.

    Returns
    -------
    str
        Raw python source code.

    Examples
    --------
    >>> patch = {"expectedFailure": {"reason": "lorem ipsum"}}
    >>> format_patch(patch)
    '@unittest.expectedFailure # TODO: RUSTPYTHON; lorem ipsum'
    """
    method, conf = next(iter(patch_conf.items()))
    prefix = f"@unittest.{method}"

    reason = conf.get("reason", "")
    res = ""
    match method:
        case "expectedFailure":
            res = f"{prefix} # {COMMENT}; {reason}"
        case "expectedFailureIfWindows" | "skip":
            res = f'{prefix}("{COMMENT}; {reason}")'
        case "skipIf":
            cond = conf["cond"]
            res = f'{prefix}({cond}, "{COMMENT}; {reason}")'

    return res.strip().rstrip(";").strip()


def is_patch_present(node: ast.Attribute | ast.Call, patch_conf: Patch) -> bool:
    """
    Detect whether an AST node (of a decorator) is matching to our patch.

    We accept both `ast.Attribute` and `ast.Call` because:
        * ast.Attribute: `@unittest.expectedFailure`
        * ast.Call: `@unittest.expectedFailureIfWindows(...)` / `@unittest.skipIf(...)`

    Parameters
    ----------
    node : ast.Attribute | ast.Call
        AST node to query.
    patch_conf : Patch
        Patch(es) to match against.

    Returns
    -------
    bool
        Whether or not we got a match.
    """
    is_attr = isinstance(node, ast.Attribute)
    attr_node = node if is_attr else node.func

    if isinstance(attr_node, ast.Name):
        return False

    if attr_node.value.id != "unittest":
        return False

    if is_attr:
        return node.attr in patch_conf

    return "RUSTPYTHON" in ast.unparse(node)


def iter_patches(tree: ast.Module, conf: Conf) -> "Iterator[tuple[int, str]]":
    """
    Get needed patches to apply for given ast tree based on the conf.

    Parameters
    ----------
    tree : ast.Module
        AST tree to iterate on.
    conf : Conf
        Dict of `{ClassName: {test_name: Patch}}`.

    Yields
    ------
    lineno : int
        Line number where to insert the patch.
    patch : str
        Raw python code to be inserted at `lineno`.
    """
    # Phase 1: Iterate and mark existing tests
    for key, nodes in ast.iter_fields(tree):
        if key != "body":
            continue

        for i, cls_node in enumerate(nodes):
            if not isinstance(cls_node, ast.ClassDef):
                continue

            if not (cls_conf := conf.get(cls_node.name)):
                continue

            for fn_node in cls_node.body:
                if not isinstance(fn_node, ast.FunctionDef):
                    continue

                if not (patch_conf := cls_conf.pop(fn_node.name, None)):
                    continue

                if any(
                    is_patch_present(dec_node, patch_conf)
                    for dec_node in fn_node.decorator_list
                    if isinstance(dec_node, (ast.Attribute, ast.Call))
                ):
                    continue

                lineno = min(
                    (dec_node.lineno for dec_node in fn_node.decorator_list),
                    default=fn_node.lineno,
                )

                indent = " " * fn_node.col_offset
                patch = format_patch(patch_conf)
                yield (lineno - 1, f"{indent}{patch}")

    # Phase 2: Iterate and mark inhereted tests
    for key, nodes in ast.iter_fields(tree):
        if key != "body":
            continue

        for i, cls_node in enumerate(nodes):
            if not isinstance(cls_node, ast.ClassDef):
                continue

            if not (cls_conf := conf.get(cls_node.name)):
                continue

            for fn_name, patch_conf in cls_conf.items():
                patch = format_patch(patch_conf)
                yield (
                    cls_node.end_lineno,
                    f"""
{INDENT1}{patch}
{INDENT1}def {fn_name}(self):
{INDENT2}return super().{fn_name}()
""".rstrip(),
                )


def apply_conf(contents: str, conf: dict) -> str:
    """
    Patch a given source code based on the conf.

    Parameters
    ----------
    contents : str
        Raw python source code.
    conf : Conf
        Dict of `{ClassName: {test_name: Patch}}`.

    Returns
    -------
    str
        Patched raw python code.
    """
    lines = contents.splitlines()
    tree = ast.parse(contents)

    # Going in reverse to not distrupt the lines offset
    patches = list(iter_patches(tree, conf))
    for lineno, patch in sorted(patches, reverse=True):
        lines.insert(lineno, patch)

    return "\n".join(lines)


def main():
    for conf_file in CONFS.rglob("*.toml"):
        conf = tomllib.loads(conf_file.read_text())
        path = ROOT_DIR / conf.pop("path")
        patched = apply_conf(path.read_text(), conf)
        path.write_text(patched + "\n")


if __name__ == "__main__":
    main()
