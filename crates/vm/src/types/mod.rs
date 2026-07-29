pub mod c_slots;
mod slot;
pub mod slot_defs;
mod structseq;
mod zoo;

pub use c_slots::{CSlotId, CSlots, OwnedCSlots, c_new_trampoline};
pub use slot::*;
pub use slot_defs::{SLOT_DEFS, SlotAccessor, SlotDef};
pub use structseq::{PyStructSequence, PyStructSequenceData, struct_sequence_new};
pub(crate) use zoo::TypeZoo;
