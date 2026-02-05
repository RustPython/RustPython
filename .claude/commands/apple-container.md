---
allowed-tools: Bash(container *), Bash(cargo *), Read, Grep, Glob
---

# Run Tests in Linux Container (Apple `container` CLI)

Run RustPython tests inside a Linux container using Apple's `container` CLI.
**NEVER use Docker, Podman, or any other container runtime.** Only use the `container` command.

## Arguments
- `$ARGUMENTS`: Test command to run (e.g., `test_io`, `test_codecs -v`, `test_io -v -m "test_errors"`)

## Prerequisites

The `container` CLI is installed via `brew install container`.
The dev image `rustpython-dev` is already built.

## Steps

1. **Check if the container is already running**
   ```shell
   container list 2>/dev/null | grep rustpython-test
   ```

2. **Start the container if not running**
   ```shell
   container run -d --name rustpython-test -m 8G -c 4 \
       --mount type=bind,source=/Users/al03219714/Projects/RustPython3,target=/workspace \
       -w /workspace rustpython-dev sleep infinity
   ```

3. **Run the test inside the container**
   ```shell
   container exec rustpython-test sh -c "cargo run --release -- -m test $ARGUMENTS"
   ```

4. **Report results**
   - Show test summary (pass/fail counts, expected failures, unexpected successes)
   - Highlight any new failures compared to macOS results if available
   - Do NOT stop or remove the container after testing (keep it for reuse)

## Notes
- The workspace is bind-mounted, so local code changes are immediately available
- Use `container exec rustpython-test sh -c "..."` for any command inside the container
- To rebuild after code changes, run: `container exec rustpython-test sh -c "cargo build --release"`
- To stop the container when done: `container rm -f rustpython-test`
