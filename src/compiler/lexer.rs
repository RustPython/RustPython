use std::str::FromStr;
use std::str::CharIndices;
pub use super::token::Tok;
use std::iter::FromIterator;
use std::collections::HashMap;

pub struct Lexer<'input> {
    chars: CharIndices<'input>,
    at_begin_of_line: bool,
    nesting: usize, // Amount of parenthesis
    indentation_stack: Vec<usize>,
    chr0: Option<char>,
    chr1: Option<char>,
    location: usize,
}

#[derive(Debug)]
pub enum LexicalError {
    StringError,
}

pub type Spanned<Tok> = Result<(usize, Tok, usize), LexicalError>;

pub fn lex_source(source: &String) -> Vec<Tok> {
    let lexer = Lexer::new(source);
    Vec::from_iter(lexer.map(|x| x.unwrap().1))
}

impl<'input> Lexer<'input> {
    pub fn new(input: &'input str) -> Self {
        let mut lxr = Lexer {
            chars: input.char_indices(),
            at_begin_of_line: true,
            nesting: 0,
            indentation_stack: vec![0],
            chr0: None,
            location: 0,
            chr1: None,
        };
        lxr.next_char();
        lxr.next_char();
        lxr
    }

    // Lexer helper functions:
    fn lex_identifier(&mut self) -> Spanned<Tok> {
        let mut name = String::new();
        let start_pos = self.location;
        while self.is_char() {
            name.push(self.next_char().unwrap());
        }
        let end_pos = self.location;

        let mut keywords: HashMap<String, Tok> = HashMap::new();

        // Alphabetical keywords:
        keywords.insert(String::from("False"), Tok::False);
        keywords.insert(String::from("None"), Tok::None);
        keywords.insert(String::from("True"), Tok::True);

        keywords.insert(String::from("and"), Tok::And);
        keywords.insert(String::from("as"), Tok::As);
        keywords.insert(String::from("assert"), Tok::Assert);
        keywords.insert(String::from("break"), Tok::Break);
        keywords.insert(String::from("class"), Tok::Class);
        keywords.insert(String::from("continue"), Tok::Continue);
        keywords.insert(String::from("def"), Tok::Def);
        keywords.insert(String::from("del"), Tok::Del);
        keywords.insert(String::from("elif"), Tok::Elif);
        keywords.insert(String::from("else"), Tok::Else);
        keywords.insert(String::from("except"), Tok::Except);
        keywords.insert(String::from("finally"), Tok::Finally);
        keywords.insert(String::from("for"), Tok::For);
        keywords.insert(String::from("from"), Tok::From);
        keywords.insert(String::from("global"), Tok::Global);
        keywords.insert(String::from("if"), Tok::If);
        keywords.insert(String::from("import"), Tok::Import);
        keywords.insert(String::from("in"), Tok::In);
        keywords.insert(String::from("is"), Tok::Is);
        keywords.insert(String::from("lambda"), Tok::Lambda);
        keywords.insert(String::from("nonlocal"), Tok::Nonlocal);
        keywords.insert(String::from("not"), Tok::Not);
        keywords.insert(String::from("or"), Tok::Or);
        keywords.insert(String::from("pass"), Tok::Pass);
        keywords.insert(String::from("raise"), Tok::Raise);
        keywords.insert(String::from("return"), Tok::Return);
        keywords.insert(String::from("try"), Tok::Try);
        keywords.insert(String::from("while"), Tok::While);
        keywords.insert(String::from("with"), Tok::With);
        keywords.insert(String::from("yield"), Tok::Yield);

        if keywords.contains_key(&name) {
            Ok((start_pos, keywords.remove(&name).unwrap(), end_pos))
        } else {
            Ok((start_pos, Tok::Name { name: name }, end_pos))
        }
    }

    fn lex_number(&mut self) -> Spanned<Tok> {
        let mut value_text = String::new();

        let start_pos = self.location;
        while self.is_number() {
            value_text.push(self.next_char().unwrap());
        }
        let end_pos = self.location;

        let value = i32::from_str(&value_text).unwrap();

        return Ok((start_pos, Tok::Number { value: value }, end_pos));
    }

    fn lex_comment(&mut self) {
        // Skip everything until end of line
        self.next_char();
        loop {
            match self.next_char() {
                Some(c) => {
                    if c == '\n' {
                        return;
                    }
                }
                None => return,
            }
        }
    }

    fn lex_string(&mut self) -> Spanned<Tok> {
        let quote_char = self.next_char().unwrap();
        let mut string_content = String::new();
        let start_pos = self.location;

        loop {
            match self.next_char() {
                Some(c) => {
                    if c == quote_char {
                        break;
                    } else {
                        string_content.push(c);
                    }
                }
                None => {
                    return Err(LexicalError::StringError);
                }
            }
        }
        let end_pos = self.location;

        return Ok((
            start_pos,
            Tok::String {
                value: string_content,
            },
            end_pos,
        ));
    }

    fn is_char(&self) -> bool {
        match self.chr0 {
            Some('a'...'z') => return true,
            _ => return false,
        }
    }

    fn is_number(&self) -> bool {
        match self.chr0 {
            Some('0'...'9') => return true,
            _ => return false,
        }
    }

    fn next_char(&mut self) -> Option<char> {
        let c = self.chr0;
        let nxt = self.chars.next();
        self.chr0 = self.chr1;
        self.chr1 = nxt.map(|x| x.1);
        self.location = match nxt {
            Some(p) => p.0,
            None => 99999,
        };
        c
    }
}

/* Implement iterator pattern for the get_tok function.

Calling the next element in the iterator will yield the next lexical
token.
*/
impl<'input> Iterator for Lexer<'input> {
    type Item = Spanned<Tok>;

    fn next(&mut self) -> Option<Self::Item> {
        // Idea: create some sort of hash map for single char tokens:
        // let mut X = HashMap::new();
        // X.insert('=', Tok::Equal);

        // Detect indentation levels
        loop {
            if self.at_begin_of_line {
                self.at_begin_of_line = false;

                // Determine indentation:
                let mut col: usize = 0;
                loop {
                    match self.chr0 {
                        Some(' ') => {
                            self.next_char();
                            col += 1;
                        }
                        _ => {
                            break;
                        }
                    }
                }

                if self.nesting == 0 {
                    // Determine indent or dedent:
                    let current_indentation = *self.indentation_stack.last().unwrap();
                    if col == current_indentation {
                        // Same same
                    } else if col > current_indentation {
                        // New indentation level:
                        self.indentation_stack.push(col);
                        return Some(Ok((0, Tok::Indent, 0)));
                    } else if col < current_indentation {
                        // One or more dedentations
                        return Some(Ok((0, Tok::Dedent, 0)));
                    }
                }
            }

            match self.chr0 {
                Some('0'...'9') => return Some(self.lex_number()),
                // TODO: 'A'...'Z'
                Some('a'...'z') => return Some(self.lex_identifier()),
                Some('#') => {
                    self.lex_comment();
                    continue;
                }
                Some('"') => {
                    return Some(self.lex_string());
                }
                Some('\'') => {
                    return Some(self.lex_string());
                }
                Some('=') => {
                    self.next_char();
                    match self.chr0 {
                        Some('=') => {
                            self.next_char();
                            return Some(Ok((self.location, Tok::EqEqual, self.location + 1)));
                        }
                        _ => return Some(Ok((self.location, Tok::Equal, self.location + 1))),
                    }
                }
                Some('+') => {
                    self.next_char();
                    match self.chr0 {
                        Some('=') => {
                            self.next_char();
                            return Some(Ok((self.location, Tok::PlusEqual, self.location + 1)));
                        }
                        _ => return Some(Ok((self.location, Tok::Plus, self.location + 1))),
                    }
                }
                Some('*') => {
                    let tok_start = self.location;
                    self.next_char();
                    match self.chr0 {
                        Some('=') => {
                            self.next_char();
                            return Some(Ok((tok_start, Tok::StarEqual, self.location + 1)));
                        }
                        Some('*') => {
                            self.next_char();
                            return Some(Ok((tok_start, Tok::DoubleStar, self.location + 1)));
                        }
                        _ => return Some(Ok((tok_start, Tok::Star, self.location + 1))),
                    }
                }
                Some('/') => {
                    let tok_start = self.location;
                    self.next_char();
                    match self.chr0 {
                        Some('=') => {
                            self.next_char();
                            return Some(Ok((tok_start, Tok::SlashEqual, self.location + 1)));
                        }
                        Some('/') => {
                            self.next_char();
                            match self.chr0 {
                                Some('=') => {
                                    self.next_char();
                                    return Some(Ok((
                                        tok_start,
                                        Tok::DoubleSlashEqual,
                                        self.location + 1,
                                    )));
                                }
                                _ => {
                                    return Some(Ok((
                                        tok_start,
                                        Tok::DoubleSlash,
                                        self.location + 1,
                                    )))
                                }
                            }
                        }
                        _ => return Some(Ok((tok_start, Tok::Slash, self.location + 1))),
                    }
                }
                Some('%') => {
                    self.next_char();
                    match self.chr0 {
                        Some('=') => {
                            self.next_char();
                            return Some(Ok((self.location, Tok::PercentEqual, self.location + 1)));
                        }
                        _ => return Some(Ok((self.location, Tok::Percent, self.location + 1))),
                    }
                }
                Some('|') => {
                    self.next_char();
                    match self.chr0 {
                        Some('=') => {
                            self.next_char();
                            return Some(Ok((self.location, Tok::VbarEqual, self.location + 1)));
                        }
                        _ => return Some(Ok((self.location, Tok::Vbar, self.location + 1))),
                    }
                }
                Some('^') => {
                    self.next_char();
                    match self.chr0 {
                        Some('=') => {
                            self.next_char();
                            return Some(Ok((
                                self.location,
                                Tok::CircumflexEqual,
                                self.location + 1,
                            )));
                        }
                        _ => return Some(Ok((self.location, Tok::CircumFlex, self.location + 1))),
                    }
                }
                Some('&') => {
                    self.next_char();
                    match self.chr0 {
                        Some('=') => {
                            self.next_char();
                            return Some(Ok((self.location, Tok::AmperEqual, self.location + 1)));
                        }
                        _ => return Some(Ok((self.location, Tok::Amper, self.location + 1))),
                    }
                }
                Some('-') => {
                    let tok_start = self.location;
                    self.next_char();
                    match self.chr0 {
                        Some('=') => {
                            self.next_char();
                            return Some(Ok((tok_start, Tok::MinusEqual, self.location + 1)));
                        }
                        _ => return Some(Ok((tok_start, Tok::Minus, self.location + 1))),
                    }
                }
                Some('@') => {
                    let tok_start = self.location;
                    self.next_char();
                    match self.chr0 {
                        Some('=') => {
                            self.next_char();
                            return Some(Ok((tok_start, Tok::AtEqual, self.location + 1)));
                        }
                        _ => return Some(Ok((tok_start, Tok::At, self.location + 1))),
                    }
                }
                Some('~') => {
                    self.next_char();
                    return Some(Ok((0, Tok::Tilde, 0)));
                }
                Some('(') => {
                    self.next_char();
                    self.nesting += 1;
                    return Some(Ok((0, Tok::Lpar, 0)));
                }
                Some(')') => {
                    self.next_char();
                    self.nesting -= 1;
                    return Some(Ok((0, Tok::Rpar, 0)));
                }
                Some('[') => {
                    self.next_char();
                    self.nesting += 1;
                    return Some(Ok((0, Tok::Lsqb, 0)));
                }
                Some(']') => {
                    self.next_char();
                    self.nesting -= 1;
                    return Some(Ok((self.location, Tok::Rsqb, self.location + 1)));
                }
                Some('{') => {
                    self.next_char();
                    self.nesting += 1;
                    return Some(Ok((0, Tok::Lbrace, 0)));
                }
                Some('}') => {
                    self.next_char();
                    self.nesting -= 1;
                    return Some(Ok((self.location, Tok::Rbrace, self.location + 1)));
                }
                Some(':') => {
                    self.next_char();
                    return Some(Ok((self.location, Tok::Colon, self.location + 1)));
                }
                Some(';') => {
                    self.next_char();
                    return Some(Ok((self.location, Tok::Semi, self.location + 1)));
                }
                Some('<') => {
                    let tok_start = self.location;
                    self.next_char();
                    match self.chr0 {
                        Some('<') => {
                            self.next_char();
                            match self.chr0 {
                                Some('=') => {
                                    return Some(Ok((
                                        tok_start,
                                        Tok::LeftShiftEqual,
                                        self.location + 1,
                                    )))
                                }
                                _ => {
                                    return Some(Ok((tok_start, Tok::LeftShift, self.location + 1)))
                                }
                            }
                        }
                        Some('=') => {
                            self.next_char();
                            return Some(Ok((tok_start, Tok::LessEqual, self.location + 1)));
                        }
                        _ => return Some(Ok((tok_start, Tok::Less, self.location + 1))),
                    }
                }
                Some('>') => {
                    let tok_start = self.location;
                    self.next_char();
                    match self.chr0 {
                        Some('>') => {
                            self.next_char();
                            match self.chr0 {
                                Some('=') => {
                                    return Some(Ok((
                                        tok_start,
                                        Tok::RightShiftEqual,
                                        self.location + 1,
                                    )))
                                }
                                _ => {
                                    return Some(Ok((tok_start, Tok::RightShift, self.location + 1)))
                                }
                            }
                        }
                        Some('=') => {
                            self.next_char();
                            return Some(Ok((tok_start, Tok::GreaterEqual, self.location + 1)));
                        }
                        _ => return Some(Ok((tok_start, Tok::Greater, self.location + 1))),
                    }
                }
                Some(',') => {
                    self.next_char();
                    return Some(Ok((self.location, Tok::Comma, self.location + 1)));
                }
                Some('.') => {
                    self.next_char();
                    return Some(Ok((self.location, Tok::Dot, self.location + 1)));
                }
                Some('\n') => {
                    self.next_char();

                    // Depending on the nesting level, we emit newline or not:
                    if self.nesting == 0 {
                        self.at_begin_of_line = true;
                        return Some(Ok((self.location, Tok::Newline, self.location + 1)));
                    } else {
                        continue;
                    }
                }
                Some(' ') => {
                    // Skip whitespaces
                    self.next_char();
                    continue;
                }
                None => return None,
                _ => {
                    let c = self.next_char();
                    panic!("Not impl {:?}", c)
                } // Ignore all the rest..
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Tok;
    use super::lex_source;

    #[test]
    fn test_line_comment() {
        let source = String::from(r"99232  # Bladibla");
        let tokens = lex_source(&source);
        assert_eq!(tokens, vec![Tok::Number { value: 99232 }]);
    }

    #[test]
    fn test_assignment() {
        let source = String::from(r"avariable = 99 + 2-0");
        let tokens = lex_source(&source);
        assert_eq!(
            tokens,
            vec![
                Tok::Name {
                    name: String::from("avariable"),
                },
                Tok::Equal,
                Tok::Number { value: 99 },
                Tok::Plus,
                Tok::Number { value: 2 },
                Tok::Minus,
                Tok::Number { value: 0 },
            ]
        );
    }

    #[test]
    fn test_indentation() {
        let source = String::from("def foo():\n   return 99\n");
        let tokens = lex_source(&source);
        assert_eq!(
            tokens,
            vec![
                Tok::Def,
                Tok::Name {
                    name: String::from("foo"),
                },
                Tok::Lpar,
                Tok::Rpar,
                Tok::Colon,
                Tok::Newline,
                Tok::Indent,
                Tok::Return,
                Tok::Number { value: 99 },
                Tok::Newline,
                Tok::Dedent,
            ]
        );
    }

    #[test]
    fn test_newline_in_brackets() {
        let source = String::from("x = [\n    1,2\n]\n");
        let tokens = lex_source(&source);
        assert_eq!(
            tokens,
            vec![
                Tok::Name {
                    name: String::from("x"),
                },
                Tok::Equal,
                Tok::Lsqb,
                Tok::Number { value: 1 },
                Tok::Comma,
                Tok::Number { value: 2 },
                Tok::Rsqb,
                Tok::Newline,
            ]
        );
    }

    #[test]
    fn test_operators() {
        let source = String::from("//////=/ /");
        let tokens = lex_source(&source);
        assert_eq!(
            tokens,
            vec![
                Tok::DoubleSlash,
                Tok::DoubleSlash,
                Tok::DoubleSlashEqual,
                Tok::Slash,
                Tok::Slash,
            ]
        );
    }

}
