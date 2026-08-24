//! Compiling functions to native code without being asked.
//!
//! `__jit__()` compiles one function on request and reports why it could not.
//! The AOT path instead tries every function on its first call, which changes
//! what the rules have to be: a function compiled behind the caller's back
//! must not answer differently from the interpreter, and must not cost
//! anything once it turns out it cannot be compiled.

use super::PyFunction;
use crate::{AsObject, Py, PyResult, VirtualMachine, builtins::PyCode};
use core::sync::atomic::{AtomicU64, Ordering::Relaxed};
use rustpython_jit::{CompiledCode, Safety};

/// What the AOT path has done so far in this interpreter. Reported by
/// `sys._jit._stats()`, which is what tells us how much of a real workload
/// the current set of supported operations actually reaches.
#[derive(Debug, Default)]
pub struct AotStats {
    pub compiled: AtomicU64,
    pub rejected: AtomicU64,
    pub deoptimized: AtomicU64,
}

/// Nothing has been attempted yet.
pub(super) const UNTRIED: u8 = 0;
/// Compiled by the AOT path. Gives up on the first argument mismatch.
pub(super) const COMPILED_AUTO: u8 = 1;
/// Compiled by an explicit `__jit__()`. Keeps its code across mismatches.
pub(super) const COMPILED_MANUAL: u8 = 2;
/// Will not be compiled again unless `__jit__()` asks.
pub(super) const REJECTED: u8 = 3;

const PRECHECK_ELIGIBLE: u8 = 1;
const PRECHECK_REJECTED: u8 = 2;

/// Whether the backend could compile this code object, remembered on the code
/// object itself so the bytecode scan runs once however many function objects
/// are built from it - a decorator or a closure factory can build thousands.
fn code_is_eligible(code: &Py<PyCode>) -> bool {
    match code.aot_precheck.load(Relaxed) {
        PRECHECK_ELIGIBLE => true,
        PRECHECK_REJECTED => false,
        _ => {
            let eligible = rustpython_jit::supports_code(&code.code);
            let verdict = if eligible {
                PRECHECK_ELIGIBLE
            } else {
                PRECHECK_REJECTED
            };
            code.aot_precheck.store(verdict, Relaxed);
            eligible
        }
    }
}

/// Discard the reason a speculative compile failed, unless it is something
/// the program has to see.
///
/// Reading annotations runs `__annotate__`, which is Python code: a forward
/// reference raises NameError and is none of our business, but a
/// KeyboardInterrupt or SystemExit that happens to land there belongs to the
/// program, not to a compile attempt nobody asked for.
fn ignore_speculative_error<T>(result: PyResult<T>, vm: &VirtualMachine) -> PyResult<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(err) if err.fast_isinstance(vm.ctx.exceptions.exception_type) => Ok(None),
        Err(err) => Err(err),
    }
}

fn try_compile(func: &Py<PyFunction>, vm: &VirtualMachine) -> PyResult<Option<CompiledCode>> {
    let code: &Py<PyCode> = &func.code;
    if !code_is_eligible(code) {
        return Ok(None);
    }

    let Some(arg_types) = ignore_speculative_error(super::jit::get_jit_arg_types(func, vm), vm)?
    else {
        return Ok(None);
    };
    let Some(ret_type) = ignore_speculative_error(super::jit::jit_ret_type(func, vm), vm)? else {
        return Ok(None);
    };

    Ok(vm
        .state
        .jit_engine
        .compile(&code.code, &arg_types, ret_type, Safety::Strict)
        .ok())
}

/// Give `func` its one automatic compile attempt and return its new state.
pub(super) fn compile_on_first_call(func: &Py<PyFunction>, vm: &VirtualMachine) -> PyResult<u8> {
    // Claim the function before running anything that can re-enter it:
    // evaluating `__annotate__` calls Python, which can reach this same
    // function, and a reentrant attempt has to interpret rather than recurse.
    func.jit_state.store(REJECTED, Relaxed);

    let Some(compiled) = try_compile(func, vm)? else {
        vm.state.aot_stats.rejected.fetch_add(1, Relaxed);
        return Ok(REJECTED);
    };
    *func.jitted_code.lock() = Some(compiled);
    func.jit_state.store(COMPILED_AUTO, Relaxed);
    vm.state.aot_stats.compiled.fetch_add(1, Relaxed);
    Ok(COMPILED_AUTO)
}
