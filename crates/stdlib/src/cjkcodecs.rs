//! The VM adapter for the CJK codec engines in `rustpython-common`.
//!
//! Laid out like `Modules/cjkcodecs/`: `multibytecodec` holds the codec objects
//! and the encode/decode drivers, and one `_codecs_*` module per region hands
//! out the codecs it registers.

pub(crate) mod multibytecodec;

/// Declares a `_codecs_*` module exposing `getcodec`, after
/// `cjkcodecs.h::I_AM_A_MODULE_FOR`.
macro_rules! codecs_module {
    ($module:ident, [$($name:literal),+ $(,)?]) => {
        #[pymodule]
        pub(crate) mod $module {
            use crate::cjkcodecs::multibytecodec;
            use crate::vm::{PyObjectRef, PyResult, VirtualMachine};

            const CODECS: &[&'static str] = &[$($name),+];

            #[pyfunction]
            fn getcodec(encoding: PyObjectRef, vm: &VirtualMachine) -> PyResult {
                multibytecodec::get_codec(CODECS, &encoding, vm)
            }
        }
    };
}

codecs_module!(_codecs_cn, ["gb2312", "gbk", "gb18030", "hz"]);
codecs_module!(_codecs_hk, ["big5hkscs"]);
codecs_module!(
    _codecs_iso2022,
    [
        "iso2022_kr",
        "iso2022_jp",
        "iso2022_jp_1",
        "iso2022_jp_2",
        "iso2022_jp_2004",
        "iso2022_jp_3",
        "iso2022_jp_ext",
    ]
);
codecs_module!(
    _codecs_jp,
    [
        "shift_jis",
        "cp932",
        "euc_jp",
        "shift_jis_2004",
        "euc_jis_2004",
        "euc_jisx0213",
        "shift_jisx0213",
    ]
);
codecs_module!(_codecs_kr, ["euc_kr", "cp949", "johab"]);
codecs_module!(_codecs_tw, ["big5", "cp950"]);
