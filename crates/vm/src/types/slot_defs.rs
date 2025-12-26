//! Slot definitions array
//!
//! This module provides a centralized array of all slot definitions,
//! enabling automatic wrapper generation and slot updates.

use super::{PyComparisonOp, PyTypeSlots};
use crate::builtins::descriptor::SlotFunc;
use crate::protocol::{PyNumberBinaryFunc, PyNumberSlots};

/// Slot definition entry
#[derive(Clone, Copy)]
pub struct SlotDef {
    /// Method name ("__init__", "__add__", etc.)
    pub name: &'static str,

    /// Slot accessor (which slot field to access)
    pub accessor: SlotAccessor,

    /// Documentation string
    pub doc: &'static str,
}

/// Slot accessor
///
/// Flat enum with all slot types inlined for `#[repr(u8)]` support.
/// Each variant directly corresponds to a slot field in PyTypeSlots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotAccessor {
    // Main slots
    Hash,
    Repr,
    Str,
    Call,
    Iter,
    IterNext,
    Init,
    New,
    Del,
    GetAttro,
    SetAttro,
    DelAttro,
    DescrGet,
    DescrSet,
    DescrDel,

    // Rich compare - tp_richcompare with different ops
    RichCompareLt,
    RichCompareLe,
    RichCompareEq,
    RichCompareNe,
    RichCompareGt,
    RichCompareGe,

    // Number binary operations
    NumAdd,
    NumRightAdd,
    NumInplaceAdd,
    NumSubtract,
    NumRightSubtract,
    NumInplaceSubtract,
    NumMultiply,
    NumRightMultiply,
    NumInplaceMultiply,
    NumRemainder,
    NumRightRemainder,
    NumInplaceRemainder,
    NumDivmod,
    NumRightDivmod,
    NumPower,
    NumRightPower,
    NumInplacePower,
    NumFloorDivide,
    NumRightFloorDivide,
    NumInplaceFloorDivide,
    NumTrueDivide,
    NumRightTrueDivide,
    NumInplaceTrueDivide,
    NumMatrixMultiply,
    NumRightMatrixMultiply,
    NumInplaceMatrixMultiply,

    // Bitwise operations
    NumLshift,
    NumRightLshift,
    NumInplaceLshift,
    NumRshift,
    NumRightRshift,
    NumInplaceRshift,
    NumAnd,
    NumRightAnd,
    NumInplaceAnd,
    NumXor,
    NumRightXor,
    NumInplaceXor,
    NumOr,
    NumRightOr,
    NumInplaceOr,

    // Number unary operations
    NumNegative,
    NumPositive,
    NumAbsolute,
    NumInvert,
    NumBoolean,
    NumInt,
    NumFloat,
    NumIndex,

    // Sequence slots
    SeqLength,
    SeqConcat,
    SeqRepeat,
    SeqItem,
    SeqAssItem,
    SeqContains,
    SeqInplaceConcat,
    SeqInplaceRepeat,

    // Mapping slots
    MapLength,
    MapSubscript,
    MapAssSubscript,
}

// SlotAccessor implementation

impl SlotAccessor {
    /// Get the PyComparisonOp for RichCompare variants
    pub fn rich_compare_op(&self) -> Option<PyComparisonOp> {
        match self {
            Self::RichCompareLt => Some(PyComparisonOp::Lt),
            Self::RichCompareLe => Some(PyComparisonOp::Le),
            Self::RichCompareEq => Some(PyComparisonOp::Eq),
            Self::RichCompareNe => Some(PyComparisonOp::Ne),
            Self::RichCompareGt => Some(PyComparisonOp::Gt),
            Self::RichCompareGe => Some(PyComparisonOp::Ge),
            _ => None,
        }
    }

    /// Check if this accessor refers to a shared slot
    ///
    /// Shared slots are used by multiple dunder methods:
    /// - SetAttro/DelAttro share setattro slot
    /// - RichCompare variants share richcompare slot
    /// - DescrSet/DescrDel share descr_set slot
    /// - SeqAssItem is used by __setitem__ and __delitem__
    /// - MapAssSubscript is used by __setitem__ and __delitem__
    pub fn is_shared_slot(&self) -> bool {
        matches!(
            self,
            Self::SetAttro
                | Self::DelAttro
                | Self::RichCompareLt
                | Self::RichCompareLe
                | Self::RichCompareEq
                | Self::RichCompareNe
                | Self::RichCompareGt
                | Self::RichCompareGe
                | Self::DescrSet
                | Self::DescrDel
                | Self::SeqAssItem
                | Self::MapAssSubscript
        )
    }

    /// Get the underlying slot accessor for shared slots
    ///
    /// For SetAttro/DelAttro, returns SetAttro (they share the same slot).
    /// For DescrSet/DescrDel, returns DescrSet (they share the same slot).
    pub fn canonical(&self) -> SlotAccessor {
        match self {
            Self::DelAttro => Self::SetAttro,
            Self::DescrDel => Self::DescrSet,
            _ => *self,
        }
    }

    /// Extract the raw function pointer from a SlotFunc if it matches this accessor's type
    pub fn extract_from_slot_func(&self, slot_func: &SlotFunc) -> bool {
        match self {
            // Main slots
            Self::Hash => matches!(slot_func, SlotFunc::Hash(_)),
            Self::Repr => matches!(slot_func, SlotFunc::Repr(_)),
            Self::Str => matches!(slot_func, SlotFunc::Str(_)),
            Self::Call => matches!(slot_func, SlotFunc::Call(_)),
            Self::Iter => matches!(slot_func, SlotFunc::Iter(_)),
            Self::IterNext => matches!(slot_func, SlotFunc::IterNext(_)),
            Self::Init => matches!(slot_func, SlotFunc::Init(_)),
            Self::Del => matches!(slot_func, SlotFunc::Del(_)),
            Self::GetAttro => matches!(slot_func, SlotFunc::GetAttro(_)),
            Self::SetAttro | Self::DelAttro => {
                matches!(slot_func, SlotFunc::SetAttro(_) | SlotFunc::DelAttro(_))
            }
            Self::DescrGet => matches!(slot_func, SlotFunc::DescrGet(_)),
            Self::DescrSet | Self::DescrDel => {
                matches!(slot_func, SlotFunc::DescrSet(_) | SlotFunc::DescrDel(_))
            }
            // RichCompare
            Self::RichCompareLt
            | Self::RichCompareLe
            | Self::RichCompareEq
            | Self::RichCompareNe
            | Self::RichCompareGt
            | Self::RichCompareGe => matches!(slot_func, SlotFunc::RichCompare(_, _)),
            // Number - Power (ternary)
            Self::NumPower | Self::NumRightPower | Self::NumInplacePower => {
                matches!(slot_func, SlotFunc::NumTernary(_))
            }
            // Number - Boolean
            Self::NumBoolean => matches!(slot_func, SlotFunc::NumBoolean(_)),
            // Number - Unary
            Self::NumNegative
            | Self::NumPositive
            | Self::NumAbsolute
            | Self::NumInvert
            | Self::NumInt
            | Self::NumFloat
            | Self::NumIndex => matches!(slot_func, SlotFunc::NumUnary(_)),
            // Number - Binary (all others)
            Self::NumAdd
            | Self::NumRightAdd
            | Self::NumInplaceAdd
            | Self::NumSubtract
            | Self::NumRightSubtract
            | Self::NumInplaceSubtract
            | Self::NumMultiply
            | Self::NumRightMultiply
            | Self::NumInplaceMultiply
            | Self::NumRemainder
            | Self::NumRightRemainder
            | Self::NumInplaceRemainder
            | Self::NumDivmod
            | Self::NumRightDivmod
            | Self::NumFloorDivide
            | Self::NumRightFloorDivide
            | Self::NumInplaceFloorDivide
            | Self::NumTrueDivide
            | Self::NumRightTrueDivide
            | Self::NumInplaceTrueDivide
            | Self::NumMatrixMultiply
            | Self::NumRightMatrixMultiply
            | Self::NumInplaceMatrixMultiply
            | Self::NumLshift
            | Self::NumRightLshift
            | Self::NumInplaceLshift
            | Self::NumRshift
            | Self::NumRightRshift
            | Self::NumInplaceRshift
            | Self::NumAnd
            | Self::NumRightAnd
            | Self::NumInplaceAnd
            | Self::NumXor
            | Self::NumRightXor
            | Self::NumInplaceXor
            | Self::NumOr
            | Self::NumRightOr
            | Self::NumInplaceOr => matches!(slot_func, SlotFunc::NumBinary(_)),
            // Sequence
            Self::SeqLength => matches!(slot_func, SlotFunc::SeqLength(_)),
            Self::SeqConcat | Self::SeqInplaceConcat => matches!(slot_func, SlotFunc::SeqConcat(_)),
            Self::SeqRepeat | Self::SeqInplaceRepeat => matches!(slot_func, SlotFunc::SeqRepeat(_)),
            Self::SeqItem => matches!(slot_func, SlotFunc::SeqItem(_)),
            Self::SeqAssItem => matches!(slot_func, SlotFunc::SeqAssItem(_)),
            Self::SeqContains => matches!(slot_func, SlotFunc::SeqContains(_)),
            // Mapping
            Self::MapLength => matches!(slot_func, SlotFunc::MapLength(_)),
            Self::MapSubscript => matches!(slot_func, SlotFunc::MapSubscript(_)),
            Self::MapAssSubscript => matches!(slot_func, SlotFunc::MapAssSubscript(_)),
            // New has no wrapper
            Self::New => false,
        }
    }

    /// Inherit slot value from MRO
    pub fn inherit_from_mro(&self, typ: &crate::builtins::PyType) {
        // Note: typ.mro does NOT include typ itself, so we iterate all elements
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
            // Main slots
            Self::Hash => inherit_main!(hash),
            Self::Repr => inherit_main!(repr),
            Self::Str => inherit_main!(str),
            Self::Call => inherit_main!(call),
            Self::Iter => inherit_main!(iter),
            Self::IterNext => inherit_main!(iternext),
            Self::Init => inherit_main!(init),
            Self::New => inherit_main!(new),
            Self::Del => inherit_main!(del),
            Self::GetAttro => inherit_main!(getattro),
            Self::SetAttro | Self::DelAttro => inherit_main!(setattro),
            Self::DescrGet => inherit_main!(descr_get),
            Self::DescrSet | Self::DescrDel => inherit_main!(descr_set),

            // RichCompare
            Self::RichCompareLt
            | Self::RichCompareLe
            | Self::RichCompareEq
            | Self::RichCompareNe
            | Self::RichCompareGt
            | Self::RichCompareGe => inherit_main!(richcompare),

            // Number binary
            Self::NumAdd => inherit_number!(add),
            Self::NumRightAdd => inherit_number!(right_add),
            Self::NumInplaceAdd => inherit_number!(inplace_add),
            Self::NumSubtract => inherit_number!(subtract),
            Self::NumRightSubtract => inherit_number!(right_subtract),
            Self::NumInplaceSubtract => inherit_number!(inplace_subtract),
            Self::NumMultiply => inherit_number!(multiply),
            Self::NumRightMultiply => inherit_number!(right_multiply),
            Self::NumInplaceMultiply => inherit_number!(inplace_multiply),
            Self::NumRemainder => inherit_number!(remainder),
            Self::NumRightRemainder => inherit_number!(right_remainder),
            Self::NumInplaceRemainder => inherit_number!(inplace_remainder),
            Self::NumDivmod => inherit_number!(divmod),
            Self::NumRightDivmod => inherit_number!(right_divmod),
            Self::NumPower => inherit_number!(power),
            Self::NumRightPower => inherit_number!(right_power),
            Self::NumInplacePower => inherit_number!(inplace_power),
            Self::NumFloorDivide => inherit_number!(floor_divide),
            Self::NumRightFloorDivide => inherit_number!(right_floor_divide),
            Self::NumInplaceFloorDivide => inherit_number!(inplace_floor_divide),
            Self::NumTrueDivide => inherit_number!(true_divide),
            Self::NumRightTrueDivide => inherit_number!(right_true_divide),
            Self::NumInplaceTrueDivide => inherit_number!(inplace_true_divide),
            Self::NumMatrixMultiply => inherit_number!(matrix_multiply),
            Self::NumRightMatrixMultiply => inherit_number!(right_matrix_multiply),
            Self::NumInplaceMatrixMultiply => inherit_number!(inplace_matrix_multiply),
            Self::NumLshift => inherit_number!(lshift),
            Self::NumRightLshift => inherit_number!(right_lshift),
            Self::NumInplaceLshift => inherit_number!(inplace_lshift),
            Self::NumRshift => inherit_number!(rshift),
            Self::NumRightRshift => inherit_number!(right_rshift),
            Self::NumInplaceRshift => inherit_number!(inplace_rshift),
            Self::NumAnd => inherit_number!(and),
            Self::NumRightAnd => inherit_number!(right_and),
            Self::NumInplaceAnd => inherit_number!(inplace_and),
            Self::NumXor => inherit_number!(xor),
            Self::NumRightXor => inherit_number!(right_xor),
            Self::NumInplaceXor => inherit_number!(inplace_xor),
            Self::NumOr => inherit_number!(or),
            Self::NumRightOr => inherit_number!(right_or),
            Self::NumInplaceOr => inherit_number!(inplace_or),

            // Number unary
            Self::NumNegative => inherit_number!(negative),
            Self::NumPositive => inherit_number!(positive),
            Self::NumAbsolute => inherit_number!(absolute),
            Self::NumInvert => inherit_number!(invert),
            Self::NumBoolean => inherit_number!(boolean),
            Self::NumInt => inherit_number!(int),
            Self::NumFloat => inherit_number!(float),
            Self::NumIndex => inherit_number!(index),

            // Sequence
            Self::SeqLength => inherit_sequence!(length),
            Self::SeqConcat => inherit_sequence!(concat),
            Self::SeqRepeat => inherit_sequence!(repeat),
            Self::SeqItem => inherit_sequence!(item),
            Self::SeqAssItem => inherit_sequence!(ass_item),
            Self::SeqContains => inherit_sequence!(contains),
            Self::SeqInplaceConcat => inherit_sequence!(inplace_concat),
            Self::SeqInplaceRepeat => inherit_sequence!(inplace_repeat),

            // Mapping
            Self::MapLength => inherit_mapping!(length),
            Self::MapSubscript => inherit_mapping!(subscript),
            Self::MapAssSubscript => inherit_mapping!(ass_subscript),
        }
    }

    /// Copy slot from base type if self's slot is None
    ///
    /// Used by inherit_slots() for slot inheritance from base classes.
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
            // Main slots
            Self::Hash => copy_main!(hash),
            Self::Repr => copy_main!(repr),
            Self::Str => copy_main!(str),
            Self::Call => copy_main!(call),
            Self::Iter => copy_main!(iter),
            Self::IterNext => copy_main!(iternext),
            Self::Init => {
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
            Self::New => {} // handled by set_new()
            Self::Del => copy_main!(del),
            Self::GetAttro => copy_main!(getattro),
            Self::SetAttro | Self::DelAttro => copy_main!(setattro),
            Self::DescrGet => copy_main!(descr_get),
            Self::DescrSet | Self::DescrDel => copy_main!(descr_set),

            // RichCompare
            Self::RichCompareLt
            | Self::RichCompareLe
            | Self::RichCompareEq
            | Self::RichCompareNe
            | Self::RichCompareGt
            | Self::RichCompareGe => copy_main!(richcompare),

            // Number binary
            Self::NumAdd => copy_number!(add),
            Self::NumRightAdd => copy_number!(right_add),
            Self::NumInplaceAdd => copy_number!(inplace_add),
            Self::NumSubtract => copy_number!(subtract),
            Self::NumRightSubtract => copy_number!(right_subtract),
            Self::NumInplaceSubtract => copy_number!(inplace_subtract),
            Self::NumMultiply => copy_number!(multiply),
            Self::NumRightMultiply => copy_number!(right_multiply),
            Self::NumInplaceMultiply => copy_number!(inplace_multiply),
            Self::NumRemainder => copy_number!(remainder),
            Self::NumRightRemainder => copy_number!(right_remainder),
            Self::NumInplaceRemainder => copy_number!(inplace_remainder),
            Self::NumDivmod => copy_number!(divmod),
            Self::NumRightDivmod => copy_number!(right_divmod),
            Self::NumPower => copy_number!(power),
            Self::NumRightPower => copy_number!(right_power),
            Self::NumInplacePower => copy_number!(inplace_power),
            Self::NumFloorDivide => copy_number!(floor_divide),
            Self::NumRightFloorDivide => copy_number!(right_floor_divide),
            Self::NumInplaceFloorDivide => copy_number!(inplace_floor_divide),
            Self::NumTrueDivide => copy_number!(true_divide),
            Self::NumRightTrueDivide => copy_number!(right_true_divide),
            Self::NumInplaceTrueDivide => copy_number!(inplace_true_divide),
            Self::NumMatrixMultiply => copy_number!(matrix_multiply),
            Self::NumRightMatrixMultiply => copy_number!(right_matrix_multiply),
            Self::NumInplaceMatrixMultiply => copy_number!(inplace_matrix_multiply),
            Self::NumLshift => copy_number!(lshift),
            Self::NumRightLshift => copy_number!(right_lshift),
            Self::NumInplaceLshift => copy_number!(inplace_lshift),
            Self::NumRshift => copy_number!(rshift),
            Self::NumRightRshift => copy_number!(right_rshift),
            Self::NumInplaceRshift => copy_number!(inplace_rshift),
            Self::NumAnd => copy_number!(and),
            Self::NumRightAnd => copy_number!(right_and),
            Self::NumInplaceAnd => copy_number!(inplace_and),
            Self::NumXor => copy_number!(xor),
            Self::NumRightXor => copy_number!(right_xor),
            Self::NumInplaceXor => copy_number!(inplace_xor),
            Self::NumOr => copy_number!(or),
            Self::NumRightOr => copy_number!(right_or),
            Self::NumInplaceOr => copy_number!(inplace_or),

            // Number unary
            Self::NumNegative => copy_number!(negative),
            Self::NumPositive => copy_number!(positive),
            Self::NumAbsolute => copy_number!(absolute),
            Self::NumInvert => copy_number!(invert),
            Self::NumBoolean => copy_number!(boolean),
            Self::NumInt => copy_number!(int),
            Self::NumFloat => copy_number!(float),
            Self::NumIndex => copy_number!(index),

            // Sequence
            Self::SeqLength => copy_sequence!(length),
            Self::SeqConcat => copy_sequence!(concat),
            Self::SeqRepeat => copy_sequence!(repeat),
            Self::SeqItem => copy_sequence!(item),
            Self::SeqAssItem => copy_sequence!(ass_item),
            Self::SeqContains => copy_sequence!(contains),
            Self::SeqInplaceConcat => copy_sequence!(inplace_concat),
            Self::SeqInplaceRepeat => copy_sequence!(inplace_repeat),

            // Mapping
            Self::MapLength => copy_mapping!(length),
            Self::MapSubscript => copy_mapping!(subscript),
            Self::MapAssSubscript => copy_mapping!(ass_subscript),
        }
    }

    /// Get the slot function from PyTypeSlots, wrapped in SlotFunc
    ///
    /// Returns None if the slot is not set.
    pub fn get_slot_func(&self, slots: &PyTypeSlots) -> Option<SlotFunc> {
        match self {
            // Main slots
            Self::Hash => slots.hash.load().map(SlotFunc::Hash),
            Self::Repr => slots.repr.load().map(SlotFunc::Repr),
            Self::Str => slots.str.load().map(SlotFunc::Str),
            Self::Call => slots.call.load().map(SlotFunc::Call),
            Self::Iter => slots.iter.load().map(SlotFunc::Iter),
            Self::IterNext => slots.iternext.load().map(SlotFunc::IterNext),
            Self::Init => slots.init.load().map(SlotFunc::Init),
            Self::New => None, // __new__ has special handling, no wrapper
            Self::Del => slots.del.load().map(SlotFunc::Del),
            Self::GetAttro => slots.getattro.load().map(SlotFunc::GetAttro),
            Self::SetAttro => slots.setattro.load().map(SlotFunc::SetAttro),
            Self::DelAttro => slots.setattro.load().map(SlotFunc::DelAttro),
            Self::DescrGet => slots.descr_get.load().map(SlotFunc::DescrGet),
            Self::DescrSet => slots.descr_set.load().map(SlotFunc::DescrSet),
            Self::DescrDel => slots.descr_set.load().map(SlotFunc::DescrDel),

            // RichCompare
            Self::RichCompareLt => slots
                .richcompare
                .load()
                .map(|f| SlotFunc::RichCompare(f, PyComparisonOp::Lt)),
            Self::RichCompareLe => slots
                .richcompare
                .load()
                .map(|f| SlotFunc::RichCompare(f, PyComparisonOp::Le)),
            Self::RichCompareEq => slots
                .richcompare
                .load()
                .map(|f| SlotFunc::RichCompare(f, PyComparisonOp::Eq)),
            Self::RichCompareNe => slots
                .richcompare
                .load()
                .map(|f| SlotFunc::RichCompare(f, PyComparisonOp::Ne)),
            Self::RichCompareGt => slots
                .richcompare
                .load()
                .map(|f| SlotFunc::RichCompare(f, PyComparisonOp::Gt)),
            Self::RichCompareGe => slots
                .richcompare
                .load()
                .map(|f| SlotFunc::RichCompare(f, PyComparisonOp::Ge)),

            // Number - binary
            Self::NumAdd => Self::get_number_slot(&slots.as_number, |s| s.add.load()),
            Self::NumRightAdd => Self::get_number_slot(&slots.as_number, |s| s.right_add.load()),
            Self::NumInplaceAdd => {
                Self::get_number_slot(&slots.as_number, |s| s.inplace_add.load())
            }
            Self::NumSubtract => Self::get_number_slot(&slots.as_number, |s| s.subtract.load()),
            Self::NumRightSubtract => {
                Self::get_number_slot(&slots.as_number, |s| s.right_subtract.load())
            }
            Self::NumInplaceSubtract => {
                Self::get_number_slot(&slots.as_number, |s| s.inplace_subtract.load())
            }
            Self::NumMultiply => Self::get_number_slot(&slots.as_number, |s| s.multiply.load()),
            Self::NumRightMultiply => {
                Self::get_number_slot(&slots.as_number, |s| s.right_multiply.load())
            }
            Self::NumInplaceMultiply => {
                Self::get_number_slot(&slots.as_number, |s| s.inplace_multiply.load())
            }
            Self::NumRemainder => Self::get_number_slot(&slots.as_number, |s| s.remainder.load()),
            Self::NumRightRemainder => {
                Self::get_number_slot(&slots.as_number, |s| s.right_remainder.load())
            }
            Self::NumInplaceRemainder => {
                Self::get_number_slot(&slots.as_number, |s| s.inplace_remainder.load())
            }
            Self::NumDivmod => Self::get_number_slot(&slots.as_number, |s| s.divmod.load()),
            Self::NumRightDivmod => {
                Self::get_number_slot(&slots.as_number, |s| s.right_divmod.load())
            }
            Self::NumFloorDivide => {
                Self::get_number_slot(&slots.as_number, |s| s.floor_divide.load())
            }
            Self::NumRightFloorDivide => {
                Self::get_number_slot(&slots.as_number, |s| s.right_floor_divide.load())
            }
            Self::NumInplaceFloorDivide => {
                Self::get_number_slot(&slots.as_number, |s| s.inplace_floor_divide.load())
            }
            Self::NumTrueDivide => {
                Self::get_number_slot(&slots.as_number, |s| s.true_divide.load())
            }
            Self::NumRightTrueDivide => {
                Self::get_number_slot(&slots.as_number, |s| s.right_true_divide.load())
            }
            Self::NumInplaceTrueDivide => {
                Self::get_number_slot(&slots.as_number, |s| s.inplace_true_divide.load())
            }
            Self::NumMatrixMultiply => {
                Self::get_number_slot(&slots.as_number, |s| s.matrix_multiply.load())
            }
            Self::NumRightMatrixMultiply => {
                Self::get_number_slot(&slots.as_number, |s| s.right_matrix_multiply.load())
            }
            Self::NumInplaceMatrixMultiply => {
                Self::get_number_slot(&slots.as_number, |s| s.inplace_matrix_multiply.load())
            }
            Self::NumLshift => Self::get_number_slot(&slots.as_number, |s| s.lshift.load()),
            Self::NumRightLshift => {
                Self::get_number_slot(&slots.as_number, |s| s.right_lshift.load())
            }
            Self::NumInplaceLshift => {
                Self::get_number_slot(&slots.as_number, |s| s.inplace_lshift.load())
            }
            Self::NumRshift => Self::get_number_slot(&slots.as_number, |s| s.rshift.load()),
            Self::NumRightRshift => {
                Self::get_number_slot(&slots.as_number, |s| s.right_rshift.load())
            }
            Self::NumInplaceRshift => {
                Self::get_number_slot(&slots.as_number, |s| s.inplace_rshift.load())
            }
            Self::NumAnd => Self::get_number_slot(&slots.as_number, |s| s.and.load()),
            Self::NumRightAnd => Self::get_number_slot(&slots.as_number, |s| s.right_and.load()),
            Self::NumInplaceAnd => {
                Self::get_number_slot(&slots.as_number, |s| s.inplace_and.load())
            }
            Self::NumXor => Self::get_number_slot(&slots.as_number, |s| s.xor.load()),
            Self::NumRightXor => Self::get_number_slot(&slots.as_number, |s| s.right_xor.load()),
            Self::NumInplaceXor => {
                Self::get_number_slot(&slots.as_number, |s| s.inplace_xor.load())
            }
            Self::NumOr => Self::get_number_slot(&slots.as_number, |s| s.or.load()),
            Self::NumRightOr => Self::get_number_slot(&slots.as_number, |s| s.right_or.load()),
            Self::NumInplaceOr => Self::get_number_slot(&slots.as_number, |s| s.inplace_or.load()),

            // Number - power (ternary)
            Self::NumPower => slots.as_number.power.load().map(SlotFunc::NumTernary),
            Self::NumRightPower => slots.as_number.right_power.load().map(SlotFunc::NumTernary),
            Self::NumInplacePower => slots
                .as_number
                .inplace_power
                .load()
                .map(SlotFunc::NumTernary),

            // Number - unary
            Self::NumNegative => slots.as_number.negative.load().map(SlotFunc::NumUnary),
            Self::NumPositive => slots.as_number.positive.load().map(SlotFunc::NumUnary),
            Self::NumAbsolute => slots.as_number.absolute.load().map(SlotFunc::NumUnary),
            Self::NumInvert => slots.as_number.invert.load().map(SlotFunc::NumUnary),
            Self::NumBoolean => slots.as_number.boolean.load().map(SlotFunc::NumBoolean),
            Self::NumInt => slots.as_number.int.load().map(SlotFunc::NumUnary),
            Self::NumFloat => slots.as_number.float.load().map(SlotFunc::NumUnary),
            Self::NumIndex => slots.as_number.index.load().map(SlotFunc::NumUnary),

            // Sequence
            Self::SeqLength => slots.as_sequence.length.load().map(SlotFunc::SeqLength),
            Self::SeqConcat => slots.as_sequence.concat.load().map(SlotFunc::SeqConcat),
            Self::SeqRepeat => slots.as_sequence.repeat.load().map(SlotFunc::SeqRepeat),
            Self::SeqItem => slots.as_sequence.item.load().map(SlotFunc::SeqItem),
            Self::SeqAssItem => slots.as_sequence.ass_item.load().map(SlotFunc::SeqAssItem),
            Self::SeqContains => slots.as_sequence.contains.load().map(SlotFunc::SeqContains),
            Self::SeqInplaceConcat => slots
                .as_sequence
                .inplace_concat
                .load()
                .map(SlotFunc::SeqConcat),
            Self::SeqInplaceRepeat => slots
                .as_sequence
                .inplace_repeat
                .load()
                .map(SlotFunc::SeqRepeat),

            // Mapping
            Self::MapLength => slots.as_mapping.length.load().map(SlotFunc::MapLength),
            Self::MapSubscript => slots
                .as_mapping
                .subscript
                .load()
                .map(SlotFunc::MapSubscript),
            Self::MapAssSubscript => slots
                .as_mapping
                .ass_subscript
                .load()
                .map(SlotFunc::MapAssSubscript),
        }
    }

    /// Helper for number binary slots
    fn get_number_slot<F>(slots: &PyNumberSlots, getter: F) -> Option<SlotFunc>
    where
        F: FnOnce(&PyNumberSlots) -> Option<PyNumberBinaryFunc>,
    {
        getter(slots).map(SlotFunc::NumBinary)
    }
}

/// All slot definitions in a single static array
///
/// Adding a new slot: just add one entry here, and implement the
/// accessor methods - everything else is automatic.
pub static SLOT_DEFS: &[SlotDef] = &[
    // Main slots (tp_*)
    SlotDef {
        name: "__init__",
        accessor: SlotAccessor::Init,
        doc: "Initialize self. See help(type(self)) for accurate signature.",
    },
    SlotDef {
        name: "__new__",
        accessor: SlotAccessor::New,
        doc: "Create and return a new object. See help(type) for accurate signature.",
    },
    SlotDef {
        name: "__del__",
        accessor: SlotAccessor::Del,
        doc: "Called when the instance is about to be destroyed.",
    },
    SlotDef {
        name: "__repr__",
        accessor: SlotAccessor::Repr,
        doc: "Return repr(self).",
    },
    SlotDef {
        name: "__str__",
        accessor: SlotAccessor::Str,
        doc: "Return str(self).",
    },
    SlotDef {
        name: "__hash__",
        accessor: SlotAccessor::Hash,
        doc: "Return hash(self).",
    },
    SlotDef {
        name: "__call__",
        accessor: SlotAccessor::Call,
        doc: "Call self as a function.",
    },
    SlotDef {
        name: "__iter__",
        accessor: SlotAccessor::Iter,
        doc: "Implement iter(self).",
    },
    SlotDef {
        name: "__next__",
        accessor: SlotAccessor::IterNext,
        doc: "Implement next(self).",
    },
    // Attribute access
    SlotDef {
        name: "__getattribute__",
        accessor: SlotAccessor::GetAttro,
        doc: "Return getattr(self, name).",
    },
    SlotDef {
        name: "__setattr__",
        accessor: SlotAccessor::SetAttro,
        doc: "Implement setattr(self, name, value).",
    },
    SlotDef {
        name: "__delattr__",
        accessor: SlotAccessor::DelAttro,
        doc: "Implement delattr(self, name).",
    },
    // Rich comparison (all map to richcompare slot with different op)
    SlotDef {
        name: "__eq__",
        accessor: SlotAccessor::RichCompareEq,
        doc: "Return self==value.",
    },
    SlotDef {
        name: "__ne__",
        accessor: SlotAccessor::RichCompareNe,
        doc: "Return self!=value.",
    },
    SlotDef {
        name: "__lt__",
        accessor: SlotAccessor::RichCompareLt,
        doc: "Return self<value.",
    },
    SlotDef {
        name: "__le__",
        accessor: SlotAccessor::RichCompareLe,
        doc: "Return self<=value.",
    },
    SlotDef {
        name: "__gt__",
        accessor: SlotAccessor::RichCompareGt,
        doc: "Return self>value.",
    },
    SlotDef {
        name: "__ge__",
        accessor: SlotAccessor::RichCompareGe,
        doc: "Return self>=value.",
    },
    // Descriptor protocol
    SlotDef {
        name: "__get__",
        accessor: SlotAccessor::DescrGet,
        doc: "Return an attribute of instance, which is of type owner.",
    },
    SlotDef {
        name: "__set__",
        accessor: SlotAccessor::DescrSet,
        doc: "Set an attribute of instance to value.",
    },
    SlotDef {
        name: "__delete__",
        accessor: SlotAccessor::DescrDel,
        doc: "Delete an attribute of instance.",
    },
    // Sequence protocol (sq_*)
    SlotDef {
        name: "__len__",
        accessor: SlotAccessor::SeqLength,
        doc: "Return len(self).",
    },
    SlotDef {
        name: "__getitem__",
        accessor: SlotAccessor::SeqItem,
        doc: "Return self[key].",
    },
    SlotDef {
        name: "__setitem__",
        accessor: SlotAccessor::SeqAssItem,
        doc: "Set self[key] to value.",
    },
    SlotDef {
        name: "__delitem__",
        accessor: SlotAccessor::SeqAssItem,
        doc: "Delete self[key].",
    },
    SlotDef {
        name: "__contains__",
        accessor: SlotAccessor::SeqContains,
        doc: "Return key in self.",
    },
    SlotDef {
        name: "__add__",
        accessor: SlotAccessor::SeqConcat,
        doc: "Return self+value.",
    },
    SlotDef {
        name: "__mul__",
        accessor: SlotAccessor::SeqRepeat,
        doc: "Return self*value.",
    },
    SlotDef {
        name: "__iadd__",
        accessor: SlotAccessor::SeqInplaceConcat,
        doc: "Implement self+=value.",
    },
    SlotDef {
        name: "__imul__",
        accessor: SlotAccessor::SeqInplaceRepeat,
        doc: "Implement self*=value.",
    },
    // Mapping protocol (mp_*)
    SlotDef {
        name: "__len__",
        accessor: SlotAccessor::MapLength,
        doc: "Return len(self).",
    },
    SlotDef {
        name: "__getitem__",
        accessor: SlotAccessor::MapSubscript,
        doc: "Return self[key].",
    },
    SlotDef {
        name: "__setitem__",
        accessor: SlotAccessor::MapAssSubscript,
        doc: "Set self[key] to value.",
    },
    SlotDef {
        name: "__delitem__",
        accessor: SlotAccessor::MapAssSubscript,
        doc: "Delete self[key].",
    },
    // Number protocol - Binary operations (nb_*)
    SlotDef {
        name: "__add__",
        accessor: SlotAccessor::NumAdd,
        doc: "Return self+value.",
    },
    SlotDef {
        name: "__radd__",
        accessor: SlotAccessor::NumRightAdd,
        doc: "Return value+self.",
    },
    SlotDef {
        name: "__iadd__",
        accessor: SlotAccessor::NumInplaceAdd,
        doc: "Implement self+=value.",
    },
    SlotDef {
        name: "__sub__",
        accessor: SlotAccessor::NumSubtract,
        doc: "Return self-value.",
    },
    SlotDef {
        name: "__rsub__",
        accessor: SlotAccessor::NumRightSubtract,
        doc: "Return value-self.",
    },
    SlotDef {
        name: "__isub__",
        accessor: SlotAccessor::NumInplaceSubtract,
        doc: "Implement self-=value.",
    },
    SlotDef {
        name: "__mul__",
        accessor: SlotAccessor::NumMultiply,
        doc: "Return self*value.",
    },
    SlotDef {
        name: "__rmul__",
        accessor: SlotAccessor::NumRightMultiply,
        doc: "Return value*self.",
    },
    SlotDef {
        name: "__imul__",
        accessor: SlotAccessor::NumInplaceMultiply,
        doc: "Implement self*=value.",
    },
    SlotDef {
        name: "__mod__",
        accessor: SlotAccessor::NumRemainder,
        doc: "Return self%value.",
    },
    SlotDef {
        name: "__rmod__",
        accessor: SlotAccessor::NumRightRemainder,
        doc: "Return value%self.",
    },
    SlotDef {
        name: "__imod__",
        accessor: SlotAccessor::NumInplaceRemainder,
        doc: "Implement self%=value.",
    },
    SlotDef {
        name: "__divmod__",
        accessor: SlotAccessor::NumDivmod,
        doc: "Return divmod(self, value).",
    },
    SlotDef {
        name: "__rdivmod__",
        accessor: SlotAccessor::NumRightDivmod,
        doc: "Return divmod(value, self).",
    },
    SlotDef {
        name: "__pow__",
        accessor: SlotAccessor::NumPower,
        doc: "Return pow(self, value, mod).",
    },
    SlotDef {
        name: "__rpow__",
        accessor: SlotAccessor::NumRightPower,
        doc: "Return pow(value, self, mod).",
    },
    SlotDef {
        name: "__ipow__",
        accessor: SlotAccessor::NumInplacePower,
        doc: "Implement self**=value.",
    },
    SlotDef {
        name: "__floordiv__",
        accessor: SlotAccessor::NumFloorDivide,
        doc: "Return self//value.",
    },
    SlotDef {
        name: "__rfloordiv__",
        accessor: SlotAccessor::NumRightFloorDivide,
        doc: "Return value//self.",
    },
    SlotDef {
        name: "__ifloordiv__",
        accessor: SlotAccessor::NumInplaceFloorDivide,
        doc: "Implement self//=value.",
    },
    SlotDef {
        name: "__truediv__",
        accessor: SlotAccessor::NumTrueDivide,
        doc: "Return self/value.",
    },
    SlotDef {
        name: "__rtruediv__",
        accessor: SlotAccessor::NumRightTrueDivide,
        doc: "Return value/self.",
    },
    SlotDef {
        name: "__itruediv__",
        accessor: SlotAccessor::NumInplaceTrueDivide,
        doc: "Implement self/=value.",
    },
    SlotDef {
        name: "__matmul__",
        accessor: SlotAccessor::NumMatrixMultiply,
        doc: "Return self@value.",
    },
    SlotDef {
        name: "__rmatmul__",
        accessor: SlotAccessor::NumRightMatrixMultiply,
        doc: "Return value@self.",
    },
    SlotDef {
        name: "__imatmul__",
        accessor: SlotAccessor::NumInplaceMatrixMultiply,
        doc: "Implement self@=value.",
    },
    // Bitwise operations
    SlotDef {
        name: "__lshift__",
        accessor: SlotAccessor::NumLshift,
        doc: "Return self<<value.",
    },
    SlotDef {
        name: "__rlshift__",
        accessor: SlotAccessor::NumRightLshift,
        doc: "Return value<<self.",
    },
    SlotDef {
        name: "__ilshift__",
        accessor: SlotAccessor::NumInplaceLshift,
        doc: "Implement self<<=value.",
    },
    SlotDef {
        name: "__rshift__",
        accessor: SlotAccessor::NumRshift,
        doc: "Return self>>value.",
    },
    SlotDef {
        name: "__rrshift__",
        accessor: SlotAccessor::NumRightRshift,
        doc: "Return value>>self.",
    },
    SlotDef {
        name: "__irshift__",
        accessor: SlotAccessor::NumInplaceRshift,
        doc: "Implement self>>=value.",
    },
    SlotDef {
        name: "__and__",
        accessor: SlotAccessor::NumAnd,
        doc: "Return self&value.",
    },
    SlotDef {
        name: "__rand__",
        accessor: SlotAccessor::NumRightAnd,
        doc: "Return value&self.",
    },
    SlotDef {
        name: "__iand__",
        accessor: SlotAccessor::NumInplaceAnd,
        doc: "Implement self&=value.",
    },
    SlotDef {
        name: "__xor__",
        accessor: SlotAccessor::NumXor,
        doc: "Return self^value.",
    },
    SlotDef {
        name: "__rxor__",
        accessor: SlotAccessor::NumRightXor,
        doc: "Return value^self.",
    },
    SlotDef {
        name: "__ixor__",
        accessor: SlotAccessor::NumInplaceXor,
        doc: "Implement self^=value.",
    },
    SlotDef {
        name: "__or__",
        accessor: SlotAccessor::NumOr,
        doc: "Return self|value.",
    },
    SlotDef {
        name: "__ror__",
        accessor: SlotAccessor::NumRightOr,
        doc: "Return value|self.",
    },
    SlotDef {
        name: "__ior__",
        accessor: SlotAccessor::NumInplaceOr,
        doc: "Implement self|=value.",
    },
    // Number protocol - Unary operations
    SlotDef {
        name: "__neg__",
        accessor: SlotAccessor::NumNegative,
        doc: "Return -self.",
    },
    SlotDef {
        name: "__pos__",
        accessor: SlotAccessor::NumPositive,
        doc: "Return +self.",
    },
    SlotDef {
        name: "__abs__",
        accessor: SlotAccessor::NumAbsolute,
        doc: "Return abs(self).",
    },
    SlotDef {
        name: "__invert__",
        accessor: SlotAccessor::NumInvert,
        doc: "Return ~self.",
    },
    SlotDef {
        name: "__bool__",
        accessor: SlotAccessor::NumBoolean,
        doc: "Return self != 0.",
    },
    SlotDef {
        name: "__int__",
        accessor: SlotAccessor::NumInt,
        doc: "int(self)",
    },
    SlotDef {
        name: "__float__",
        accessor: SlotAccessor::NumFloat,
        doc: "float(self)",
    },
    SlotDef {
        name: "__index__",
        accessor: SlotAccessor::NumIndex,
        doc: "Return self converted to an integer, if self is suitable for use as an index into a list.",
    },
];

/// Find all slot definitions with a given name
pub fn find_slot_defs_by_name(name: &str) -> impl Iterator<Item = &'static SlotDef> {
    SLOT_DEFS.iter().filter(move |def| def.name == name)
}

/// Total number of slot definitions
pub const SLOT_DEFS_COUNT: usize = SLOT_DEFS.len();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slot_defs_count() {
        // Should have a reasonable number of entries
        assert!(
            SLOT_DEFS.len() > 60,
            "Expected at least 60 slot definitions"
        );
        assert!(SLOT_DEFS.len() < 150, "Too many slot definitions");
    }

    #[test]
    fn test_find_by_name() {
        // __len__ appears in both sequence and mapping
        let len_defs: Vec<_> = find_slot_defs_by_name("__len__").collect();
        assert_eq!(len_defs.len(), 2);

        // __init__ appears once
        let init_defs: Vec<_> = find_slot_defs_by_name("__init__").collect();
        assert_eq!(init_defs.len(), 1);

        // __add__ appears in sequence and number
        let add_defs: Vec<_> = find_slot_defs_by_name("__add__").collect();
        assert_eq!(add_defs.len(), 2);
    }

    #[test]
    fn test_repr_u8() {
        // Verify that SlotAccessor fits in u8
        assert!(std::mem::size_of::<SlotAccessor>() == 1);
    }

    #[test]
    fn test_rich_compare_op() {
        assert_eq!(
            SlotAccessor::RichCompareLt.rich_compare_op(),
            Some(PyComparisonOp::Lt)
        );
        assert_eq!(
            SlotAccessor::RichCompareEq.rich_compare_op(),
            Some(PyComparisonOp::Eq)
        );
        assert_eq!(SlotAccessor::Hash.rich_compare_op(), None);
    }
}
