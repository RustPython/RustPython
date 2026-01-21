---
allowed-tools: Bash(python3:*), Bash(cargo run:*), Read, Grep, Glob, Bash(git add:*), Bash(git commit:*)
---

# Investigate Test Failure

Investigate why a specific test is failing and determine if it can be fixed or needs an issue.

## Arguments
- `$ARGUMENTS`: Failed test identifier (e.g., `test_inspect.TestGetSourceBase.test_getsource_reload`)

## Steps

1. **Analyze failure cause**
   - Read the test code
   - Analyze failure message/traceback
   - Check related RustPython code

2. **Verify behavior in CPython**
   - Run the test with `python3 -m unittest` to confirm expected behavior
   - Document the expected output

3. **Determine fix feasibility**
   - **Simple fix** (import issues, small logic bugs): Fix and commit
   - **Complex fix** (major unimplemented features): Collect issue info and report to user

4. **For complex issues - Collect issue information**
   Following `.github/ISSUE_TEMPLATE/report-incompatibility.md` format:

   - **Feature**: Description of missing/broken Python feature
   - **Minimal reproduction code**: Smallest code that reproduces the issue
   - **CPython behavior**: Result when running with python3
   - **RustPython behavior**: Result when running with cargo run
   - **Python Documentation link**: Link to relevant CPython docs

   Report collected information to the user. Issue creation is done only upon user request.

   Example issue creation command:
   ```
   gh issue create --template report-incompatibility.md --title "..." --body "..."
   ```
