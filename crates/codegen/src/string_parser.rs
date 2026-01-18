//! A stripped-down version of ruff's string literal parser, modified to
//! handle surrogates in string literals and output WTF-8.
//!
//! Any `unreachable!()` statements in this file are because we only get here
//! after ruff has already successfully parsed the string literal, meaning
//! we don't need to do any validation or error handling.

use core::convert::Infallible;

use ruff_python_ast::{self as ast, StringFlags as _};
use rustpython_wtf8::{CodePoint, Wtf8, Wtf8Buf};

// use ruff_python_parser::{LexicalError, LexicalErrorType};
type LexicalError = Infallible;

enum EscapedChar {
    Literal(CodePoint),
    Escape(char),
}

struct StringParser {
    /// The raw content of the string e.g., the `foo` part in `"foo"`.
    source: Box<str>,
    /// Current position of the parser in the source.
    cursor: usize,
    /// Flags that can be used to query information about the string.
    flags: ast::AnyStringFlags,
}

impl StringParser {
    const fn new(source: Box<str>, flags: ast::AnyStringFlags) -> Self {
        Self {
            source,
            cursor: 0,
            flags,
        }
    }

    #[inline]
    fn skip_bytes(&mut self, bytes: usize) -> &str {
        let skipped_str = &self.source[self.cursor..self.cursor + bytes];
        self.cursor += bytes;
        skipped_str
    }

    /// Returns the next byte in the string, if there is one.
    ///
    /// # Panics
    ///
    /// When the next byte is a part of a multi-byte character.
    #[inline]
    fn next_byte(&mut self) -> Option<u8> {
        self.source[self.cursor..].as_bytes().first().map(|&byte| {
            self.cursor += 1;
            byte
        })
    }

    #[inline]
    fn next_char(&mut self) -> Option<char> {
        self.source[self.cursor..].chars().next().inspect(|c| {
            self.cursor += c.len_utf8();
        })
    }

    #[inline]
    fn peek_byte(&self) -> Option<u8> {
        self.source[self.cursor..].as_bytes().first().copied()
    }

    fn parse_unicode_literal(&mut self, literal_number: usize) -> Result<CodePoint, LexicalError> {
        let mut p: u32 = 0u32;
        for i in 1..=literal_number {
            match self.next_char() {
                Some(c) => match c.to_digit(16) {
                    Some(d) => p += d << ((literal_number - i) * 4),
                    None => unreachable!(),
                },
                None => unreachable!(),
            }
        }
        Ok(CodePoint::from_u32(p).unwrap())
    }

    fn parse_octet(&mut self, o: u8) -> char {
        let mut radix_bytes = [o, 0, 0];
        let mut len = 1;

        while len < 3 {
            let Some(b'0'..=b'7') = self.peek_byte() else {
                break;
            };

            radix_bytes[len] = self.next_byte().unwrap();
            len += 1;
        }

        // OK because radix_bytes is always going to be in the ASCII range.
        let radix_str = core::str::from_utf8(&radix_bytes[..len]).expect("ASCII bytes");
        let value = u32::from_str_radix(radix_str, 8).unwrap();
        char::from_u32(value).unwrap()
    }

    fn parse_unicode_name(&mut self) -> Result<char, LexicalError> {
        let Some('{') = self.next_char() else {
            unreachable!()
        };

        let Some(close_idx) = self.source[self.cursor..].find('}') else {
            unreachable!()
        };

        let name_and_ending = self.skip_bytes(close_idx + 1);
        let name = &name_and_ending[..name_and_ending.len() - 1];

        unicode_names2::character(name).ok_or_else(|| unreachable!())
    }

    /// Parse an escaped character, returning the new character.
    fn parse_escaped_char(&mut self) -> Result<Option<EscapedChar>, LexicalError> {
        let Some(first_char) = self.next_char() else {
            unreachable!()
        };

        let new_char = match first_char {
            '\\' => '\\'.into(),
            '\'' => '\''.into(),
            '\"' => '"'.into(),
            'a' => '\x07'.into(),
            'b' => '\x08'.into(),
            'f' => '\x0c'.into(),
            'n' => '\n'.into(),
            'r' => '\r'.into(),
            't' => '\t'.into(),
            'v' => '\x0b'.into(),
            o @ '0'..='7' => self.parse_octet(o as u8).into(),
            'x' => self.parse_unicode_literal(2)?,
            'u' if !self.flags.is_byte_string() => self.parse_unicode_literal(4)?,
            'U' if !self.flags.is_byte_string() => self.parse_unicode_literal(8)?,
            'N' if !self.flags.is_byte_string() => self.parse_unicode_name()?.into(),
            // Special cases where the escape sequence is not a single character
            '\n' => return Ok(None),
            '\r' => {
                if self.peek_byte() == Some(b'\n') {
                    self.next_byte();
                }

                return Ok(None);
            }
            _ => return Ok(Some(EscapedChar::Escape(first_char))),
        };

        Ok(Some(EscapedChar::Literal(new_char)))
    }

    fn parse_fstring_middle(mut self) -> Result<Box<Wtf8>, LexicalError> {
        // Fast-path: if the f-string doesn't contain any escape sequences, return the literal.
        let Some(mut index) = memchr::memchr3(b'{', b'}', b'\\', self.source.as_bytes()) else {
            return Ok(self.source.into());
        };

        let mut value = Wtf8Buf::with_capacity(self.source.len());
        loop {
            // Add the characters before the escape sequence (or curly brace) to the string.
            let before_with_slash_or_brace = self.skip_bytes(index + 1);
            let before = &before_with_slash_or_brace[..before_with_slash_or_brace.len() - 1];
            value.push_str(before);

            // Add the escaped character to the string.
            match &self.source.as_bytes()[self.cursor - 1] {
                // If there are any curly braces inside a `FStringMiddle` token,
                // then they were escaped (i.e. `{{` or `}}`). This means that
                // we need increase the location by 2 instead of 1.
                b'{' => value.push_char('{'),
                b'}' => value.push_char('}'),
                // We can encounter a `\` as the last character in a `FStringMiddle`
                // token which is valid in this context. For example,
                //
                // ```python
                // f"\{foo} \{bar:\}"
                // # ^     ^^     ^
                // ```
                //
                // Here, the `FStringMiddle` token content will be "\" and " \"
                // which is invalid if we look at the content in isolation:
                //
                // ```python
                // "\"
                // ```
                //
                // However, the content is syntactically valid in the context of
                // the f-string because it's a substring of the entire f-string.
                // This is still an invalid escape sequence, but we don't want to
                // raise a syntax error as is done by the CPython parser. It might
                // be supported in the future, refer to point 3: https://peps.python.org/pep-0701/#rejected-ideas
                b'\\' => {
                    if !self.flags.is_raw_string() && self.peek_byte().is_some() {
                        match self.parse_escaped_char()? {
                            None => {}
                            Some(EscapedChar::Literal(c)) => value.push(c),
                            Some(EscapedChar::Escape(c)) => {
                                value.push_char('\\');
                                value.push_char(c);
                            }
                        }
                    } else {
                        value.push_char('\\');
                    }
                }
                ch => {
                    unreachable!("Expected '{{', '}}', or '\\' but got {:?}", ch);
                }
            }

            let Some(next_index) =
                memchr::memchr3(b'{', b'}', b'\\', self.source[self.cursor..].as_bytes())
            else {
                // Add the rest of the string to the value.
                let rest = &self.source[self.cursor..];
                value.push_str(rest);
                break;
            };

            index = next_index;
        }

        Ok(value.into())
    }

    fn parse_string(mut self) -> Result<Box<Wtf8>, LexicalError> {
        if self.flags.is_raw_string() {
            // For raw strings, no escaping is necessary.
            return Ok(self.source.into());
        }

        let Some(mut escape) = memchr::memchr(b'\\', self.source.as_bytes()) else {
            // If the string doesn't contain any escape sequences, return the owned string.
            return Ok(self.source.into());
        };

        // If the string contains escape sequences, we need to parse them.
        let mut value = Wtf8Buf::with_capacity(self.source.len());

        loop {
            // Add the characters before the escape sequence to the string.
            let before_with_slash = self.skip_bytes(escape + 1);
            let before = &before_with_slash[..before_with_slash.len() - 1];
            value.push_str(before);

            // Add the escaped character to the string.
            match self.parse_escaped_char()? {
                None => {}
                Some(EscapedChar::Literal(c)) => value.push(c),
                Some(EscapedChar::Escape(c)) => {
                    value.push_char('\\');
                    value.push_char(c);
                }
            }

            let Some(next_escape) = self.source[self.cursor..].find('\\') else {
                // Add the rest of the string to the value.
                let rest = &self.source[self.cursor..];
                value.push_str(rest);
                break;
            };

            // Update the position of the next escape sequence.
            escape = next_escape;
        }

        Ok(value.into())
    }
}

pub(crate) fn parse_string_literal(source: &str, flags: ast::AnyStringFlags) -> Box<Wtf8> {
    let source = &source[flags.opener_len().to_usize()..];
    let source = &source[..source.len() - flags.quote_len().to_usize()];
    StringParser::new(source.into(), flags)
        .parse_string()
        .unwrap_or_else(|x| match x {})
}

pub(crate) fn parse_fstring_literal_element(
    source: Box<str>,
    flags: ast::AnyStringFlags,
) -> Box<Wtf8> {
    StringParser::new(source, flags)
        .parse_fstring_middle()
        .unwrap_or_else(|x| match x {})
}
