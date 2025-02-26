pub(crate) use _uuid::make_module;

#[pymodule]
mod _uuid {
    use crate::{builtins::PyNone, vm::VirtualMachine};
    use mac_address::get_mac_address;
    use once_cell::sync::OnceCell;
    use uuid::{Context, Uuid, timestamp::Timestamp};

    fn get_node_id() -> [u8; 6] {
        match get_mac_address() {
            Ok(Some(_ma)) => get_mac_address().unwrap().unwrap().bytes(),
            _ => rand::random::<[u8; 6]>(),
        }
    }

    #[pyfunction]
    fn generate_time_safe() -> (Vec<u8>, PyNone) {
        static CONTEXT: Context = Context::new(0);
        let ts = Timestamp::now(&CONTEXT);

        static NODE_ID: OnceCell<[u8; 6]> = OnceCell::new();
        let unique_node_id = NODE_ID.get_or_init(get_node_id);

        (Uuid::new_v1(ts, unique_node_id).as_bytes().to_vec(), PyNone)
    }

    #[pyattr]
    fn has_uuid_generate_time_safe(_vm: &VirtualMachine) -> u32 {
        0
    }
}
