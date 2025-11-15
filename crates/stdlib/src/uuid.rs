pub(crate) use _uuid::make_module;

#[pymodule]
mod _uuid {
    use crate::{builtins::PyNone, vm::VirtualMachine};
    use mac_address::get_mac_address;
    use std::sync::OnceLock;
    use uuid::{Context, Uuid, timestamp::Timestamp};

    fn get_node_id() -> [u8; 6] {
        match get_mac_address() {
            Ok(Some(_ma)) => get_mac_address().unwrap().unwrap().bytes(),
            // os_random is expensive, but this is only ever called once
            _ => rustpython_common::rand::os_random::<6>(),
        }
    }

    #[pyfunction]
    fn generate_time_safe() -> (Vec<u8>, PyNone) {
        static CONTEXT: Context = Context::new(0);
        let ts = Timestamp::now(&CONTEXT);

        static NODE_ID: OnceLock<[u8; 6]> = OnceLock::new();
        let unique_node_id = NODE_ID.get_or_init(get_node_id);

        (Uuid::new_v1(ts, unique_node_id).as_bytes().to_vec(), PyNone)
    }

    #[pyattr]
    fn has_uuid_generate_time_safe(_vm: &VirtualMachine) -> u32 {
        0
    }

    #[pyattr(name = "has_stable_extractable_node")]
    const HAS_STABLE_EXTRACTABLE_NODE: bool = false;
}
