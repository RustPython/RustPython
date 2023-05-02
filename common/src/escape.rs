#[derive(Debug, Clone, Copy)]
pub enum Quote {
    Single,
    Double,
}

impl Quote {
    #[inline]
    pub const fn swap(self) -> Quote {
        match self {
            Quote::Single => Quote::Double,
            Quote::Double => Quote::Single,
        }
    }

    #[inline]
    pub const fn to_byte(&self) -> u8 {
        match self {
            Quote::Single => b'\'',
            Quote::Double => b'"',
        }
    }

    #[inline]
    pub const fn to_char(&self) -> char {
        match self {
            Quote::Single => '\'',
            Quote::Double => '"',
        }
    }
}

pub struct EscapeLayout {
    pub quote: Quote,
    pub len: Option<usize>,
}

pub trait Escape {
    type Source: ?Sized;

    fn source_len(&self) -> usize;
    fn layout(&self) -> &EscapeLayout;
    fn changed(&self) -> bool {
        self.layout().len != Some(self.source_len())
    }

    fn output_layout_with_checker(
        source: &Self::Source,
        preferred_quote: Quote,
        reserved_len: usize,
        length_add: impl Fn(usize, usize) -> Option<usize>,
    ) -> EscapeLayout;
    // fn output_layout(source: &Self::Source, preferred_quote: Quote) -> EscapeLayout {
    //     Self::output_layout_with_checker(source, preferred_quote, 2, |a, b| a.checked_add(b))
    // }
    fn output_layout(source: &Self::Source, preferred_quote: Quote) -> EscapeLayout {
        Self::output_layout_with_checker(source, preferred_quote, 2, |a, b| {
            Some((a as isize).checked_add(b as isize)? as usize)
        })
    }

    fn write_source(&self, formatter: &mut impl std::fmt::Write) -> std::fmt::Result;
    fn write_body_slow(&self, formatter: &mut impl std::fmt::Write) -> std::fmt::Result;
    fn write_body(&self, formatter: &mut impl std::fmt::Write) -> std::fmt::Result {
        if self.changed() {
            self.write_body_slow(formatter)
        } else {
            self.write_source(formatter)
        }
    }
    fn write_quoted(&self, formatter: &mut impl std::fmt::Write) -> std::fmt::Result {
        let quote = self.layout().quote.to_char();
        formatter.write_char(quote)?;
        self.write_body(formatter)?;
        formatter.write_char(quote)
    }
    fn to_quoted_string(&self) -> Option<String> {
        let len = self.layout().len?.checked_add(2)?;
        let mut s = String::with_capacity(len);
        self.write_quoted(&mut s).unwrap();
        Some(s)
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

    // always use primary unless we have primary but no seconday
    let use_secondary = primary_count > 0 && secondary_count == 0;
    if use_secondary {
        (preferred_quote.swap(), secondary_count)
    } else {
        (preferred_quote, primary_count)
    }
}

pub struct UnicodeEscape<'a> {
    source: &'a str,
    layout: EscapeLayout,
}

impl<'a> UnicodeEscape<'a> {
    pub fn with_forced_quote(source: &'a str, quote: Quote) -> Self {
        let layout = EscapeLayout { quote, len: None };
        Self { source, layout }
    }
    pub fn new_repr(source: &'a str) -> Self {
        let layout = Self::output_layout(source, Quote::Single);
        Self { source, layout }
    }
    pub fn repr<'r>(&'a self) -> UnicodeRepr<'r, 'a> {
        UnicodeRepr(self)
    }
}

pub struct UnicodeRepr<'r, 'a>(&'r UnicodeEscape<'a>);

impl std::fmt::Display for UnicodeRepr<'_, '_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.write_quoted(formatter)
    }
}

impl UnicodeEscape<'_> {
    fn escaped_char_len(ch: char) -> usize {
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
        ch: char,
        quote: Quote,
        formatter: &mut impl std::fmt::Write,
    ) -> std::fmt::Result {
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

impl<'a> Escape for UnicodeEscape<'a> {
    type Source = str;

    fn source_len(&self) -> usize {
        self.source.len()
    }

    fn layout(&self) -> &EscapeLayout {
        &self.layout
    }

    fn output_layout_with_checker(
        source: &str,
        preferred_quote: Quote,
        reserved_len: usize,
        length_add: impl Fn(usize, usize) -> Option<usize>,
    ) -> EscapeLayout {
        let mut out_len = reserved_len;
        let mut single_count = 0;
        let mut double_count = 0;

        for ch in source.chars() {
            let incr = match ch {
                '\'' => {
                    single_count += 1;
                    1
                }
                '"' => {
                    double_count += 1;
                    1
                }
                c => Self::escaped_char_len(c),
            };
            let Some(new_len) = length_add(out_len, incr) else {
                #[cold]
                fn stop(single_count: usize, double_count: usize, preferred_quote: Quote) -> EscapeLayout {
                    EscapeLayout { quote: choose_quote(single_count, double_count, preferred_quote).0, len: None }
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

    fn write_source(&self, formatter: &mut impl std::fmt::Write) -> std::fmt::Result {
        formatter.write_str(self.source)
    }

    #[cold]
    fn write_body_slow(&self, formatter: &mut impl std::fmt::Write) -> std::fmt::Result {
        for ch in self.source.chars() {
            Self::write_char(ch, self.layout().quote, formatter)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod unicode_escapse_tests {
    use super::*;

    #[test]
    fn changed() {
        fn test(s: &str) -> bool {
            UnicodeEscape::new_repr(s).changed()
        }
        assert!(!test("hello"));
        assert!(!test("'hello'"));
        assert!(!test("\"hello\""));

        assert!(test("'\"hello"));
        assert!(test("hello\n"));
    }
}
