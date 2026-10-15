"""Validate an explicit target's remaining metadata before canonical migration."""

from pathlib import Path

from update_lib.patch_spec import UtMethod, patches_from_json


def select_remaining_patches(current, document, relative_path: Path):
    if not isinstance(document, dict) or set(document) != {"target", "patches"}:
        raise ValueError("Expected target and patches metadata")
    if (
        relative_path.parts[:2] != ("Lib", "test")
        or relative_path.suffix != ".py"
        or document["target"] != relative_path.as_posix()
    ):
        raise ValueError("Marker override target does not match the test file")
    try:
        remaining = patches_from_json(document["patches"])
    except (TypeError, KeyError, AttributeError, ValueError) as error:
        raise ValueError("Invalid remaining marker metadata") from error
    for cls, methods in remaining.items():
        if cls not in current or any(method not in current[cls] for method in methods):
            raise ValueError("Marker override cannot add targets")
    for cls, methods in current.items():
        for method, existing in methods.items():
            pending = list(existing)
            omitted = []
            for marker in remaining.get(cls, {}).get(method, []):
                if marker not in pending:
                    raise ValueError(
                        "Marker override cannot add, change, or reorder markers"
                    )
                index = pending.index(marker)
                omitted.extend(pending[:index])
                pending = pending[index + 1 :]
            omitted.extend(pending)
            if any(marker.ut_method != UtMethod.ExpectedFailure for marker in omitted):
                raise ValueError(
                    "Only unconditional expectedFailure markers may be removed"
                )
    return remaining
