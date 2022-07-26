pub(crate) use _uuid::make_module;

#[pymodule]
mod _uuid {
    use crate::{builtins::PyNone, vm::VirtualMachine};
    use mac_address::get_mac_address;
    use once_cell::sync::OnceCell;
    use rand::Rng;
    use std::time::{Duration, SystemTime};
    use uuid::{
        v1::{Context, Timestamp},
        Uuid,
    };

    static NODE_ID: OnceCell<[u8; 6]> = OnceCell::new();

    fn get_node_id() -> &'static [u8; 6] {
        match get_mac_address() {
            Ok(Some(_ma)) => {
                let node_id = NODE_ID.get_or_init(|| get_mac_address().unwrap().unwrap().bytes());
                node_id
            }
            Ok(None) => {
                let node_id = NODE_ID.get_or_init(|| rand::thread_rng().gen::<[u8; 6]>());
                node_id
            }
            Err(_e) => {
                let node_id = NODE_ID.get_or_init(|| rand::thread_rng().gen::<[u8; 6]>());
                node_id
            }
        }
    }

    pub fn now_unix_duration() -> Duration {
        use std::time::UNIX_EPOCH;

        let now = SystemTime::now();
        now.duration_since(UNIX_EPOCH)
            .expect("SystemTime before UNIX EPOCH!")
    }

    #[pyfunction]
    fn generate_time_safe(_vm: &VirtualMachine) -> (Vec<u8>, PyNone) {
        let now = now_unix_duration();
        static CONTEXT: Context = Context::new(0);
        let ts = Timestamp::from_unix(&CONTEXT, now.as_secs(), now.subsec_nanos());

        let node_id = get_node_id();

        (Uuid::new_v1(ts, &node_id).as_bytes().to_vec(), PyNone)
    }
}
