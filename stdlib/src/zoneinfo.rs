pub(crate) use _zoneinfo::make_module;

#[pymodule]
mod _zoneinfo {
    #[pyattr]
    #[pyclass(name)]
    struct ZoneInfo {
        // PyDateTime_TZInfo base;
        // PyObject *key;
        // PyObject *file_repr;
        // PyObject *weakreflist;
        // size_t num_transitions;
        num_transitions: usize,
        // size_t num_ttinfos;
        num_ttinfos: usize,
        // int64_t *trans_list_utc;
        // int64_t *trans_list_wall[2];
        // _ttinfo **trans_ttinfos;  // References to the ttinfo for each transition
        // _ttinfo *ttinfo_before;
        // _tzrule tzrule_after;
        // _ttinfo *_ttinfos;  // Unique array of ttinfos for ease of deallocation
        // unsigned char fixed_offset;
        fixed_offset: u8,
        // unsigned char source;
        source: u8,
    }

    #[pyclass]
    impl ZoneInfo {

    }
}
