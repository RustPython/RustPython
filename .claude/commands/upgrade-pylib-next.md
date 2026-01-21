---
allowed-tools: Skill(upgrade-pylib), Bash(gh pr list:*)
---

# Upgrade Next Python Library

Find the next Python library module ready for upgrade and run `/upgrade-pylib` for it.

## Current TODO Status

!`cargo run --release -- scripts/update_lib todo 2>/dev/null`

## Open Upgrade PRs

!`gh pr list --search "Update in:title" --json number,title --template '{{range .}}#{{.number}} {{.title}}{{"\n"}}{{end}}'`

## Instructions

From the TODO list above, find modules matching these patterns (in priority order):

1. `[ ] [no deps]` - Modules with no dependencies (can be upgraded immediately)
2. `[ ] [n/n]` - Modules where all dependencies are already upgraded (e.g., `[3/3]`, `[5/5]`)

These patterns indicate modules that are ready to upgrade without blocking dependencies.

**Important**: Skip any modules that already have an open PR in the "Open Upgrade PRs" list above.

**After identifying a suitable module**, run:
```
/upgrade-pylib <module_name>
```

If no modules match these criteria, inform the user that all eligible modules have dependencies that need to be upgraded first.
