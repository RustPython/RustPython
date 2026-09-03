//! Compiling functions to native code without being asked.
//!
//! `__jit__()` compiles one function on request, from the types its
//! annotations declare, and reports why it could not. The AOT path instead
//! waits for a function to be called often enough to be worth compiling and
//! takes its types from the call in front of it, which changes what the rules
//! have to be: a function compiled behind the caller's back must not answer
//! differently from the interpreter, and must not cost anything once it turns
//! out it cannot be compiled.

use super::PyFunction;
use crate::{Py, VirtualMachine, builtins::PyCode, function::FuncArgs};
use core::sync::atomic::{AtomicU64, Ordering::Relaxed};
use rustpython_jit::{CompiledCode, Safety};

/// What the AOT path has done so far in this interpreter. Reported by
/// `sys._jit._stats()`, which is what tells us how much of a real workload
/// the current set of supported operations actually reaches. `rejected`
/// counts the functions the compiler was asked about, which is the warm ones
/// - a function called a few times and dropped is in neither number.
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

/// Calls a function makes before the compiler is asked to look at it.
///
/// Compiling on the first call spends the eligibility scan on every function a
/// program calls once, which is most of them, and gets nothing back. Waiting
/// also decides what to specialize on: the types come from the call that
/// crosses this line, and a function called this often is being called with
/// the types it is meant for.
const WARMUP_CALLS: u32 = 64;

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

fn try_compile(
    func: &Py<PyFunction>,
    func_args: &FuncArgs,
    vm: &VirtualMachine,
) -> Option<CompiledCode> {
    let code: &Py<PyCode> = &func.code;
    if !code_is_eligible(code) {
        return None;
    }

    // The return type is left to the compiler, which widens the signature to
    // whatever the returns it lowered produce.
    let arg_types = super::jit::observed_arg_types(func, func_args, vm).ok()?;
    vm.state
        .jit_engine
        .compile(&code.code, &arg_types, None, Safety::Strict)
        .ok()
}

/// Count this call, and once the function is warm give it its one automatic
/// compile attempt. Returns the function's new state.
pub(super) fn observe_call(func: &Py<PyFunction>, func_args: &FuncArgs, vm: &VirtualMachine) -> u8 {
    if func.jit_warmup.fetch_add(1, Relaxed) < WARMUP_CALLS {
        return UNTRIED;
    }

    // Claim the function before compiling, so that two threads crossing the
    // line together make one attempt rather than two.
    func.jit_state.store(REJECTED, Relaxed);

    let Some(compiled) = try_compile(func, func_args, vm) else {
        vm.state.aot_stats.rejected.fetch_add(1, Relaxed);
        return REJECTED;
    };
    *func.jitted_code.lock() = Some(compiled);
    func.jit_state.store(COMPILED_AUTO, Relaxed);
    vm.state.aot_stats.compiled.fetch_add(1, Relaxed);
    COMPILED_AUTO
}
