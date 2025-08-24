import ast
import urllib.request
from typing import TYPE_CHECKING

from rpau.logger import get_logger

if TYPE_CHECKING:
    import pathlib

COL_OFFSET = 4
INDENT1 = " " * COL_OFFSET
INDENT2 = INDENT1 * 2
COMMENT = "TODO: RUSTPYTHON"

type Patch = dict[str, dict[str, str]]
type Conf = dict[str, Patch]

logger = get_logger(__name__)


def fetch_upstream(*, base_url: str, path: str, version: str) -> str:
    upstream_url = "/".join((base_url, version, path))
    logger.debug(f"{upstream_url=}")

    with urllib.request.urlopen(upstream_url) as f:
        contents = f.read().decode()
    return contents


def get_upstream_contents(
    *, base_url: str, cache_dir: "pathlib.Path | None", path: str, version: str
) -> str:
    fetch = lambda: fetch_upstream(base_url=base_url, path=path, version=version)

    if cache_dir:
        cached_file = cache_dir / version / path
        try:
            contents = cached_file.read_text()
        except FileNotFoundError:
            cached_file.parent.mkdir(parents=True, exist_ok=True)
            contents = fetch()
            cached_file.write_text(contents)

        return contents
    else:
        return fetch()


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

                """
                if any(
                    is_patch_present(dec_node, patch_conf)
                    for dec_node in fn_node.decorator_list
                    if isinstance(dec_node, (ast.Attribute, ast.Call))
                ):
                    continue
                """

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

    # Going in reverse to not distrupt the line offset
    patches = list(iter_patches(tree, conf))
    for lineno, patch in sorted(patches, reverse=True):
        lines.insert(lineno, patch)

    return "\n".join(lines)


def run(
    conf: dict,
    path: str,
    version: str,
    output_dir: "pathlib.Path",
    cache_dir: "pathlib.Path | None",
    base_upstream_url: str,
):
    contents = get_upstream_contents(
        path=path, version=version, base_url=base_upstream_url, cache_dir=cache_dir
    )

    patched_contents = apply_conf(contents, conf)
    new_contents = f"# upstream_version: {version}\n{patched_contents}"

    output_file = output_dir / path
    # TODO: Add logic to preserve file permissions if exists
    output_file.parent.mkdir(parents=True, exist_ok=True)
    output_file.write_text(f"{new_contents}\n")
