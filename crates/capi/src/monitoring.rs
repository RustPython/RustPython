use crate::{PyObject, pystate::with_vm, util::FfiPtrExt};
use core::ffi::c_int;
use rustpython_vm::stdlib::sys::monitoring_capi;

pub use monitoring_capi::EventState as PyMonitoringState;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyMonitoring_EnterScope(
    state_array: *mut PyMonitoringState,
    version: *mut u64,
    event_types: *const u8,
    length: isize,
) -> c_int {
    with_vm(|vm| {
        let length = usize::try_from(length)
            .map_err(|_| vm.new_value_error("invalid monitoring scope length"))?;
        let (states, events) = if length == 0 {
            (&mut [][..], &[][..])
        } else {
            unsafe {
                (
                    core::slice::from_raw_parts_mut(state_array, length),
                    core::slice::from_raw_parts(event_types, length),
                )
            }
        };
        monitoring_capi::enter_scope(vm, states, unsafe { &mut *version }, events)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyMonitoring_ExitScope() -> c_int {
    monitoring_capi::exit_scope();
    0
}

macro_rules! fire_event {
    ($name:ident, $event:expr $(, $arg:ident)*) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(
            state: *mut PyMonitoringState,
            codelike: *mut PyObject,
            offset: i32,
            $($arg: *mut PyObject,)*
        ) -> c_int {
            with_vm(|vm| {
                // Exception setup precedes reading the codelike or arguments,
                // matching CPython's exception_event_setup error ordering.
                if matches!($event, 11..=15 | 17)
                    && unsafe { (*state).active } != 0
                    && vm.raised_exception().is_none()
                {
                    return Err(vm.new_value_error(format!(
                        "Firing event {} with no exception set", $event
                    )));
                }
                unsafe { monitoring_capi::fire_event(
                    vm,
                    state,
                    codelike.assume_borrowed(),
                    offset,
                    $event,
                    &[$($arg.assume_borrowed().to_owned(),)*],
                ) }
            })
        }
    };
}

fire_event!(_PyMonitoring_FirePyStartEvent, 0);
fire_event!(_PyMonitoring_FirePyResumeEvent, 1);
fire_event!(_PyMonitoring_FirePyReturnEvent, 2, retval);
fire_event!(_PyMonitoring_FirePyYieldEvent, 3, retval);
fire_event!(_PyMonitoring_FireCallEvent, 4, callable, arg0);
fire_event!(_PyMonitoring_FireJumpEvent, 7, target_offset);
fire_event!(_PyMonitoring_FireBranchLeftEvent, 8, target_offset);
fire_event!(_PyMonitoring_FireBranchRightEvent, 9, target_offset);
// The deprecated BRANCH entry point fires BRANCH_RIGHT.
fire_event!(_PyMonitoring_FireBranchEvent, 9, target_offset);
fire_event!(_PyMonitoring_FireRaiseEvent, 11);
fire_event!(_PyMonitoring_FireExceptionHandledEvent, 12);
fire_event!(_PyMonitoring_FirePyUnwindEvent, 13);
fire_event!(_PyMonitoring_FirePyThrowEvent, 14);
fire_event!(_PyMonitoring_FireReraiseEvent, 15);
fire_event!(_PyMonitoring_FireCReturnEvent, 16, retval);
fire_event!(_PyMonitoring_FireCRaiseEvent, 17);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyMonitoring_FireLineEvent(
    state: *mut PyMonitoringState,
    codelike: *mut PyObject,
    offset: i32,
    lineno: c_int,
) -> c_int {
    with_vm(|vm| unsafe {
        monitoring_capi::fire_event(
            vm,
            state,
            codelike.assume_borrowed(),
            offset,
            5,
            &[vm.ctx.new_int(lineno).into()],
        )
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _PyMonitoring_FireStopIterationEvent(
    state: *mut PyMonitoringState,
    codelike: *mut PyObject,
    offset: i32,
    value: *mut PyObject,
) -> c_int {
    with_vm(|vm| unsafe {
        let value = value.assume_borrowed_or_opt().map(PyObject::to_owned);
        monitoring_capi::fire_event(
            vm,
            state,
            codelike.assume_borrowed(),
            offset,
            10,
            value.as_slice(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyo3::{prelude::*, types::PyModule};

    #[test]
    fn native_scope_and_disable() {
        // Monitoring callbacks are interpreter-wide. Other C-API unit tests
        // execute Python concurrently in the shared main interpreter.
        const CHILD_MARKER: &str = "RUSTPYTHON_CAPI_MONITORING_TEST_CHILD";
        if std::env::var_os(CHILD_MARKER).is_none() {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "monitoring::tests::native_scope_and_disable",
                    "--nocapture",
                ])
                .env(CHILD_MARKER, "1")
                .status()
                .unwrap();
            assert!(
                status.success(),
                "isolated monitoring test failed: {status}"
            );
            return;
        }
        Python::attach(|py| {
            let fixture = PyModule::from_code(
                py,
                c"import sys\nseen = []\ndef callback(code, offset):\n    seen.append((code, offset))\n    return sys.monitoring.DISABLE\n",
                c"monitoring_fixture.py",
                c"monitoring_fixture",
            )
            .unwrap();
            let monitoring = py.import("sys").unwrap().getattr("monitoring").unwrap();
            monitoring
                .call_method1("use_tool_id", (5, "capi scope"))
                .unwrap();
            monitoring
                .call_method1(
                    "register_callback",
                    (5, 1, fixture.getattr("callback").unwrap()),
                )
                .unwrap();
            monitoring.call_method1("set_events", (5, 1)).unwrap();
            let mut state = PyMonitoringState::default();
            let mut version = 0;
            assert_eq!(
                unsafe { PyMonitoring_EnterScope(&mut state, &mut version, &0, 1) },
                0
            );
            assert_eq!(state.active, 1 << 5);
            let codelike = py.None();
            assert_eq!(
                unsafe { _PyMonitoring_FirePyStartEvent(&mut state, codelike.as_ptr().cast(), 42) },
                0
            );
            assert_eq!(state.active, 0);
            let seen: Vec<(Option<i32>, i32)> = fixture.getattr("seen").unwrap().extract().unwrap();
            assert_eq!(seen, [(None, 42)]);
            assert_eq!(
                unsafe { PyMonitoring_EnterScope(&mut state, &mut version, &0, 1) },
                0
            );
            assert_eq!(state.active, 0);
            monitoring.call_method0("restart_events").unwrap();
            assert_eq!(
                unsafe { PyMonitoring_EnterScope(&mut state, &mut version, &0, 1) },
                0
            );
            assert_eq!(state.active, 1 << 5);
            assert_eq!(PyMonitoring_ExitScope(), 0);
            monitoring.call_method1("free_tool_id", (5,)).unwrap();
        });
    }
}
