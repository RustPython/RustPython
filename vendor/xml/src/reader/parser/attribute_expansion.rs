use super::PullParser;
use crate::common::Position;
use crate::reader::error::SyntaxError;
use crate::reader::{Error, Result};

impl PullParser {
    pub(super) fn expand_attribute(&mut self, raw: &str) -> Result<String> {
        let mut value = String::new();
        Self::expand_attribute_part(
            raw,
            0,
            &self.entities,
            &self.config,
            self.data.version,
            &mut self.lexer,
            &mut value,
        )?;
        Ok(value)
    }

    fn expand_attribute_part(
        raw: &str,
        depth: usize,
        entities: &std::collections::HashMap<String, String>,
        config: &crate::reader::ParserConfig,
        version: Option<crate::common::XmlVersion>,
        lexer: &mut crate::reader::lexer::Lexer,
        value: &mut String,
    ) -> Result<()> {
        let error = |e: SyntaxError| Error::syntax(e.to_cow(), lexer.position());
        if depth > usize::from(config.max_entity_expansion_depth) {
            return Err(error(SyntaxError::EntityTooBig));
        }
        let mut rest = raw;
        while !rest.is_empty() {
            let split = rest.split_once('&');
            let (literal, tail) = split.unwrap_or((rest, ""));
            if literal.contains('<') {
                return Err(Error::syntax(
                    SyntaxError::UnexpectedOpeningTag.to_cow(),
                    lexer.position(),
                ));
            }
            if value.len().saturating_add(literal.len()) > config.max_attribute_length {
                return Err(Error::syntax(
                    SyntaxError::ExceededConfiguredLimit.to_cow(),
                    lexer.position(),
                ));
            }
            // XML attribute whitespace normalization also applies to the
            // replacement text of general entities, but not to numeric
            // references written directly in the attribute.
            let mut chars = literal.chars().peekable();
            while let Some(c) = chars.next() {
                value.push(if matches!(c, '\t' | '\n' | '\r') {
                    ' '
                } else {
                    c
                });
                if depth == 0 && c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
            }
            if split.is_none() {
                break;
            }
            let Some((name, after)) = tail.split_once(';') else {
                return Err(Error::syntax(
                    SyntaxError::EmptyEntity.to_cow(),
                    lexer.position(),
                ));
            };
            rest = after;
            let predefined = match name {
                "lt" => Some('<'),
                "gt" => Some('>'),
                "amp" => Some('&'),
                "apos" => Some('\''),
                "quot" => Some('"'),
                _ => None,
            };
            if let Some(c) = predefined {
                lexer.account_indirect(c.len_utf8(), false)?;
                value.push(c);
            } else if let Some(number) = name.strip_prefix('#') {
                let n = if let Some(hex) = number.strip_prefix('x') {
                    u32::from_str_radix(hex, 16)
                } else {
                    number.parse::<u32>()
                };
                let n = n.map_err(|_| {
                    Error::syntax(
                        SyntaxError::InvalidNumericEntity(number.into()).to_cow(),
                        lexer.position(),
                    )
                })?;
                let valid = char::from_u32(n).filter(|&c| {
                    if version == Some(crate::common::XmlVersion::Version11) {
                        crate::common::is_xml11_char(c)
                    } else {
                        crate::common::is_xml10_char(c)
                    }
                });
                let c = if let Some(c) = valid {
                    c
                } else if config.replace_unknown_entity_references {
                    '\u{fffd}'
                } else {
                    return Err(Error::syntax(
                        SyntaxError::InvalidCharacterEntity(n).to_cow(),
                        lexer.position(),
                    ));
                };
                value.push(c);
            } else if let Some(entity) = config.extra_entities.get(name) {
                lexer.account_indirect(entity.len(), true)?;
                if value.len().saturating_add(entity.len()) > config.max_attribute_length {
                    return Err(Error::syntax(
                        SyntaxError::ExceededConfiguredLimit.to_cow(),
                        lexer.position(),
                    ));
                }
                value.push_str(entity);
            } else if let Some(entity) = entities.get(name) {
                // Charge each replacement before recursively materializing it.
                // The spelling of nested references is counted again when used.
                lexer.account_indirect(entity.len(), true)?;
                if entity.len() > config.max_entity_expansion_length {
                    return Err(Error::syntax(
                        SyntaxError::EntityTooBig.to_cow(),
                        lexer.position(),
                    ));
                }
                Self::expand_attribute_part(
                    entity,
                    depth + 1,
                    entities,
                    config,
                    version,
                    lexer,
                    value,
                )?;
            } else {
                return Err(Error::syntax(
                    SyntaxError::UnexpectedEntity(name.into()).to_cow(),
                    lexer.position(),
                ));
            }
            if value.len() > config.max_attribute_length {
                return Err(Error::syntax(
                    SyntaxError::ExceededConfiguredLimit.to_cow(),
                    lexer.position(),
                ));
            }
        }
        Ok(())
    }
}
