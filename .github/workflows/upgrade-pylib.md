---
description: |
  Pick an out-of-sync Python library from the todo list and upgrade it
  by running `scripts/update_lib quick`, then open a pull request.

on:
  workflow_dispatch:
    inputs:
      name:
        description: "Module name to upgrade (leave empty to auto-pick)"
        required: false
        type: string

timeout-minutes: 45

permissions:
  contents: read
  issues: read
  pull-requests: read

network:
  allowed:
    - defaults
    - rust
    - python

engine: claude

runtimes:
  python:
    version: "3.12"

tools:
  bash:
    - ":*"
  edit:
  github:
    toolsets: [repos, issues, pull_requests]
    read-only: true

safe-outputs:
  create-pull-request:
    title-prefix: "Update "
    labels: [pylib-sync]
    draft: false
    expires: 30

env:
  PYTHON_VERSION: "v3.14.3"
  ISSUE_ID: "6839"
---

# Upgrade Python Library

You are an automated maintenance agent for RustPython, a Python 3 interpreter written in Rust. Your task is to upgrade one out-of-sync Python standard library module from CPython.

## Step 1: Set up the environment

Clone CPython at the correct version tag under the working directory:

```bash
git clone --depth 1 --branch "$PYTHON_VERSION" https://github.com/python/cpython.git cpython
```

## Step 2: Pick a module to upgrade

If the user provided a module name via `${{ github.event.inputs.name }}`, use **exactly** that module. Skip the selection logic below and go directly to Step 3.

If NO module name was provided, run the todo script to auto-pick one:

```bash
python3 scripts/update_lib todo
```

From the output, pick **one** module that:
- Is marked `[ ]` (not yet up-to-date)
- Has `[no deps]` or `[0/N deps]` (all dependencies are satisfied)
- Has a small diff count (`Δ` number) — prefer modules with smaller diffs for reliability
- Is NOT one of these complex modules: `opcode`, `datetime`, `collections`, `random`, `hashlib`, `tokenize`, `pdb`, `_pyrepl`, `concurrent`, `asyncio`, `multiprocessing`, `ctypes`, `idlelib`, `tkinter`, `shutil`, `tarfile`, `email`, `unittest`

## Step 3: Run the upgrade

Run the quick upgrade command. This will copy the library from CPython, migrate test files preserving RustPython markers, auto-mark test failures, and create a git commit:

```bash
python3 scripts/update_lib quick <module_name>
```

This takes a while because it builds RustPython (`cargo build --release`) and runs tests to determine which ones pass or fail.

If the command fails, report the error and stop. Do not try to fix Rust code or modify test files manually.

## Step 4: Verify the result

After the script succeeds, check what changed:

```bash
git log -1 --stat
git diff HEAD~1 --stat
```

Make sure the commit was created with the correct message format: `Update <name> from <version>`.

## Step 5: Create the pull request

Create a pull request. Reference issue #${{ env.ISSUE_ID }} in the body but do **NOT** use keywords that auto-close issues (Fix, Close, Resolve).

Use this format for the PR body:

```
## Summary

Upgrade `<module_name>` from CPython $PYTHON_VERSION.

Part of #$ISSUE_ID

## Changes

- Updated `Lib/<module_name>` from CPython
- Migrated test files preserving RustPython markers
- Auto-marked test failures with `@expectedFailure`
```
