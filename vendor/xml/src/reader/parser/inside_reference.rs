use super::{PullParser, Result, State};
use crate::common::{is_name_char, is_name_start_char, is_whitespace_char};
use crate::reader::error::SyntaxError;
use crate::reader::lexer::Token;
use std::char;

impl PullParser {
    pub fn inside_reference(&mut self, t: Token) -> Option<Result> {
        match t {
            Token::Character(c) if !self.data.ref_data.is_empty() && is_name_char(c) ||
                             self.data.ref_data.is_empty() && (is_name_start_char(c) || c == '#') => {
                self.data.ref_data.push(c);
                None
            },

            Token::ReferenceEnd => {
                let name = self.data.take_ref_data();
                if name.is_empty() {
                    return Some(self.error(SyntaxError::EmptyEntity));
                }

                // Opening-tag attributes are expanded after the entire raw
                // tag has been counted, including namespace declarations.
                if self.config.amplification_limits.is_some()
                    && matches!(self.state_after_reference, State::InsideOpeningTag(_)) {
                    self.buf.push('&');
                    self.buf.push_str(&name);
                    self.buf.push(';');
                    return self.into_state_continue(self.state_after_reference);
                }
                if let Err(e) = self.lexer.check_amplification() { return Some(Err(e)); }
                let c = match &*name {
                    "lt"   => Some('<'),
                    "gt"   => Some('>'),
                    "amp"  => Some('&'),
                    "apos" => Some('\''),
                    "quot" => Some('"'),
                    _ if name.starts_with('#') => match self.numeric_reference_from_str(&name[1..]) {
                        Ok(c) => Some(c),
                        Err(e) => return Some(self.error(e)),
                    },
                    _ => None,
                };
                if let Some(c) = c {
                    // Numeric references are part of the input token; the five
                    // named predefined replacements count as indirect bytes.
                    if !name.starts_with('#') {
                        if let Err(e) = self.lexer.account_indirect(c.len_utf8(), false) { return Some(Err(e)); }
                    }
                    self.buf.push(c);
                } else if let Some(v) = self.config.extra_entities.get(&name) {
                    if let Err(e) = self.lexer.account_indirect(v.len(), true) { return Some(Err(e)); }
                    self.buf.push_str(v);
                } else if let Some(v) = self.entities.get(&name) {
                    if self.state_after_reference == State::OutsideTag {
                        // an entity can expand to *elements*, so outside of a tag it needs a full reparse
                        if let Err(e) = self.lexer.reparse(v) {
                            return Some(Err(e));
                        }
                    } else {
                        // however, inside attributes it's not allowed to affect attribute quoting,
                        // so it can't be fed to the lexer
                        if let Err(e) = self.lexer.account_indirect(v.len(), true) { return Some(Err(e)); }
                        self.buf.push_str(v);
                    }
                } else {
                    return Some(self.error(SyntaxError::UnexpectedEntity(name.into())));
                }
                let prev_st = self.state_after_reference;
                if prev_st == State::OutsideTag && !is_whitespace_char(self.buf.chars().last().unwrap_or('\0')) {
                    self.inside_whitespace = false;
                }
                if prev_st == State::OutsideTag && c.is_some()
                    && self.config.amplification_limits.is_some() && !self.config.coalesce_characters {
                    let value = self.take_buf();
                    self.push_pos();
                    return self.into_state_emit(prev_st, Ok(crate::reader::XmlEvent::Characters(value)));
                }
                self.into_state_continue(prev_st)
            },

            _ => Some(self.error(SyntaxError::UnexpectedTokenInEntity(t))),
        }
    }

    pub(crate) fn numeric_reference_from_str(&self, num_str: &str) -> std::result::Result<char, SyntaxError> {
        let val = if let Some(hex) = num_str.strip_prefix('x') {
            u32::from_str_radix(hex, 16).map_err(move |_| SyntaxError::InvalidNumericEntity(num_str.into()))?
        } else {
            num_str.parse::<u32>().map_err(move |_| SyntaxError::InvalidNumericEntity(num_str.into()))?
        };
        match char::from_u32(val) {
            Some(c) if self.is_valid_xml_char(c) => Ok(c),
            Some(_) if self.config.replace_unknown_entity_references => Ok('\u{fffd}'),
            None if self.config.replace_unknown_entity_references => Ok('\u{fffd}'),
            _ => Err(SyntaxError::InvalidCharacterEntity(val)),
        }
    }
}
