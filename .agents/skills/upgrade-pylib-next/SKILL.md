---
name: upgrade-pylib-next
description: Pick the next CPython stdlib module ready for upgrade based on update_lib todo output and skip modules with open upgrade PRs.
allowed-tools: Skill(upgrade-pylib) Bash(gh pr list:*) Bash(cargo run:*)
---

# Upgrade Next Python Library

Find the next Python library module ready for upgrade and then run the `upgrade-pylib` workflow for it.

## Workflow

1. Get current TODO status:

```shell
cargo run --release -- scripts/update_lib todo 2>/dev/null
```

2. Get open upgrade PRs:

```shell
gh pr list --search "Update in:title" --json number,title --template '{{range .}}#{{.number}} {{.title}}{{"\\n"}}{{end}}'
```

3. From TODO output, select modules in this priority order:

- `[ ] [no deps]`
- `[ ] [0/n]` where dependencies are all upgraded (for example `[0/3]`, `[0/5]`)

4. Skip modules that already have an open upgrade PR.

5. Run upgrade for selected module using the `upgrade-pylib` workflow.

If no modules match, report that eligible modules are blocked by dependencies.
