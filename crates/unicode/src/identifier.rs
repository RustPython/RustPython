use icu_properties::props::{BinaryProperty, XidContinue, XidStart};

use crate::char_from_codepoint;

pub fn is_xid_start(cp: u32) -> bool {
    char_from_codepoint(cp).is_some_and(XidStart::for_char)
}

pub fn is_xid_continue(cp: u32) -> bool {
    char_from_codepoint(cp).is_some_and(XidContinue::for_char)
}

pub fn is_python_identifier_start(cp: u32) -> bool {
    cp == '_' as u32 || is_xid_start(cp)
}

pub fn is_python_identifier_continue(cp: u32) -> bool {
    is_xid_continue(cp)
}

pub fn is_python_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    let is_identifier_start = chars
        .next()
        .is_some_and(|ch| is_python_identifier_start(ch as u32));
    is_identifier_start && chars.all(|ch| is_python_identifier_continue(ch as u32))
}
