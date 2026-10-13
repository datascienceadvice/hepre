#!/usr/bin/env python3
"""Structural validation of repository metadata files (no external dependencies).

Checks the places where YAML mistakes actually happen in this repository:
tab characters, non-space indentation, block scalar (`|`, `>-`) indentation,
and the presence of required top-level keys.

It deliberately does NOT validate CITATION.cff against the official CFF JSON
schema: that schema is not shipped locally. Before a release, run a real
validator, e.g. `cffconvert --validate` or the GitHub "Citation File Format"
check, which requires network access.

Usage (from the repository root):
    python scripts/validate_metadata.py
Exits non-zero on failure, so it can be wired into CI.
"""

from __future__ import annotations

import re
import sys

BLOCK_SCALAR = re.compile(r"^(\s*)([A-Za-z_][\w.\-]*):\s*([|>][-+]?)\s*$")

TARGETS: list[tuple[str, list[str]]] = [
    (
        "CITATION.cff",
        ["cff-version", "message", "title", "type", "authors", "license", "version"],
    ),
    (".github/workflows/ci.yml", ["name", "on", "jobs"]),
]


def check(path: str, required: list[str]) -> list[str]:
    with open(path, encoding="utf-8") as handle:
        lines = handle.read().split("\n")

    errors: list[str] = []

    for number, line in enumerate(lines, 1):
        if "\t" in line:
            errors.append(f"{path}:{number}: tab character in indentation")
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        indent = len(line) - len(line.lstrip(" "))
        if line[:indent] != " " * indent:
            errors.append(f"{path}:{number}: indentation uses non-space characters")

    for index, line in enumerate(lines):
        match = BLOCK_SCALAR.match(line)
        if not match:
            continue
        key_indent = len(match.group(1))
        cursor = index + 1
        while cursor < len(lines) and lines[cursor].strip() == "":
            cursor += 1
        if cursor >= len(lines):
            continue
        content = lines[cursor]
        content_indent = len(content) - len(content.lstrip(" "))
        if content_indent <= key_indent:
            errors.append(
                f"{path}:{cursor + 1}: block scalar '{match.group(2)}' content is not "
                f"indented past its key"
            )

    text = "\n".join(lines)
    for key in required:
        if not re.search(rf"^{re.escape(key)}:", text, re.M):
            errors.append(f"{path}: missing required top-level key: {key}")

    return errors


def main() -> int:
    errors: list[str] = []
    for path, required in TARGETS:
        try:
            errors += check(path, required)
        except OSError as exc:
            errors.append(f"{path}: cannot read ({exc})")

    if errors:
        print("metadata validation: FAILED")
        for error in errors:
            print("  " + error)
        return 1

    print(f"metadata validation: OK ({len(TARGETS)} files checked)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
