// TLS record observation adapter. The engine lives in rustpython-common.

use rustpython_vm::{VirtualMachine, builtins::PyBaseExceptionRef};

pub(super) use rustpython_host_env::ssl::msg::{MsgState, tls12_unique};

pub(super) fn unknown_binding_type_error(cb_type: &str, vm: &VirtualMachine) -> PyBaseExceptionRef {
    vm.new_value_error(format!("'{cb_type}' channel binding type not implemented"))
}
