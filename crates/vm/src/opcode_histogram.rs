//! Dispatch-loop opcode histogram, used to rank superinstruction candidates and
//! to see which generic (unspecialized) opcodes still dominate.
//!
//! Compiled only under the `opcode-histogram` feature; the dispatch loop's call
//! into [`record`] is `#[cfg]`-gated, so a default build carries no trace of it.
//! Counts land in the file named by `RUSTPYTHON_OPCODE_HISTOGRAM` (stderr otherwise) when the
//! process exits.

// The dump runs from an `atexit` hook after the VM is gone, so it reads its
// destination straight from the process environment rather than through
// `rustpython_host_env`. Measurement-only code behind a non-default feature.
#![allow(clippy::disallowed_methods)]

use core::sync::atomic::{AtomicU64, Ordering::Relaxed};
use rustpython_compiler_core::bytecode::Opcode;
use std::{io::Write, sync::Once};

const N: usize = 256;

/// `PAIRS[prev * 256 + op]` counts how often `op` was dispatched directly after
/// `prev` in the same frame. Row 0 doubles as "first instruction of a frame",
/// since `Cache` (opcode 0) is never itself dispatched.
static PAIRS: [AtomicU64; N * N] = [const { AtomicU64::new(0) }; N * N];

static INSTALL: Once = Once::new();

#[inline]
pub fn record(prev: u8, op: u8) {
    INSTALL.call_once(|| unsafe {
        libc::atexit(dump_at_exit);
    });
    PAIRS[prev as usize * N + op as usize].fetch_add(1, Relaxed);
}

extern "C" fn dump_at_exit() {
    let mut out = Vec::new();
    let _ = writeln!(out, "# opcode histogram: prev op count");
    for prev in 0..N {
        for op in 0..N {
            let count = PAIRS[prev * N + op].load(Relaxed);
            if count == 0 {
                continue;
            }
            let _ = writeln!(out, "{} {} {count}", name(prev as u8), name(op as u8));
        }
    }
    match std::env::var_os("RUSTPYTHON_OPCODE_HISTOGRAM") {
        Some(path) => {
            let _ = std::fs::write(path, &out);
        }
        None => {
            let _ = std::io::stderr().write_all(&out);
        }
    }
}

fn name(value: u8) -> String {
    match Opcode::try_from(value) {
        Ok(opcode) => format!("{opcode:?}"),
        Err(_) => format!("Unknown{value}"),
    }
}
