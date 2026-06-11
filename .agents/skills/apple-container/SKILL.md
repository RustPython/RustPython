---
name: apple-container
description: Run RustPython tests in Apple's `container` CLI Linux environment. Use when user asks to run or compare Linux test results on macOS.
allowed-tools: Bash(container:*) Bash(cargo:*) Read Grep Glob
---

# Run Tests in Linux Container (Apple `container` CLI)

Run RustPython tests inside a Linux container using Apple's `container` CLI.
NEVER use Docker, Podman, or any other container runtime. Only use the `container` command.

## Arguments

- Test command to run (examples: `test_io`, `test_codecs -v`, `test_io -v -m "test_errors"`)

## Prerequisites

- The `container` CLI is installed via `brew install container`.
- The dev image `rustpython-dev` is already built.

## Workflow

1. Check whether the test container is already running:

   ```shell
   container list 2>/dev/null | grep rustpython-test
   ```

2. Start the container if it is not running:

   ```shell
   container run -d --name rustpython-test -m 8G -c 4 \
       --mount type=bind,source="$(pwd)",target=/workspace \
       -w /workspace rustpython-dev sleep infinity
   ```

3. Run the requested test command inside the container:

   ```shell
   container exec rustpython-test cargo run --release -- -m test <test-args>
   ```

4. Report results:

- Show pass/fail summary and expected failures / unexpected successes.
- Highlight new failures compared to macOS results, if available.
- Do not stop or remove the container after testing.

## Notes

- The workspace is bind-mounted, so local code changes are immediately available in the container.
- Use `container exec rustpython-test sh -c "..."` for any command inside the container.
- Rebuild after code changes with:

  ```shell
  container exec rustpython-test sh -c "cargo build --release"
  ```

- Stop the container when explicitly requested:

  ```shell
  container rm -f rustpython-test
  ```
