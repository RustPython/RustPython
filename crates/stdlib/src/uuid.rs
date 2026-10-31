pub(crate) use _uuid::module_def;

#[pymodule]
mod _uuid {
    use crate::{builtins::PyNone, vm::VirtualMachine};
    use std::sync::OnceLock;
    use uuid::{ContextV1, Uuid, timestamp::Timestamp};

    fn hardware_node_id() -> Option<&'static [u8; 6]> {
        static HARDWARE_NODE_ID: OnceLock<Option<[u8; 6]>> = OnceLock::new();
        HARDWARE_NODE_ID
            .get_or_init(rustpython_host_env::socket::mac_address)
            .as_ref()
    }

    fn get_node_id() -> [u8; 6] {
        // os_random is expensive, but this is only ever called once
        hardware_node_id()
            .copied()
            .unwrap_or_else(rustpython_common::rand::os_random::<6>)
    }

    #[pyfunction]
    fn generate_time_safe() -> (Vec<u8>, PyNone) {
        static CONTEXT: ContextV1 = ContextV1::new(0);
        let ts = Timestamp::now(&CONTEXT);

        static NODE_ID: OnceLock<[u8; 6]> = OnceLock::new();
        let unique_node_id = NODE_ID.get_or_init(get_node_id);

        (Uuid::new_v1(ts, unique_node_id).as_bytes().to_vec(), PyNone)
    }

    #[pyattr]
    fn has_uuid_generate_time_safe(_vm: &VirtualMachine) -> u32 {
        0
    }

    #[pyattr]
    fn has_stable_extractable_node(_vm: &VirtualMachine) -> u32 {
        hardware_node_id().is_some().into()
    }
}
