//! Slot definitions array
//!
//! This module provides a centralized array of all slot definitions,

use super::{PyComparisonOp, PyTypeSlots};
use crate::builtins::descriptor::SlotFunc;

/// Slot operation type
///
/// Used to distinguish between different operations that share the same slot:
/// - RichCompare: Lt, Le, Eq, Ne, Gt, Ge
/// - Binary ops: Left (__add__) vs Right (__radd__)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotOp {
    // RichCompare operations
    Lt,
    Le,
    Eq,
    Ne,
    Gt,
    Ge,
    // Binary operation direction
    Left,
    Right,
    // Setter vs Deleter
    Delete,
}

impl SlotOp {
    /// Convert to PyComparisonOp if this is a comparison operation
    pub fn as_compare_op(&self) -> Option<PyComparisonOp> {
        match self {
            Self::Lt => Some(PyComparisonOp::Lt),
            Self::Le => Some(PyComparisonOp::Le),
            Self::Eq => Some(PyComparisonOp::Eq),
            Self::Ne => Some(PyComparisonOp::Ne),
            Self::Gt => Some(PyComparisonOp::Gt),
            Self::Ge => Some(PyComparisonOp::Ge),
            _ => None,
        }
    }

    /// Check if this is a right operation (__radd__, __rsub__, etc.)
    pub fn is_right(&self) -> bool {
        matches!(self, Self::Right)
    }
}

/// Slot definition entry
#[derive(Clone, Copy)]
pub struct SlotDef {
    /// Method name ("__init__", "__add__", etc.)
    pub name: &'static str,

    /// Slot accessor (which slot field to access)
    pub accessor: SlotAccessor,

    /// Operation type (for shared slots like RichCompare, binary ops)
    pub op: Option<SlotOp>,

    /// Documentation string
    pub doc: &'static str,
}

/// Slot accessor
///
/// Values match CPython's Py_* slot IDs from typeslots.h.
/// Unused slots are included for value reservation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum SlotAccessor {
    // Buffer protocol (1-2) - Reserved, not used in RustPython
    BfGetBuffer = 1,
    BfReleaseBuffer = 2,

    // Mapping protocol (3-5)
    MpAssSubscript = 3,
    MpLength = 4,
    MpSubscript = 5,

    // Number protocol (6-38)
    NbAbsolute = 6,
    NbAdd = 7,
    NbAnd = 8,
    NbBool = 9,
    NbDivmod = 10,
    NbFloat = 11,
    NbFloorDivide = 12,
    NbIndex = 13,
    NbInplaceAdd = 14,
    NbInplaceAnd = 15,
    NbInplaceFloorDivide = 16,
    NbInplaceLshift = 17,
    NbInplaceMultiply = 18,
    NbInplaceOr = 19,
    NbInplacePower = 20,
    NbInplaceRemainder = 21,
    NbInplaceRshift = 22,
    NbInplaceSubtract = 23,
    NbInplaceTrueDivide = 24,
    NbInplaceXor = 25,
    NbInt = 26,
    NbInvert = 27,
    NbLshift = 28,
    NbMultiply = 29,
    NbNegative = 30,
    NbOr = 31,
    NbPositive = 32,
    NbPower = 33,
    NbRemainder = 34,
    NbRshift = 35,
    NbSubtract = 36,
    NbTrueDivide = 37,
    NbXor = 38,

    // Sequence protocol (39-46)
    SqAssItem = 39,
    SqConcat = 40,
    SqContains = 41,
    SqInplaceConcat = 42,
    SqInplaceRepeat = 43,
    SqItem = 44,
    SqLength = 45,
    SqRepeat = 46,

    // Type slots (47-74)
    TpAlloc = 47, // Reserved
    TpBase = 48,  // Reserved
    TpBases = 49, // Reserved
    TpCall = 50,
    TpClear = 51,   // Reserved
    TpDealloc = 52, // Reserved
    TpDel = 53,
    TpDescrGet = 54,
    TpDescrSet = 55,
    TpDoc = 56,     // Reserved
    TpGetattr = 57, // Reserved (use TpGetattro)
    TpGetattro = 58,
    TpHash = 59,
    TpInit = 60,
    TpIsGc = 61, // Reserved
    TpIter = 62,
    TpIternext = 63,
    TpMethods = 64, // Reserved
    TpNew = 65,
    TpRepr = 66,
    TpRichcompare = 67,
    TpSetattr = 68, // Reserved (use TpSetattro)
    TpSetattro = 69,
    TpStr = 70,
    TpTraverse = 71, // Reserved
    TpMembers = 72,  // Reserved
    TpGetset = 73,   // Reserved
    TpFree = 74,     // Reserved

    // Number protocol additions (75-76)
    NbMatrixMultiply = 75,
    NbInplaceMatrixMultiply = 76,

    // Async protocol (77-81) - Reserved for future
    AmAwait = 77,
    AmAiter = 78,
    AmAnext = 79,
    TpFinalize = 80,
    AmSend = 81,
}

impl SlotAccessor {
    /// Check if this accessor is for a reserved/unused slot
    pub fn is_reserved(&self) -> bool {
        matches!(
            self,
            Self::BfGetBuffer
                | Self::BfReleaseBuffer
                | Self::TpAlloc
                | Self::TpBase
                | Self::TpBases
                | Self::TpClear
                | Self::TpDealloc
                | Self::TpDoc
                | Self::TpGetattr
                | Self::TpIsGc
                | Self::TpMethods
                | Self::TpSetattr
                | Self::TpTraverse
                | Self::TpMembers
                | Self::TpGetset
                | Self::TpFree
                | Self::TpFinalize
                | Self::AmAwait
                | Self::AmAiter
                | Self::AmAnext
                | Self::AmSend
        )
    }

    /// Check if this is a number binary operation slot
    pub fn is_number_binary(&self) -> bool {
        matches!(
            self,
            Self::NbAdd
                | Self::NbSubtract
                | Self::NbMultiply
                | Self::NbRemainder
                | Self::NbDivmod
                | Self::NbPower
                | Self::NbLshift
                | Self::NbRshift
                | Self::NbAnd
                | Self::NbXor
                | Self::NbOr
                | Self::NbFloorDivide
                | Self::NbTrueDivide
                | Self::NbMatrixMultiply
        )
    }

    /// Check if this accessor refers to a shared slot
    ///
    /// Shared slots are used by multiple dunder methods:
    /// - TpSetattro: __setattr__ and __delattr__
    /// - TpRichcompare: __lt__, __le__, __eq__, __ne__, __gt__, __ge__
    /// - TpDescrSet: __set__ and __delete__
    /// - SqAssItem/MpAssSubscript: __setitem__ and __delitem__
    /// - Number binaries: __add__ and __radd__, etc.
    pub fn is_shared_slot(&self) -> bool {
        matches!(
            self,
            Self::TpSetattro
                | Self::TpRichcompare
                | Self::TpDescrSet
                | Self::SqAssItem
                | Self::MpAssSubscript
        ) || self.is_number_binary()
    }

    /// Get underlying slot field name for debugging
    pub fn slot_name(&self) -> &'static str {
        match self {
            Self::BfGetBuffer => "bf_getbuffer",
            Self::BfReleaseBuffer => "bf_releasebuffer",
            Self::MpAssSubscript => "mp_ass_subscript",
            Self::MpLength => "mp_length",
            Self::MpSubscript => "mp_subscript",
            Self::NbAbsolute => "nb_absolute",
            Self::NbAdd => "nb_add",
            Self::NbAnd => "nb_and",
            Self::NbBool => "nb_bool",
            Self::NbDivmod => "nb_divmod",
            Self::NbFloat => "nb_float",
            Self::NbFloorDivide => "nb_floor_divide",
            Self::NbIndex => "nb_index",
            Self::NbInplaceAdd => "nb_inplace_add",
            Self::NbInplaceAnd => "nb_inplace_and",
            Self::NbInplaceFloorDivide => "nb_inplace_floor_divide",
            Self::NbInplaceLshift => "nb_inplace_lshift",
            Self::NbInplaceMultiply => "nb_inplace_multiply",
            Self::NbInplaceOr => "nb_inplace_or",
            Self::NbInplacePower => "nb_inplace_power",
            Self::NbInplaceRemainder => "nb_inplace_remainder",
            Self::NbInplaceRshift => "nb_inplace_rshift",
            Self::NbInplaceSubtract => "nb_inplace_subtract",
            Self::NbInplaceTrueDivide => "nb_inplace_true_divide",
            Self::NbInplaceXor => "nb_inplace_xor",
            Self::NbInt => "nb_int",
            Self::NbInvert => "nb_invert",
            Self::NbLshift => "nb_lshift",
            Self::NbMultiply => "nb_multiply",
            Self::NbNegative => "nb_negative",
            Self::NbOr => "nb_or",
            Self::NbPositive => "nb_positive",
            Self::NbPower => "nb_power",
            Self::NbRemainder => "nb_remainder",
            Self::NbRshift => "nb_rshift",
            Self::NbSubtract => "nb_subtract",
            Self::NbTrueDivide => "nb_true_divide",
            Self::NbXor => "nb_xor",
            Self::SqAssItem => "sq_ass_item",
            Self::SqConcat => "sq_concat",
            Self::SqContains => "sq_contains",
            Self::SqInplaceConcat => "sq_inplace_concat",
            Self::SqInplaceRepeat => "sq_inplace_repeat",
            Self::SqItem => "sq_item",
            Self::SqLength => "sq_length",
            Self::SqRepeat => "sq_repeat",
            Self::TpAlloc => "tp_alloc",
            Self::TpBase => "tp_base",
            Self::TpBases => "tp_bases",
            Self::TpCall => "tp_call",
            Self::TpClear => "tp_clear",
            Self::TpDealloc => "tp_dealloc",
            Self::TpDel => "tp_del",
            Self::TpDescrGet => "tp_descr_get",
            Self::TpDescrSet => "tp_descr_set",
            Self::TpDoc => "tp_doc",
            Self::TpGetattr => "tp_getattr",
            Self::TpGetattro => "tp_getattro",
            Self::TpHash => "tp_hash",
            Self::TpInit => "tp_init",
            Self::TpIsGc => "tp_is_gc",
            Self::TpIter => "tp_iter",
            Self::TpIternext => "tp_iternext",
            Self::TpMethods => "tp_methods",
            Self::TpNew => "tp_new",
            Self::TpRepr => "tp_repr",
            Self::TpRichcompare => "tp_richcompare",
            Self::TpSetattr => "tp_setattr",
            Self::TpSetattro => "tp_setattro",
            Self::TpStr => "tp_str",
            Self::TpTraverse => "tp_traverse",
            Self::TpMembers => "tp_members",
            Self::TpGetset => "tp_getset",
            Self::TpFree => "tp_free",
            Self::NbMatrixMultiply => "nb_matrix_multiply",
            Self::NbInplaceMatrixMultiply => "nb_inplace_matrix_multiply",
            Self::AmAwait => "am_await",
            Self::AmAiter => "am_aiter",
            Self::AmAnext => "am_anext",
            Self::TpFinalize => "tp_finalize",
            Self::AmSend => "am_send",
        }
    }

    /// Extract the raw function pointer from a SlotFunc if it matches this accessor's type
    pub fn extract_from_slot_func(&self, slot_func: &SlotFunc) -> bool {
        match self {
            // Type slots
            Self::TpHash => matches!(slot_func, SlotFunc::Hash(_)),
            Self::TpRepr => matches!(slot_func, SlotFunc::Repr(_)),
            Self::TpStr => matches!(slot_func, SlotFunc::Str(_)),
            Self::TpCall => matches!(slot_func, SlotFunc::Call(_)),
            Self::TpIter => matches!(slot_func, SlotFunc::Iter(_)),
            Self::TpIternext => matches!(slot_func, SlotFunc::IterNext(_)),
            Self::TpInit => matches!(slot_func, SlotFunc::Init(_)),
            Self::TpDel => matches!(slot_func, SlotFunc::Del(_)),
            Self::TpGetattro => matches!(slot_func, SlotFunc::GetAttro(_)),
            Self::TpSetattro => {
                matches!(slot_func, SlotFunc::SetAttro(_) | SlotFunc::DelAttro(_))
            }
            Self::TpDescrGet => matches!(slot_func, SlotFunc::DescrGet(_)),
            Self::TpDescrSet => {
                matches!(slot_func, SlotFunc::DescrSet(_) | SlotFunc::DescrDel(_))
            }
            Self::TpRichcompare => matches!(slot_func, SlotFunc::RichCompare(_, _)),

            // Number - Power (ternary)
            Self::NbPower | Self::NbInplacePower => {
                matches!(slot_func, SlotFunc::NumTernary(_))
            }
            // Number - Boolean
            Self::NbBool => matches!(slot_func, SlotFunc::NumBoolean(_)),
            // Number - Unary
            Self::NbNegative
            | Self::NbPositive
            | Self::NbAbsolute
            | Self::NbInvert
            | Self::NbInt
            | Self::NbFloat
            | Self::NbIndex => matches!(slot_func, SlotFunc::NumUnary(_)),
            // Number - Binary
            Self::NbAdd
            | Self::NbSubtract
            | Self::NbMultiply
            | Self::NbRemainder
            | Self::NbDivmod
            | Self::NbLshift
            | Self::NbRshift
            | Self::NbAnd
            | Self::NbXor
            | Self::NbOr
            | Self::NbFloorDivide
            | Self::NbTrueDivide
            | Self::NbMatrixMultiply
            | Self::NbInplaceAdd
            | Self::NbInplaceSubtract
            | Self::NbInplaceMultiply
            | Self::NbInplaceRemainder
            | Self::NbInplaceLshift
            | Self::NbInplaceRshift
            | Self::NbInplaceAnd
            | Self::NbInplaceXor
            | Self::NbInplaceOr
            | Self::NbInplaceFloorDivide
            | Self::NbInplaceTrueDivide
            | Self::NbInplaceMatrixMultiply => matches!(slot_func, SlotFunc::NumBinary(_)),

            // Sequence
            Self::SqLength => matches!(slot_func, SlotFunc::SeqLength(_)),
            Self::SqConcat | Self::SqInplaceConcat => matches!(slot_func, SlotFunc::SeqConcat(_)),
            Self::SqRepeat | Self::SqInplaceRepeat => matches!(slot_func, SlotFunc::SeqRepeat(_)),
            Self::SqItem => matches!(slot_func, SlotFunc::SeqItem(_)),
            Self::SqAssItem => matches!(slot_func, SlotFunc::SeqAssItem(_)),
            Self::SqContains => matches!(slot_func, SlotFunc::SeqContains(_)),

            // Mapping
            Self::MpLength => matches!(slot_func, SlotFunc::MapLength(_)),
            Self::MpSubscript => matches!(slot_func, SlotFunc::MapSubscript(_)),
            Self::MpAssSubscript => matches!(slot_func, SlotFunc::MapAssSubscript(_)),

            // New and reserved slots
            Self::TpNew => false,
            _ => false, // Reserved slots
        }
    }

    /// Inherit slot value from MRO
    pub fn inherit_from_mro(&self, typ: &crate::builtins::PyType) {
        // Note: typ.mro does NOT include typ itself
        let mro = typ.mro.read();

        macro_rules! inherit_main {
            ($slot:ident) => {{
                let inherited = mro.iter().find_map(|cls| cls.slots.$slot.load());
                typ.slots.$slot.store(inherited);
            }};
        }

        macro_rules! inherit_number {
            ($slot:ident) => {{
                let inherited = mro.iter().find_map(|cls| cls.slots.as_number.$slot.load());
                typ.slots.as_number.$slot.store(inherited);
            }};
        }

        macro_rules! inherit_sequence {
            ($slot:ident) => {{
                let inherited = mro
                    .iter()
                    .find_map(|cls| cls.slots.as_sequence.$slot.load());
                typ.slots.as_sequence.$slot.store(inherited);
            }};
        }

        macro_rules! inherit_mapping {
            ($slot:ident) => {{
                let inherited = mro.iter().find_map(|cls| cls.slots.as_mapping.$slot.load());
                typ.slots.as_mapping.$slot.store(inherited);
            }};
        }

        match self {
            // Type slots
            Self::TpHash => inherit_main!(hash),
            Self::TpRepr => inherit_main!(repr),
            Self::TpStr => inherit_main!(str),
            Self::TpCall => inherit_main!(call),
            Self::TpIter => inherit_main!(iter),
            Self::TpIternext => inherit_main!(iternext),
            Self::TpInit => inherit_main!(init),
            Self::TpNew => inherit_main!(new),
            Self::TpDel => inherit_main!(del),
            Self::TpGetattro => inherit_main!(getattro),
            Self::TpSetattro => inherit_main!(setattro),
            Self::TpDescrGet => inherit_main!(descr_get),
            Self::TpDescrSet => inherit_main!(descr_set),
            Self::TpRichcompare => inherit_main!(richcompare),

            // Number slots
            Self::NbAdd => inherit_number!(add),
            Self::NbSubtract => inherit_number!(subtract),
            Self::NbMultiply => inherit_number!(multiply),
            Self::NbRemainder => inherit_number!(remainder),
            Self::NbDivmod => inherit_number!(divmod),
            Self::NbPower => inherit_number!(power),
            Self::NbLshift => inherit_number!(lshift),
            Self::NbRshift => inherit_number!(rshift),
            Self::NbAnd => inherit_number!(and),
            Self::NbXor => inherit_number!(xor),
            Self::NbOr => inherit_number!(or),
            Self::NbFloorDivide => inherit_number!(floor_divide),
            Self::NbTrueDivide => inherit_number!(true_divide),
            Self::NbMatrixMultiply => inherit_number!(matrix_multiply),
            Self::NbInplaceAdd => inherit_number!(inplace_add),
            Self::NbInplaceSubtract => inherit_number!(inplace_subtract),
            Self::NbInplaceMultiply => inherit_number!(inplace_multiply),
            Self::NbInplaceRemainder => inherit_number!(inplace_remainder),
            Self::NbInplacePower => inherit_number!(inplace_power),
            Self::NbInplaceLshift => inherit_number!(inplace_lshift),
            Self::NbInplaceRshift => inherit_number!(inplace_rshift),
            Self::NbInplaceAnd => inherit_number!(inplace_and),
            Self::NbInplaceXor => inherit_number!(inplace_xor),
            Self::NbInplaceOr => inherit_number!(inplace_or),
            Self::NbInplaceFloorDivide => inherit_number!(inplace_floor_divide),
            Self::NbInplaceTrueDivide => inherit_number!(inplace_true_divide),
            Self::NbInplaceMatrixMultiply => inherit_number!(inplace_matrix_multiply),
            // Number unary
            Self::NbNegative => inherit_number!(negative),
            Self::NbPositive => inherit_number!(positive),
            Self::NbAbsolute => inherit_number!(absolute),
            Self::NbInvert => inherit_number!(invert),
            Self::NbBool => inherit_number!(boolean),
            Self::NbInt => inherit_number!(int),
            Self::NbFloat => inherit_number!(float),
            Self::NbIndex => inherit_number!(index),

            // Sequence slots
            Self::SqLength => inherit_sequence!(length),
            Self::SqConcat => inherit_sequence!(concat),
            Self::SqRepeat => inherit_sequence!(repeat),
            Self::SqItem => inherit_sequence!(item),
            Self::SqAssItem => inherit_sequence!(ass_item),
            Self::SqContains => inherit_sequence!(contains),
            Self::SqInplaceConcat => inherit_sequence!(inplace_concat),
            Self::SqInplaceRepeat => inherit_sequence!(inplace_repeat),

            // Mapping slots
            Self::MpLength => inherit_mapping!(length),
            Self::MpSubscript => inherit_mapping!(subscript),
            Self::MpAssSubscript => inherit_mapping!(ass_subscript),

            // Reserved slots - no-op
            _ => {}
        }
    }

    /// Copy slot from base type if self's slot is None
    pub fn copyslot_if_none(&self, typ: &crate::builtins::PyType, base: &crate::builtins::PyType) {
        macro_rules! copy_main {
            ($slot:ident) => {{
                if typ.slots.$slot.load().is_none() {
                    if let Some(base_val) = base.slots.$slot.load() {
                        typ.slots.$slot.store(Some(base_val));
                    }
                }
            }};
        }

        macro_rules! copy_number {
            ($slot:ident) => {{
                if typ.slots.as_number.$slot.load().is_none() {
                    if let Some(base_val) = base.slots.as_number.$slot.load() {
                        typ.slots.as_number.$slot.store(Some(base_val));
                    }
                }
            }};
        }

        macro_rules! copy_sequence {
            ($slot:ident) => {{
                if typ.slots.as_sequence.$slot.load().is_none() {
                    if let Some(base_val) = base.slots.as_sequence.$slot.load() {
                        typ.slots.as_sequence.$slot.store(Some(base_val));
                    }
                }
            }};
        }

        macro_rules! copy_mapping {
            ($slot:ident) => {{
                if typ.slots.as_mapping.$slot.load().is_none() {
                    if let Some(base_val) = base.slots.as_mapping.$slot.load() {
                        typ.slots.as_mapping.$slot.store(Some(base_val));
                    }
                }
            }};
        }

        match self {
            // Type slots
            Self::TpHash => copy_main!(hash),
            Self::TpRepr => copy_main!(repr),
            Self::TpStr => copy_main!(str),
            Self::TpCall => copy_main!(call),
            Self::TpIter => copy_main!(iter),
            Self::TpIternext => copy_main!(iternext),
            Self::TpInit => {
                // SLOTDEFINED check for multiple inheritance support
                if typ.slots.init.load().is_none()
                    && let Some(base_val) = base.slots.init.load()
                {
                    let slot_defined = base.base.as_ref().is_none_or(|bb| {
                        bb.slots.init.load().map(|v| v as usize) != Some(base_val as usize)
                    });
                    if slot_defined {
                        typ.slots.init.store(Some(base_val));
                    }
                }
            }
            Self::TpNew => {} // handled by set_new()
            Self::TpDel => copy_main!(del),
            Self::TpGetattro => copy_main!(getattro),
            Self::TpSetattro => copy_main!(setattro),
            Self::TpDescrGet => copy_main!(descr_get),
            Self::TpDescrSet => copy_main!(descr_set),
            Self::TpRichcompare => copy_main!(richcompare),

            // Number slots
            Self::NbAdd => copy_number!(add),
            Self::NbSubtract => copy_number!(subtract),
            Self::NbMultiply => copy_number!(multiply),
            Self::NbRemainder => copy_number!(remainder),
            Self::NbDivmod => copy_number!(divmod),
            Self::NbPower => copy_number!(power),
            Self::NbLshift => copy_number!(lshift),
            Self::NbRshift => copy_number!(rshift),
            Self::NbAnd => copy_number!(and),
            Self::NbXor => copy_number!(xor),
            Self::NbOr => copy_number!(or),
            Self::NbFloorDivide => copy_number!(floor_divide),
            Self::NbTrueDivide => copy_number!(true_divide),
            Self::NbMatrixMultiply => copy_number!(matrix_multiply),
            Self::NbInplaceAdd => copy_number!(inplace_add),
            Self::NbInplaceSubtract => copy_number!(inplace_subtract),
            Self::NbInplaceMultiply => copy_number!(inplace_multiply),
            Self::NbInplaceRemainder => copy_number!(inplace_remainder),
            Self::NbInplacePower => copy_number!(inplace_power),
            Self::NbInplaceLshift => copy_number!(inplace_lshift),
            Self::NbInplaceRshift => copy_number!(inplace_rshift),
            Self::NbInplaceAnd => copy_number!(inplace_and),
            Self::NbInplaceXor => copy_number!(inplace_xor),
            Self::NbInplaceOr => copy_number!(inplace_or),
            Self::NbInplaceFloorDivide => copy_number!(inplace_floor_divide),
            Self::NbInplaceTrueDivide => copy_number!(inplace_true_divide),
            Self::NbInplaceMatrixMultiply => copy_number!(inplace_matrix_multiply),
            // Number unary
            Self::NbNegative => copy_number!(negative),
            Self::NbPositive => copy_number!(positive),
            Self::NbAbsolute => copy_number!(absolute),
            Self::NbInvert => copy_number!(invert),
            Self::NbBool => copy_number!(boolean),
            Self::NbInt => copy_number!(int),
            Self::NbFloat => copy_number!(float),
            Self::NbIndex => copy_number!(index),

            // Sequence slots
            Self::SqLength => copy_sequence!(length),
            Self::SqConcat => copy_sequence!(concat),
            Self::SqRepeat => copy_sequence!(repeat),
            Self::SqItem => copy_sequence!(item),
            Self::SqAssItem => copy_sequence!(ass_item),
            Self::SqContains => copy_sequence!(contains),
            Self::SqInplaceConcat => copy_sequence!(inplace_concat),
            Self::SqInplaceRepeat => copy_sequence!(inplace_repeat),

            // Mapping slots
            Self::MpLength => copy_mapping!(length),
            Self::MpSubscript => copy_mapping!(subscript),
            Self::MpAssSubscript => copy_mapping!(ass_subscript),

            // Reserved slots - no-op
            _ => {}
        }
    }

    /// Get the SlotFunc from type slots for this accessor
    pub fn get_slot_func(&self, slots: &PyTypeSlots) -> Option<SlotFunc> {
        match self {
            // Type slots
            Self::TpHash => slots.hash.load().map(SlotFunc::Hash),
            Self::TpRepr => slots.repr.load().map(SlotFunc::Repr),
            Self::TpStr => slots.str.load().map(SlotFunc::Str),
            Self::TpCall => slots.call.load().map(SlotFunc::Call),
            Self::TpIter => slots.iter.load().map(SlotFunc::Iter),
            Self::TpIternext => slots.iternext.load().map(SlotFunc::IterNext),
            Self::TpInit => slots.init.load().map(SlotFunc::Init),
            Self::TpNew => None, // __new__ handled separately
            Self::TpDel => slots.del.load().map(SlotFunc::Del),
            Self::TpGetattro => slots.getattro.load().map(SlotFunc::GetAttro),
            Self::TpSetattro => slots.setattro.load().map(SlotFunc::SetAttro),
            Self::TpDescrGet => slots.descr_get.load().map(SlotFunc::DescrGet),
            Self::TpDescrSet => slots.descr_set.load().map(SlotFunc::DescrSet),
            Self::TpRichcompare => slots
                .richcompare
                .load()
                .map(|f| SlotFunc::RichCompare(f, PyComparisonOp::Eq)),

            // Number binary slots
            Self::NbAdd => slots.as_number.add.load().map(SlotFunc::NumBinary),
            Self::NbSubtract => slots.as_number.subtract.load().map(SlotFunc::NumBinary),
            Self::NbMultiply => slots.as_number.multiply.load().map(SlotFunc::NumBinary),
            Self::NbRemainder => slots.as_number.remainder.load().map(SlotFunc::NumBinary),
            Self::NbDivmod => slots.as_number.divmod.load().map(SlotFunc::NumBinary),
            Self::NbPower => slots.as_number.power.load().map(SlotFunc::NumTernary),
            Self::NbLshift => slots.as_number.lshift.load().map(SlotFunc::NumBinary),
            Self::NbRshift => slots.as_number.rshift.load().map(SlotFunc::NumBinary),
            Self::NbAnd => slots.as_number.and.load().map(SlotFunc::NumBinary),
            Self::NbXor => slots.as_number.xor.load().map(SlotFunc::NumBinary),
            Self::NbOr => slots.as_number.or.load().map(SlotFunc::NumBinary),
            Self::NbFloorDivide => slots.as_number.floor_divide.load().map(SlotFunc::NumBinary),
            Self::NbTrueDivide => slots.as_number.true_divide.load().map(SlotFunc::NumBinary),
            Self::NbMatrixMultiply => slots
                .as_number
                .matrix_multiply
                .load()
                .map(SlotFunc::NumBinary),

            // Number inplace slots
            Self::NbInplaceAdd => slots.as_number.inplace_add.load().map(SlotFunc::NumBinary),
            Self::NbInplaceSubtract => slots
                .as_number
                .inplace_subtract
                .load()
                .map(SlotFunc::NumBinary),
            Self::NbInplaceMultiply => slots
                .as_number
                .inplace_multiply
                .load()
                .map(SlotFunc::NumBinary),
            Self::NbInplaceRemainder => slots
                .as_number
                .inplace_remainder
                .load()
                .map(SlotFunc::NumBinary),
            Self::NbInplacePower => slots
                .as_number
                .inplace_power
                .load()
                .map(SlotFunc::NumTernary),
            Self::NbInplaceLshift => slots
                .as_number
                .inplace_lshift
                .load()
                .map(SlotFunc::NumBinary),
            Self::NbInplaceRshift => slots
                .as_number
                .inplace_rshift
                .load()
                .map(SlotFunc::NumBinary),
            Self::NbInplaceAnd => slots.as_number.inplace_and.load().map(SlotFunc::NumBinary),
            Self::NbInplaceXor => slots.as_number.inplace_xor.load().map(SlotFunc::NumBinary),
            Self::NbInplaceOr => slots.as_number.inplace_or.load().map(SlotFunc::NumBinary),
            Self::NbInplaceFloorDivide => slots
                .as_number
                .inplace_floor_divide
                .load()
                .map(SlotFunc::NumBinary),
            Self::NbInplaceTrueDivide => slots
                .as_number
                .inplace_true_divide
                .load()
                .map(SlotFunc::NumBinary),
            Self::NbInplaceMatrixMultiply => slots
                .as_number
                .inplace_matrix_multiply
                .load()
                .map(SlotFunc::NumBinary),

            // Number unary slots
            Self::NbNegative => slots.as_number.negative.load().map(SlotFunc::NumUnary),
            Self::NbPositive => slots.as_number.positive.load().map(SlotFunc::NumUnary),
            Self::NbAbsolute => slots.as_number.absolute.load().map(SlotFunc::NumUnary),
            Self::NbInvert => slots.as_number.invert.load().map(SlotFunc::NumUnary),
            Self::NbBool => slots.as_number.boolean.load().map(SlotFunc::NumBoolean),
            Self::NbInt => slots.as_number.int.load().map(SlotFunc::NumUnary),
            Self::NbFloat => slots.as_number.float.load().map(SlotFunc::NumUnary),
            Self::NbIndex => slots.as_number.index.load().map(SlotFunc::NumUnary),

            // Sequence slots
            Self::SqLength => slots.as_sequence.length.load().map(SlotFunc::SeqLength),
            Self::SqConcat => slots.as_sequence.concat.load().map(SlotFunc::SeqConcat),
            Self::SqRepeat => slots.as_sequence.repeat.load().map(SlotFunc::SeqRepeat),
            Self::SqItem => slots.as_sequence.item.load().map(SlotFunc::SeqItem),
            Self::SqAssItem => slots.as_sequence.ass_item.load().map(SlotFunc::SeqAssItem),
            Self::SqContains => slots.as_sequence.contains.load().map(SlotFunc::SeqContains),
            Self::SqInplaceConcat => slots
                .as_sequence
                .inplace_concat
                .load()
                .map(SlotFunc::SeqConcat),
            Self::SqInplaceRepeat => slots
                .as_sequence
                .inplace_repeat
                .load()
                .map(SlotFunc::SeqRepeat),

            // Mapping slots
            Self::MpLength => slots.as_mapping.length.load().map(SlotFunc::MapLength),
            Self::MpSubscript => slots
                .as_mapping
                .subscript
                .load()
                .map(SlotFunc::MapSubscript),
            Self::MpAssSubscript => slots
                .as_mapping
                .ass_subscript
                .load()
                .map(SlotFunc::MapAssSubscript),

            // Reserved slots
            _ => None,
        }
    }

    /// Get slot function considering SlotOp for right-hand and delete operations
    pub fn get_slot_func_with_op(
        &self,
        slots: &PyTypeSlots,
        op: Option<SlotOp>,
    ) -> Option<SlotFunc> {
        // For Delete operations, return the delete variant
        if op == Some(SlotOp::Delete) {
            match self {
                Self::TpSetattro => return slots.setattro.load().map(SlotFunc::DelAttro),
                Self::TpDescrSet => return slots.descr_set.load().map(SlotFunc::DescrDel),
                _ => {}
            }
        }
        // For Right operations on binary number slots, use right_* fields with swapped args
        if op == Some(SlotOp::Right) {
            match self {
                Self::NbAdd => {
                    return slots
                        .as_number
                        .right_add
                        .load()
                        .map(SlotFunc::NumBinaryRight);
                }
                Self::NbSubtract => {
                    return slots
                        .as_number
                        .right_subtract
                        .load()
                        .map(SlotFunc::NumBinaryRight);
                }
                Self::NbMultiply => {
                    return slots
                        .as_number
                        .right_multiply
                        .load()
                        .map(SlotFunc::NumBinaryRight);
                }
                Self::NbRemainder => {
                    return slots
                        .as_number
                        .right_remainder
                        .load()
                        .map(SlotFunc::NumBinaryRight);
                }
                Self::NbDivmod => {
                    return slots
                        .as_number
                        .right_divmod
                        .load()
                        .map(SlotFunc::NumBinaryRight);
                }
                Self::NbPower => {
                    return slots
                        .as_number
                        .right_power
                        .load()
                        .map(SlotFunc::NumTernaryRight);
                }
                Self::NbLshift => {
                    return slots
                        .as_number
                        .right_lshift
                        .load()
                        .map(SlotFunc::NumBinaryRight);
                }
                Self::NbRshift => {
                    return slots
                        .as_number
                        .right_rshift
                        .load()
                        .map(SlotFunc::NumBinaryRight);
                }
                Self::NbAnd => {
                    return slots
                        .as_number
                        .right_and
                        .load()
                        .map(SlotFunc::NumBinaryRight);
                }
                Self::NbXor => {
                    return slots
                        .as_number
                        .right_xor
                        .load()
                        .map(SlotFunc::NumBinaryRight);
                }
                Self::NbOr => {
                    return slots
                        .as_number
                        .right_or
                        .load()
                        .map(SlotFunc::NumBinaryRight);
                }
                Self::NbFloorDivide => {
                    return slots
                        .as_number
                        .right_floor_divide
                        .load()
                        .map(SlotFunc::NumBinaryRight);
                }
                Self::NbTrueDivide => {
                    return slots
                        .as_number
                        .right_true_divide
                        .load()
                        .map(SlotFunc::NumBinaryRight);
                }
                Self::NbMatrixMultiply => {
                    return slots
                        .as_number
                        .right_matrix_multiply
                        .load()
                        .map(SlotFunc::NumBinaryRight);
                }
                _ => {}
            }
        }
        // For comparison operations, use the appropriate PyComparisonOp
        if let Self::TpRichcompare = self
            && let Some(cmp_op) = op.and_then(|o| o.as_compare_op())
        {
            return slots
                .richcompare
                .load()
                .map(|f| SlotFunc::RichCompare(f, cmp_op));
        }
        // Fall back to existing get_slot_func for left/other operations
        self.get_slot_func(slots)
    }
}

/// Find all slot definitions with a given name
pub fn find_slot_defs_by_name(name: &str) -> impl Iterator<Item = &'static SlotDef> {
    SLOT_DEFS.iter().filter(move |def| def.name == name)
}

/// Total number of slot definitions
pub const SLOT_DEFS_COUNT: usize = SLOT_DEFS.len();

/// All slot definitions
pub static SLOT_DEFS: &[SlotDef] = &[
    // Type slots (tp_*)
    SlotDef {
        name: "__init__",
        accessor: SlotAccessor::TpInit,
        op: None,
        doc: "Initialize self. See help(type(self)) for accurate signature.",
    },
    SlotDef {
        name: "__new__",
        accessor: SlotAccessor::TpNew,
        op: None,
        doc: "Create and return a new object. See help(type) for accurate signature.",
    },
    SlotDef {
        name: "__del__",
        accessor: SlotAccessor::TpDel,
        op: None,
        doc: "Called when the instance is about to be destroyed.",
    },
    SlotDef {
        name: "__repr__",
        accessor: SlotAccessor::TpRepr,
        op: None,
        doc: "Return repr(self).",
    },
    SlotDef {
        name: "__str__",
        accessor: SlotAccessor::TpStr,
        op: None,
        doc: "Return str(self).",
    },
    SlotDef {
        name: "__hash__",
        accessor: SlotAccessor::TpHash,
        op: None,
        doc: "Return hash(self).",
    },
    SlotDef {
        name: "__call__",
        accessor: SlotAccessor::TpCall,
        op: None,
        doc: "Call self as a function.",
    },
    SlotDef {
        name: "__iter__",
        accessor: SlotAccessor::TpIter,
        op: None,
        doc: "Implement iter(self).",
    },
    SlotDef {
        name: "__next__",
        accessor: SlotAccessor::TpIternext,
        op: None,
        doc: "Implement next(self).",
    },
    // Attribute access
    SlotDef {
        name: "__getattribute__",
        accessor: SlotAccessor::TpGetattro,
        op: None,
        doc: "Return getattr(self, name).",
    },
    SlotDef {
        name: "__setattr__",
        accessor: SlotAccessor::TpSetattro,
        op: None,
        doc: "Implement setattr(self, name, value).",
    },
    SlotDef {
        name: "__delattr__",
        accessor: SlotAccessor::TpSetattro,
        op: Some(SlotOp::Delete),
        doc: "Implement delattr(self, name).",
    },
    // Rich comparison - all map to TpRichcompare with different op
    SlotDef {
        name: "__eq__",
        accessor: SlotAccessor::TpRichcompare,
        op: Some(SlotOp::Eq),
        doc: "Return self==value.",
    },
    SlotDef {
        name: "__ne__",
        accessor: SlotAccessor::TpRichcompare,
        op: Some(SlotOp::Ne),
        doc: "Return self!=value.",
    },
    SlotDef {
        name: "__lt__",
        accessor: SlotAccessor::TpRichcompare,
        op: Some(SlotOp::Lt),
        doc: "Return self<value.",
    },
    SlotDef {
        name: "__le__",
        accessor: SlotAccessor::TpRichcompare,
        op: Some(SlotOp::Le),
        doc: "Return self<=value.",
    },
    SlotDef {
        name: "__gt__",
        accessor: SlotAccessor::TpRichcompare,
        op: Some(SlotOp::Gt),
        doc: "Return self>value.",
    },
    SlotDef {
        name: "__ge__",
        accessor: SlotAccessor::TpRichcompare,
        op: Some(SlotOp::Ge),
        doc: "Return self>=value.",
    },
    // Descriptor protocol
    SlotDef {
        name: "__get__",
        accessor: SlotAccessor::TpDescrGet,
        op: None,
        doc: "Return an attribute of instance, which is of type owner.",
    },
    SlotDef {
        name: "__set__",
        accessor: SlotAccessor::TpDescrSet,
        op: None,
        doc: "Set an attribute of instance to value.",
    },
    SlotDef {
        name: "__delete__",
        accessor: SlotAccessor::TpDescrSet,
        op: Some(SlotOp::Delete),
        doc: "Delete an attribute of instance.",
    },
    // Sequence protocol (sq_*)
    SlotDef {
        name: "__len__",
        accessor: SlotAccessor::SqLength,
        op: None,
        doc: "Return len(self).",
    },
    SlotDef {
        name: "__getitem__",
        accessor: SlotAccessor::SqItem,
        op: None,
        doc: "Return self[key].",
    },
    SlotDef {
        name: "__setitem__",
        accessor: SlotAccessor::SqAssItem,
        op: None,
        doc: "Set self[key] to value.",
    },
    SlotDef {
        name: "__delitem__",
        accessor: SlotAccessor::SqAssItem,
        op: None,
        doc: "Delete self[key].",
    },
    SlotDef {
        name: "__contains__",
        accessor: SlotAccessor::SqContains,
        op: None,
        doc: "Return key in self.",
    },
    // Mapping protocol (mp_*)
    SlotDef {
        name: "__len__",
        accessor: SlotAccessor::MpLength,
        op: None,
        doc: "Return len(self).",
    },
    SlotDef {
        name: "__getitem__",
        accessor: SlotAccessor::MpSubscript,
        op: None,
        doc: "Return self[key].",
    },
    SlotDef {
        name: "__setitem__",
        accessor: SlotAccessor::MpAssSubscript,
        op: None,
        doc: "Set self[key] to value.",
    },
    SlotDef {
        name: "__delitem__",
        accessor: SlotAccessor::MpAssSubscript,
        op: None,
        doc: "Delete self[key].",
    },
    // Number protocol - binary ops with left/right variants
    SlotDef {
        name: "__add__",
        accessor: SlotAccessor::NbAdd,
        op: Some(SlotOp::Left),
        doc: "Return self+value.",
    },
    SlotDef {
        name: "__radd__",
        accessor: SlotAccessor::NbAdd,
        op: Some(SlotOp::Right),
        doc: "Return value+self.",
    },
    SlotDef {
        name: "__iadd__",
        accessor: SlotAccessor::NbInplaceAdd,
        op: None,
        doc: "Implement self+=value.",
    },
    SlotDef {
        name: "__sub__",
        accessor: SlotAccessor::NbSubtract,
        op: Some(SlotOp::Left),
        doc: "Return self-value.",
    },
    SlotDef {
        name: "__rsub__",
        accessor: SlotAccessor::NbSubtract,
        op: Some(SlotOp::Right),
        doc: "Return value-self.",
    },
    SlotDef {
        name: "__isub__",
        accessor: SlotAccessor::NbInplaceSubtract,
        op: None,
        doc: "Implement self-=value.",
    },
    SlotDef {
        name: "__mul__",
        accessor: SlotAccessor::NbMultiply,
        op: Some(SlotOp::Left),
        doc: "Return self*value.",
    },
    SlotDef {
        name: "__rmul__",
        accessor: SlotAccessor::NbMultiply,
        op: Some(SlotOp::Right),
        doc: "Return value*self.",
    },
    SlotDef {
        name: "__imul__",
        accessor: SlotAccessor::NbInplaceMultiply,
        op: None,
        doc: "Implement self*=value.",
    },
    SlotDef {
        name: "__mod__",
        accessor: SlotAccessor::NbRemainder,
        op: Some(SlotOp::Left),
        doc: "Return self%value.",
    },
    SlotDef {
        name: "__rmod__",
        accessor: SlotAccessor::NbRemainder,
        op: Some(SlotOp::Right),
        doc: "Return value%self.",
    },
    SlotDef {
        name: "__imod__",
        accessor: SlotAccessor::NbInplaceRemainder,
        op: None,
        doc: "Implement self%=value.",
    },
    SlotDef {
        name: "__divmod__",
        accessor: SlotAccessor::NbDivmod,
        op: Some(SlotOp::Left),
        doc: "Return divmod(self, value).",
    },
    SlotDef {
        name: "__rdivmod__",
        accessor: SlotAccessor::NbDivmod,
        op: Some(SlotOp::Right),
        doc: "Return divmod(value, self).",
    },
    SlotDef {
        name: "__pow__",
        accessor: SlotAccessor::NbPower,
        op: Some(SlotOp::Left),
        doc: "Return pow(self, value, mod).",
    },
    SlotDef {
        name: "__rpow__",
        accessor: SlotAccessor::NbPower,
        op: Some(SlotOp::Right),
        doc: "Return pow(value, self, mod).",
    },
    SlotDef {
        name: "__ipow__",
        accessor: SlotAccessor::NbInplacePower,
        op: None,
        doc: "Implement self**=value.",
    },
    SlotDef {
        name: "__lshift__",
        accessor: SlotAccessor::NbLshift,
        op: Some(SlotOp::Left),
        doc: "Return self<<value.",
    },
    SlotDef {
        name: "__rlshift__",
        accessor: SlotAccessor::NbLshift,
        op: Some(SlotOp::Right),
        doc: "Return value<<self.",
    },
    SlotDef {
        name: "__ilshift__",
        accessor: SlotAccessor::NbInplaceLshift,
        op: None,
        doc: "Implement self<<=value.",
    },
    SlotDef {
        name: "__rshift__",
        accessor: SlotAccessor::NbRshift,
        op: Some(SlotOp::Left),
        doc: "Return self>>value.",
    },
    SlotDef {
        name: "__rrshift__",
        accessor: SlotAccessor::NbRshift,
        op: Some(SlotOp::Right),
        doc: "Return value>>self.",
    },
    SlotDef {
        name: "__irshift__",
        accessor: SlotAccessor::NbInplaceRshift,
        op: None,
        doc: "Implement self>>=value.",
    },
    SlotDef {
        name: "__and__",
        accessor: SlotAccessor::NbAnd,
        op: Some(SlotOp::Left),
        doc: "Return self&value.",
    },
    SlotDef {
        name: "__rand__",
        accessor: SlotAccessor::NbAnd,
        op: Some(SlotOp::Right),
        doc: "Return value&self.",
    },
    SlotDef {
        name: "__iand__",
        accessor: SlotAccessor::NbInplaceAnd,
        op: None,
        doc: "Implement self&=value.",
    },
    SlotDef {
        name: "__xor__",
        accessor: SlotAccessor::NbXor,
        op: Some(SlotOp::Left),
        doc: "Return self^value.",
    },
    SlotDef {
        name: "__rxor__",
        accessor: SlotAccessor::NbXor,
        op: Some(SlotOp::Right),
        doc: "Return value^self.",
    },
    SlotDef {
        name: "__ixor__",
        accessor: SlotAccessor::NbInplaceXor,
        op: None,
        doc: "Implement self^=value.",
    },
    SlotDef {
        name: "__or__",
        accessor: SlotAccessor::NbOr,
        op: Some(SlotOp::Left),
        doc: "Return self|value.",
    },
    SlotDef {
        name: "__ror__",
        accessor: SlotAccessor::NbOr,
        op: Some(SlotOp::Right),
        doc: "Return value|self.",
    },
    SlotDef {
        name: "__ior__",
        accessor: SlotAccessor::NbInplaceOr,
        op: None,
        doc: "Implement self|=value.",
    },
    SlotDef {
        name: "__floordiv__",
        accessor: SlotAccessor::NbFloorDivide,
        op: Some(SlotOp::Left),
        doc: "Return self//value.",
    },
    SlotDef {
        name: "__rfloordiv__",
        accessor: SlotAccessor::NbFloorDivide,
        op: Some(SlotOp::Right),
        doc: "Return value//self.",
    },
    SlotDef {
        name: "__ifloordiv__",
        accessor: SlotAccessor::NbInplaceFloorDivide,
        op: None,
        doc: "Implement self//=value.",
    },
    SlotDef {
        name: "__truediv__",
        accessor: SlotAccessor::NbTrueDivide,
        op: Some(SlotOp::Left),
        doc: "Return self/value.",
    },
    SlotDef {
        name: "__rtruediv__",
        accessor: SlotAccessor::NbTrueDivide,
        op: Some(SlotOp::Right),
        doc: "Return value/self.",
    },
    SlotDef {
        name: "__itruediv__",
        accessor: SlotAccessor::NbInplaceTrueDivide,
        op: None,
        doc: "Implement self/=value.",
    },
    SlotDef {
        name: "__matmul__",
        accessor: SlotAccessor::NbMatrixMultiply,
        op: Some(SlotOp::Left),
        doc: "Return self@value.",
    },
    SlotDef {
        name: "__rmatmul__",
        accessor: SlotAccessor::NbMatrixMultiply,
        op: Some(SlotOp::Right),
        doc: "Return value@self.",
    },
    SlotDef {
        name: "__imatmul__",
        accessor: SlotAccessor::NbInplaceMatrixMultiply,
        op: None,
        doc: "Implement self@=value.",
    },
    // Number unary operations
    SlotDef {
        name: "__neg__",
        accessor: SlotAccessor::NbNegative,
        op: None,
        doc: "Return -self.",
    },
    SlotDef {
        name: "__pos__",
        accessor: SlotAccessor::NbPositive,
        op: None,
        doc: "Return +self.",
    },
    SlotDef {
        name: "__abs__",
        accessor: SlotAccessor::NbAbsolute,
        op: None,
        doc: "Return abs(self).",
    },
    SlotDef {
        name: "__invert__",
        accessor: SlotAccessor::NbInvert,
        op: None,
        doc: "Return ~self.",
    },
    SlotDef {
        name: "__bool__",
        accessor: SlotAccessor::NbBool,
        op: None,
        doc: "Return self != 0.",
    },
    SlotDef {
        name: "__int__",
        accessor: SlotAccessor::NbInt,
        op: None,
        doc: "Return int(self).",
    },
    SlotDef {
        name: "__float__",
        accessor: SlotAccessor::NbFloat,
        op: None,
        doc: "Return float(self).",
    },
    SlotDef {
        name: "__index__",
        accessor: SlotAccessor::NbIndex,
        op: None,
        doc: "Return self converted to an integer, if self is suitable for use as an index into a list.",
    },
    // Sequence inplace operations (also map to number slots for some types)
    SlotDef {
        name: "__add__",
        accessor: SlotAccessor::SqConcat,
        op: None,
        doc: "Return self+value.",
    },
    SlotDef {
        name: "__mul__",
        accessor: SlotAccessor::SqRepeat,
        op: None,
        doc: "Return self*value.",
    },
    SlotDef {
        name: "__iadd__",
        accessor: SlotAccessor::SqInplaceConcat,
        op: None,
        doc: "Implement self+=value.",
    },
    SlotDef {
        name: "__imul__",
        accessor: SlotAccessor::SqInplaceRepeat,
        op: None,
        doc: "Implement self*=value.",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_by_name() {
        // __len__ appears in both sequence and mapping
        let len_defs: Vec<_> = find_slot_defs_by_name("__len__").collect();
        assert_eq!(len_defs.len(), 2);

        // __init__ appears once
        let init_defs: Vec<_> = find_slot_defs_by_name("__init__").collect();
        assert_eq!(init_defs.len(), 1);

        // __add__ appears in number (left/right) and sequence
        let add_defs: Vec<_> = find_slot_defs_by_name("__add__").collect();
        assert_eq!(add_defs.len(), 2); // NbAdd(Left) and SqConcat
    }

    #[test]
    fn test_slot_op() {
        // Test comparison ops
        assert_eq!(SlotOp::Lt.as_compare_op(), Some(PyComparisonOp::Lt));
        assert_eq!(SlotOp::Eq.as_compare_op(), Some(PyComparisonOp::Eq));
        assert_eq!(SlotOp::Left.as_compare_op(), None);

        // Test right check
        assert!(SlotOp::Right.is_right());
        assert!(!SlotOp::Left.is_right());
    }
}
