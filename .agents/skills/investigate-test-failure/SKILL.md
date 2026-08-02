---
name: investigate-test-failure
description: Investigate a failing RustPython test, compare with CPython behavior, and decide whether to implement a fix or prepare incompatibility issue details.
allowed-tools: Bash(python3:*) Bash(cargo run:*) Bash(gh issue create:*) Read Grep Glob Bash(git add:*) Bash(git commit:*) Bash(cargo fmt:*) Bash(git diff:*) Task
---

# Investigate Test Failure

Investigate why a specific test is failing and determine whether it can be fixed now or should be reported as an incompatibility issue.

## Arguments

- Failed test identifier (example: `test_inspect.TestGetSourceBase.test_getsource_reload`)

## Workflow

1. Analyze the failure cause:

- Read the test code.
- Analyze failure message and traceback.
- Check related RustPython implementation code.

2. Verify behavior in CPython:

- Run the test with `python3 -m unittest`.
- Document expected behavior/output.

3. Determine fix feasibility:

- Simple fix (import issues, small logic bugs): implement fix, run formatting and checks, review changes, and commit.
- Complex fix (major missing feature): gather issue information and report to user.

Pre-commit review process:

- Run `git diff` to review local changes.
- Use a focused subagent review to compare implementation against CPython behavior and check for missed edge cases.
- Commit only after review passes.

4. For complex issues, collect issue information using `.github/ISSUE_TEMPLATE/report-incompatibility.md`:

- Feature: missing or broken Python feature.
- Minimal reproduction code.
- CPython behavior (`python3`).
- RustPython behavior (`cargo run`).
- Python documentation reference link.

Report the collected information to the user. Create an issue only when explicitly requested.

Example issue command:

```shell
gh issue create --template report-incompatibility.md --title "..." --body "..."
```
