pub(crate) use _uuid::module_def;

#[pymodule]
mod _uuid {
    #[cfg(not(windows))]
    use crate::builtins::PyNone;
    use crate::vm::VirtualMachine;
    #[cfg(windows)]
    use crate::vm::{PyResult, convert::ToPyObject};
    #[cfg(not(windows))]
    use std::sync::OnceLock;
    #[cfg(not(windows))]
    use uuid::{ContextV1, Uuid, timestamp::Timestamp};

    #[cfg(not(windows))]
    fn hardware_node_id() -> Option<&'static [u8; 6]> {
        static HARDWARE_NODE_ID: OnceLock<Option<[u8; 6]>> = OnceLock::new();
        HARDWARE_NODE_ID
            .get_or_init(rustpython_host_env::socket::mac_address)
            .as_ref()
    }

    #[cfg(not(windows))]
    fn get_node_id() -> [u8; 6] {
        // os_random is expensive, but this is only ever called once
        hardware_node_id()
            .copied()
            .unwrap_or_else(rustpython_common::rand::os_random::<6>)
    }

    #[cfg(not(windows))]
    #[pyfunction]
    fn generate_time_safe() -> (Vec<u8>, PyNone) {
        static CONTEXT: ContextV1 = ContextV1::new(0);
        let ts = Timestamp::now(&CONTEXT);

        static NODE_ID: OnceLock<[u8; 6]> = OnceLock::new();
        let unique_node_id = NODE_ID.get_or_init(get_node_id);

        (Uuid::new_v1(ts, unique_node_id).as_bytes().to_vec(), PyNone)
    }

    #[cfg(windows)]
    #[pyfunction(name = "UuidCreate")]
    fn uuid_create(vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        use rustpython_host_env::uuid;

        let uuid = uuid::create_sequential();
        match uuid.status {
            uuid::STATUS_OK | uuid::STATUS_LOCAL_ONLY | uuid::STATUS_NO_ADDRESS => {
                Ok(uuid.bytes.to_vec())
            }
            status => {
                let errno = rustpython_host_env::os::winerror_to_errno(status);
                let exc = vm.new_errno_error(errno, uuid::format_error_message(status));
                exc.as_object()
                    .set_attr("winerror", status.to_pyobject(vm), vm)?;
                Err(exc.upcast())
            }
        }
    }

    #[pyattr]
    fn has_uuid_generate_time_safe(_vm: &VirtualMachine) -> u32 {
        0
    }

    #[pyattr]
    fn has_stable_extractable_node(_vm: &VirtualMachine) -> u32 {
        #[cfg(windows)]
        let has_stable_node = rustpython_host_env::uuid::has_stable_node();
        #[cfg(not(windows))]
        let has_stable_node = hardware_node_id().is_some();
        has_stable_node.into()
    }
}
