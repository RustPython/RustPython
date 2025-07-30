use rustpython_wtf8::{CodePoint, Wtf8};

#[derive(Debug, PartialEq, Eq, Copy, Clone, Hash, is_macro::Is)]
pub enum Quote {
    Single,
    Double,
}

impl Quote {
    #[inline]
    pub const fn swap(self) -> Self {
        match self {
            Self::Single => Self::Double,
            Self::Double => Self::Single,
        }
    }

    #[inline]
    pub const fn to_byte(&self) -> u8 {
        match self {
            Self::Single => b'\'',
            Self::Double => b'"',
        }
    }

    #[inline]
    pub const fn to_char(&self) -> char {
        match self {
            Self::Single => '\'',
            Self::Double => '"',
        }
    }
}

pub struct EscapeLayout {
    pub quote: Quote,
    pub len: Option<usize>,
}

/// Represents string types that can be escape-printed.
///
/// # Safety
///
/// `source_len` and `layout` must be accurate, and `layout.len` must not be equal
/// to `Some(source_len)` if the string contains non-printable characters.
pub unsafe trait Escape {
    fn source_len(&self) -> usize;
    fn layout(&self) -> &EscapeLayout;
    fn changed(&self) -> bool {
        self.layout().len != Some(self.source_len())
    }

    /// Write the body of the string directly to the formatter.
    ///
    /// # Safety
    ///
    /// This string must only contain printable characters.
    unsafe fn write_source(&self, formatter: &mut impl std::fmt::Write) -> std::fmt::Result;
    fn write_body_slow(&self, formatter: &mut impl std::fmt::Write) -> std::fmt::Result;
    fn write_body(&self, formatter: &mut impl std::fmt::Write) -> std::fmt::Result {
        if self.changed() {
            self.write_body_slow(formatter)
        } else {
            // SAFETY: verified the string contains only printable characters.
            unsafe { self.write_source(formatter) }
        }
    }
}

/// Returns the outer quotes to use and the number of quotes that need to be
/// escaped.
pub(crate) const fn choose_quote(
    single_count: usize,
    double_count: usize,
    preferred_quote: Quote,
) -> (Quote, usize) {
    let (primary_count, secondary_count) = match preferred_quote {
        Quote::Single => (single_count, double_count),
        Quote::Double => (double_count, single_count),
    };

    // always use primary unless we have primary but no secondary
    let use_secondary = primary_count > 0 && secondary_count == 0;
    if use_secondary {
        (preferred_quote.swap(), secondary_count)
    } else {
        (preferred_quote, primary_count)
    }
}

pub struct UnicodeEscape<'a> {
    source: &'a Wtf8,
    layout: EscapeLayout,
}

impl<'a> UnicodeEscape<'a> {
    #[inline]
    pub const fn with_forced_quote(source: &'a Wtf8, quote: Quote) -> Self {
        let layout = EscapeLayout { quote, len: None };
        Self { source, layout }
    }
    #[inline]
    pub fn with_preferred_quote(source: &'a Wtf8, quote: Quote) -> Self {
        let layout = Self::repr_layout(source, quote);
        Self { source, layout }
    }
    #[inline]
    pub fn new_repr(source: &'a Wtf8) -> Self {
        Self::with_preferred_quote(source, Quote::Single)
    }
    #[inline]
    pub const fn str_repr<'r>(&'a self) -> StrRepr<'r, 'a> {
        StrRepr(self)
    }
}

pub struct StrRepr<'r, 'a>(&'r UnicodeEscape<'a>);

impl StrRepr<'_, '_> {
    pub fn write(&self, formatter: &mut impl std::fmt::Write) -> std::fmt::Result {
        let quote = self.0.layout().quote.to_char();
        formatter.write_char(quote)?;
        self.0.write_body(formatter)?;
        formatter.write_char(quote)
    }

    pub fn to_string(&self) -> Option<String> {
        let mut s = String::with_capacity(self.0.layout().len?);
        self.write(&mut s).unwrap();
        Some(s)
    }
}

impl std::fmt::Display for StrRepr<'_, '_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.write(formatter)
    }
}

impl UnicodeEscape<'_> {
    const REPR_RESERVED_LEN: usize = 2; // for quotes

    pub fn repr_layout(source: &Wtf8, preferred_quote: Quote) -> EscapeLayout {
        Self::output_layout_with_checker(source, preferred_quote, |a, b| {
            Some((a as isize).checked_add(b as isize)? as usize)
        })
    }

    fn output_layout_with_checker(
        source: &Wtf8,
        preferred_quote: Quote,
        length_add: impl Fn(usize, usize) -> Option<usize>,
    ) -> EscapeLayout {
        let mut out_len = Self::REPR_RESERVED_LEN;
        let mut single_count = 0;
        let mut double_count = 0;

        for ch in source.code_points() {
            let incr = match ch.to_char() {
                Some('\'') => {
                    single_count += 1;
                    1
                }
                Some('"') => {
                    double_count += 1;
                    1
                }
                _ => Self::escaped_char_len(ch),
            };
            let Some(new_len) = length_add(out_len, incr) else {
                #[cold]
                const fn stop(
                    single_count: usize,
                    double_count: usize,
                    preferred_quote: Quote,
                ) -> EscapeLayout {
                    EscapeLayout {
                        quote: choose_quote(single_count, double_count, preferred_quote).0,
                        len: None,
                    }
                }
                return stop(single_count, double_count, preferred_quote);
            };
            out_len = new_len;
        }

        let (quote, num_escaped_quotes) = choose_quote(single_count, double_count, preferred_quote);
        // we'll be adding backslashes in front of the existing inner quotes
        let Some(out_len) = length_add(out_len, num_escaped_quotes) else {
            return EscapeLayout { quote, len: None };
        };

        EscapeLayout {
            quote,
            len: Some(out_len - Self::REPR_RESERVED_LEN),
        }
    }

    fn escaped_char_len(ch: CodePoint) -> usize {
        // surrogates are \uHHHH
        let Some(ch) = ch.to_char() else { return 6 };
        match ch {
            '\\' | '\t' | '\r' | '\n' => 2,
            ch if ch < ' ' || ch as u32 == 0x7f => 4, // \xHH
            ch if ch.is_ascii() => 1,
            ch if crate::char::is_printable(ch) => {
                // max = std::cmp::max(ch, max);
                ch.len_utf8()
            }
            ch if (ch as u32) < 0x100 => 4,   // \xHH
            ch if (ch as u32) < 0x10000 => 6, // \uHHHH
            _ => 10,                          // \uHHHHHHHH
        }
    }

    fn write_char(
        ch: CodePoint,
        quote: Quote,
        formatter: &mut impl std::fmt::Write,
    ) -> std::fmt::Result {
        let Some(ch) = ch.to_char() else {
            return write!(formatter, "\\u{:04x}", ch.to_u32());
        };
        match ch {
            '\n' => formatter.write_str("\\n"),
            '\t' => formatter.write_str("\\t"),
            '\r' => formatter.write_str("\\r"),
            // these 2 branches *would* be handled below, but we shouldn't have to do a
            // unicodedata lookup just for ascii characters
            '\x20'..='\x7e' => {
                // printable ascii range
                if ch == quote.to_char() || ch == '\\' {
                    formatter.write_char('\\')?;
                }
                formatter.write_char(ch)
            }
            ch if ch.is_ascii() => {
                write!(formatter, "\\x{:02x}", ch as u8)
            }
            ch if crate::char::is_printable(ch) => formatter.write_char(ch),
            '\0'..='\u{ff}' => {
                write!(formatter, "\\x{:02x}", ch as u32)
            }
            '\0'..='\u{ffff}' => {
                write!(formatter, "\\u{:04x}", ch as u32)
            }
            _ => {
                write!(formatter, "\\U{:08x}", ch as u32)
            }
        }
    }
}

unsafe impl Escape for UnicodeEscape<'_> {
    fn source_len(&self) -> usize {
        self.source.len()
    }

    fn layout(&self) -> &EscapeLayout {
        &self.layout
    }

    unsafe fn write_source(&self, formatter: &mut impl std::fmt::Write) -> std::fmt::Result {
        formatter.write_str(unsafe {
            // SAFETY: this function must be called only when source is printable characters (i.e. no surrogates)
            std::str::from_utf8_unchecked(self.source.as_bytes())
        })
    }

    #[cold]
    fn write_body_slow(&self, formatter: &mut impl std::fmt::Write) -> std::fmt::Result {
        for ch in self.source.code_points() {
            Self::write_char(ch, self.layout().quote, formatter)?;
        }
        Ok(())
    }
}

pub struct AsciiEscape<'a> {
    source: &'a [u8],
    layout: EscapeLayout,
}

impl<'a> AsciiEscape<'a> {
    #[inline]
    pub const fn new(source: &'a [u8], layout: EscapeLayout) -> Self {
        Self { source, layout }
    }
    #[inline]
    pub const fn with_forced_quote(source: &'a [u8], quote: Quote) -> Self {
        let layout = EscapeLayout { quote, len: None };
        Self { source, layout }
    }
    #[inline]
    pub fn with_preferred_quote(source: &'a [u8], quote: Quote) -> Self {
        let layout = Self::repr_layout(source, quote);
        Self { source, layout }
    }
    #[inline]
    pub fn new_repr(source: &'a [u8]) -> Self {
        Self::with_preferred_quote(source, Quote::Single)
    }
    #[inline]
    pub const fn bytes_repr<'r>(&'a self) -> BytesRepr<'r, 'a> {
        BytesRepr(self)
    }
}

impl AsciiEscape<'_> {
    pub fn repr_layout(source: &[u8], preferred_quote: Quote) -> EscapeLayout {
        Self::output_layout_with_checker(source, preferred_quote, 3, |a, b| {
            Some((a as isize).checked_add(b as isize)? as usize)
        })
    }

    pub fn named_repr_layout(source: &[u8], name: &str) -> EscapeLayout {
        Self::output_layout_with_checker(source, Quote::Single, name.len() + 2 + 3, |a, b| {
            Some((a as isize).checked_add(b as isize)? as usize)
        })
    }

    fn output_layout_with_checker(
        source: &[u8],
        preferred_quote: Quote,
        reserved_len: usize,
        length_add: impl Fn(usize, usize) -> Option<usize>,
    ) -> EscapeLayout {
        let mut out_len = reserved_len;
        let mut single_count = 0;
        let mut double_count = 0;

        for ch in source {
            let incr = match ch {
                b'\'' => {
                    single_count += 1;
                    1
                }
                b'"' => {
                    double_count += 1;
                    1
                }
                c => Self::escaped_char_len(*c),
            };
            let Some(new_len) = length_add(out_len, incr) else {
                #[cold]
                const fn stop(
                    single_count: usize,
                    double_count: usize,
                    preferred_quote: Quote,
                ) -> EscapeLayout {
                    EscapeLayout {
                        quote: choose_quote(single_count, double_count, preferred_quote).0,
                        len: None,
                    }
                }
                return stop(single_count, double_count, preferred_quote);
            };
            out_len = new_len;
        }

        let (quote, num_escaped_quotes) = choose_quote(single_count, double_count, preferred_quote);
        // we'll be adding backslashes in front of the existing inner quotes
        let Some(out_len) = length_add(out_len, num_escaped_quotes) else {
            return EscapeLayout { quote, len: None };
        };

        EscapeLayout {
            quote,
            len: Some(out_len - reserved_len),
        }
    }

    const fn escaped_char_len(ch: u8) -> usize {
        match ch {
            b'\\' | b'\t' | b'\r' | b'\n' => 2,
            0x20..=0x7e => 1,
            _ => 4, // \xHH
        }
    }

    fn write_char(ch: u8, quote: Quote, formatter: &mut impl std::fmt::Write) -> std::fmt::Result {
        match ch {
            b'\t' => formatter.write_str("\\t"),
            b'\n' => formatter.write_str("\\n"),
            b'\r' => formatter.write_str("\\r"),
            0x20..=0x7e => {
                // printable ascii range
                if ch == quote.to_byte() || ch == b'\\' {
                    formatter.write_char('\\')?;
                }
                formatter.write_char(ch as char)
            }
            ch => write!(formatter, "\\x{ch:02x}"),
        }
    }
}

unsafe impl Escape for AsciiEscape<'_> {
    fn source_len(&self) -> usize {
        self.source.len()
    }

    fn layout(&self) -> &EscapeLayout {
        &self.layout
    }

    unsafe fn write_source(&self, formatter: &mut impl std::fmt::Write) -> std::fmt::Result {
        formatter.write_str(unsafe {
            // SAFETY: this function must be called only when source is printable ascii characters
            std::str::from_utf8_unchecked(self.source)
        })
    }

    #[cold]
    fn write_body_slow(&self, formatter: &mut impl std::fmt::Write) -> std::fmt::Result {
        for ch in self.source {
            Self::write_char(*ch, self.layout().quote, formatter)?;
        }
        Ok(())
    }
}

pub struct BytesRepr<'r, 'a>(&'r AsciiEscape<'a>);

impl BytesRepr<'_, '_> {
    pub fn write(&self, formatter: &mut impl std::fmt::Write) -> std::fmt::Result {
        let quote = self.0.layout().quote.to_char();
        formatter.write_char('b')?;
        formatter.write_char(quote)?;
        self.0.write_body(formatter)?;
        formatter.write_char(quote)
    }

    pub fn to_string(&self) -> Option<String> {
        let mut s = String::with_capacity(self.0.layout().len?);
        self.write(&mut s).unwrap();
        Some(s)
    }
}

impl std::fmt::Display for BytesRepr<'_, '_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.write(formatter)
    }
}

#[cfg(test)]
mod unicode_escape_tests {
    use super::*;

    #[test]
    fn changed() {
        fn test(s: &str) -> bool {
            UnicodeEscape::new_repr(s.as_ref()).changed()
        }
        assert!(!test("hello"));
        assert!(!test("'hello'"));
        assert!(!test("\"hello\""));

        assert!(test("'\"hello"));
        assert!(test("hello\n"));
    }
}
