//! Unicode aliases.
//!
//! This module contains a map of unicode aliases to their corresponding values. The map is generated
//! from the [NamesList.txt] file in the Unicode Character Database.
//!
//! [NamesList.txt]: https://www.unicode.org/Public/14.0.0/ucd/NameAliases.txt

// generated in build.rs, in gen_unicode_aliases()
/// A map of unicode aliases to their corresponding values.
static ALIASES: phf::Map<&'static str, char> = include!(concat!(env!("OUT_DIR"), "/aliases.rs"));

/// Get alias value from alias name, returns `None` if the alias is not found.
///
/// # Examples
///
/// ```
/// use rustpython_common::unicode_aliases::unicode_alias;
///
/// assert_eq!(unicode_alias("NEW LINE"), Some('\n'));
/// assert_eq!(unicode_alias("BACKSPACE"), Some('\u{8}'));
/// assert_eq!(unicode_alias("NOT AN ALIAS"), None);
pub fn unicode_alias(alias: &str) -> Option<char> {
    ALIASES.get(alias).copied()
}
