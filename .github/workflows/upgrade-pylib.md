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

engine: copilot

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

cache:
  key: cpython-lib-${{ env.PYTHON_VERSION }}
  path: cpython
  restore-keys:
    - cpython-lib-

env:
  PYTHON_VERSION: "v3.14.3"
  ISSUE_ID: "6839"
---

# Upgrade Python Library

You are an automated maintenance agent for RustPython, a Python 3 interpreter written in Rust. Your task is to upgrade one out-of-sync Python standard library module from CPython.

## Step 1: Set up the environment

The CPython source may already be cached. Check if the `cpython` directory exists and has the correct version:

```bash
if [ -d "cpython/Lib" ]; then
    echo "CPython cache hit, skipping clone"
else
    git clone --depth 1 --branch "$PYTHON_VERSION" https://github.com/python/cpython.git cpython
fi
```

## Step 2: Determine module name

Run this script to determine the module name:

```bash
MODULE_NAME="${{ github.event.inputs.name }}"
if [ -z "$MODULE_NAME" ]; then
    echo "No module specified, running todo to find one..."
    python3 scripts/update_lib todo
    echo "Pick one module from the list above that is marked [ ], has no unmet deps, and has a small Δ number."
    echo "Do NOT pick: opcode, datetime, random, hashlib, tokenize, pdb, _pyrepl, concurrent, asyncio, multiprocessing, ctypes, idlelib, tkinter, shutil, tarfile, email, unittest"
else
    echo "Module specified by user: $MODULE_NAME"
fi
```

If the script printed "Module specified by user: ...", use that exact name. If it printed the todo list, pick one suitable module from it.

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
