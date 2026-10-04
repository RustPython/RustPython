use crate::attribute::OwnedAttribute;
use crate::common::{is_name_start_char, is_whitespace_char, Position};
use crate::namespace;
use crate::reader::error::SyntaxError;

use crate::reader::lexer::Token;

use super::{OpeningTagSubstate, PullParser, QualifiedNameTarget, Result, State};

impl PullParser {
    pub fn inside_opening_tag(&mut self, t: Token, s: OpeningTagSubstate) -> Option<Result> {
        match s {
            OpeningTagSubstate::InsideName => self.read_qualified_name(t, QualifiedNameTarget::OpeningTag, |this, token, name| {
                match name.prefix_ref() {
                    Some(prefix) if prefix == namespace::NS_XML_PREFIX ||
                                    prefix == namespace::NS_XMLNS_PREFIX =>
                        Some(this.error(SyntaxError::InvalidNamePrefix(prefix.into()))),
                    _ => {
                        this.data.element_name = Some(name.clone());
                        match token {
                            Token::TagEnd => this.emit_start_element(false),
                            Token::EmptyTagEnd => this.emit_start_element(true),
                            Token::Character(c) if is_whitespace_char(c) => this.into_state_continue(State::InsideOpeningTag(OpeningTagSubstate::InsideTag)),
                            _ => {
                                debug_assert!(false, "unreachable");
                                None
                            },
                        }
                    }
                }
            }),

            OpeningTagSubstate::InsideTag => match t {
                Token::TagEnd => self.emit_start_element(false),
                Token::EmptyTagEnd => self.emit_start_element(true),
                Token::Character(c) if is_whitespace_char(c) => None, // skip whitespace
                Token::Character(c) if is_name_start_char(c) => {
                    if self.qualified_name_buf.len() > self.config.max_name_length {
                        return Some(self.error(SyntaxError::ExceededConfiguredLimit));
                    }
                    self.data.attr_pos = Some(self.lexer.position());
                    self.qualified_name_buf.push(c);
                    self.into_state_continue(State::InsideOpeningTag(OpeningTagSubstate::InsideAttributeName))
                },
                _ => Some(self.error(SyntaxError::UnexpectedTokenInOpeningTag(t))),
            },

            OpeningTagSubstate::InsideAttributeName => self.read_qualified_name(t, QualifiedNameTarget::Attribute, |this, token, name| {
                this.data.attr_name = Some(name);
                match token {
                    Token::EqualsSign => this.into_state_continue(State::InsideOpeningTag(OpeningTagSubstate::InsideAttributeValue)),
                    Token::Character(c) if is_whitespace_char(c) => this.into_state_continue(State::InsideOpeningTag(OpeningTagSubstate::AfterAttributeName)),
                    _ => Some(this.error(SyntaxError::UnexpectedTokenInOpeningTag(t))) // likely unreachable
                }
            }),

            OpeningTagSubstate::AfterAttributeName => match t {
                Token::EqualsSign => {
                    self.into_state_continue(State::InsideOpeningTag(OpeningTagSubstate::InsideAttributeValue))
                },
                Token::Character(c) if is_whitespace_char(c) => None,
                _ => Some(self.error(SyntaxError::UnexpectedTokenInOpeningTag(t))),
            },

            OpeningTagSubstate::InsideAttributeValue => self.read_attribute_value(t, |this, value| {
                let name = this.data.take_attr_name()?;  // will always succeed here
                let pos = this.data.take_attr_pos();
                if this.config.amplification_limits.is_some() {
                    if this.pending_attributes.len() >= this.config.max_attributes {
                        return Some(this.error(SyntaxError::ExceededConfiguredLimit));
                    }
                    this.pending_attributes.push((name, value, pos));
                } else if let Err(e) = this.finish_attribute(name, value, pos) {
                    return Some(Err(e));
                }
                this.into_state_continue(State::InsideOpeningTag(OpeningTagSubstate::AfterAttributeValue))
            }),

            OpeningTagSubstate::AfterAttributeValue => match t {
                Token::Character(c) if is_whitespace_char(c) => {
                    self.into_state_continue(State::InsideOpeningTag(OpeningTagSubstate::InsideTag))
                },
                Token::TagEnd => self.emit_start_element(false),
                Token::EmptyTagEnd => self.emit_start_element(true),
                _ => Some(self.error(SyntaxError::UnexpectedTokenInOpeningTag(t))),
            },
        }
    }

    pub(super) fn finish_attribute(&mut self, name: crate::name::OwnedName, value: String,
                                  pos: Option<crate::common::TextPosition>) -> crate::reader::Result<()> {
        match name.prefix_ref() {
            Some(namespace::NS_XMLNS_PREFIX) => {
                let ln = &*name.local_name;
                let error = if ln == namespace::NS_XMLNS_PREFIX {
                    Some(SyntaxError::CannotRedefineXmlnsPrefix)
                } else if ln == namespace::NS_XML_PREFIX && &*value != namespace::NS_XML_URI {
                    Some(SyntaxError::CannotRedefineXmlPrefix)
                } else if value.is_empty() {
                    Some(SyntaxError::CannotUndefinePrefix(ln.into()))
                } else { None };
                if let Some(e) = error {
                    return Err(crate::reader::Error::syntax(e.to_cow(), self.lexer.position()));
                }
                self.nst.put(name.local_name, value);
            }
            None if &*name.local_name == namespace::NS_XMLNS_PREFIX => {
                match &*value {
                    namespace::NS_XMLNS_PREFIX | namespace::NS_XML_PREFIX | namespace::NS_XML_URI | namespace::NS_XMLNS_URI =>
                        return Err(crate::reader::Error::syntax(SyntaxError::InvalidDefaultNamespace(value.into()).to_cow(), self.lexer.position())),
                    _ => { self.nst.put(namespace::NS_NO_PREFIX, value); }
                }
            }
            _ => {
                if self.data.attributes.len() >= self.config.max_attributes {
                    return Err(crate::reader::Error::syntax(SyntaxError::ExceededConfiguredLimit.to_cow(), self.lexer.position()));
                }
                if let Some(pos) = pos { self.data.attribute_positions.push(pos); }
                self.data.attributes.push(OwnedAttribute { name, value });
            }
        }
        Ok(())
    }
}
