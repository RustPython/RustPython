//! Python identifier predicates (`str.isidentifier`).

use icu_properties::props::{BinaryProperty, XidContinue, XidStart};

/// Whether `c` has the `XID_Start` property.
#[must_use]
pub fn is_xid_start(c: char) -> bool {
    XidStart::for_char(c)
}

/// Whether `c` has the `XID_Continue` property.
#[must_use]
pub fn is_xid_continue(c: char) -> bool {
    XidContinue::for_char(c)
}

/// Whether `c` may start a Python identifier: `_` or `XID_Start`.
#[must_use]
pub fn is_start(c: char) -> bool {
    c == '_' || is_xid_start(c)
}

/// Whether `c` may continue a Python identifier: `XID_Continue`.
#[must_use]
pub fn is_continue(c: char) -> bool {
    is_xid_continue(c)
}
