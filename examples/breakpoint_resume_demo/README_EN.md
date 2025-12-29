# Breakpoint/Resume Demo (VM Level)

This demo uses RustPython's VM-level checkpoint/resume prototype: when execution
reaches a checkpoint line, it saves the VM state and exits; on the next run it
loads the snapshot and continues execution right after the checkpoint.

## How to run

First run (hits checkpoint 1 and exits):

```
./target/release/pvm examples/breakpoint_resume_demo/demo.py
```

Second run (resume from checkpoint 1, continue to checkpoint 2 and exit):

```
./target/release/pvm --resume examples/breakpoint_resume_demo/demo.rpsnap examples/breakpoint_resume_demo/demo.py
```

Third run (resume from checkpoint 2, finish, and clean up the snapshot file):

```
./target/release/pvm --resume examples/breakpoint_resume_demo/demo.rpsnap examples/breakpoint_resume_demo/demo.py
```

## Test program

You can run the automated test script to validate checkpoint/resume:

```
./target/release/pvm examples/breakpoint_resume_demo/test_checkpoint.py
```

To specify a custom pvm binary path:

```
./target/release/pvm examples/breakpoint_resume_demo/test_checkpoint.py --bin /path/to/pvm
```

## Snapshot file

The checkpoint state is saved to:

```
examples/breakpoint_resume_demo/demo.rpsnap
```

This is a binary file serialized with `marshal`. Do not edit it by hand.

## Limitations (prototype boundaries)

This is a "minimum viable" VM-level checkpoint/resume prototype, with these
constraints:

- Only supports top-level (module-level) code; no checkpoints inside functions
  or methods.
- A checkpoint must be a standalone statement (e.g. `checkpoint()` on its own
  line), not embedded in assignments or expressions.
- A checkpoint cannot be inside loops/try/finally or other control-flow blocks
  (the block stack must be empty).
- Only simple, `marshal`-serializable types are saved (int/str/list/dict, etc.).
  Module objects, classes, file handles, etc. are not saved and must be
  re-imported or rebuilt after resume.

These limitations are intentional so the demo shows "checkpoint/resume within
the same code block" clearly.
