pub use super::token::Tok;
use std::collections::HashMap;
use std::str::CharIndices;

pub struct Lexer<'input> {
    chars: CharIndices<'input>,
    at_begin_of_line: bool,
    nesting: usize, // Amount of parenthesis
    indentation_stack: Vec<usize>,
    pending: Vec<Spanned<Tok>>,
    chr0: Option<char>,
    chr1: Option<char>,
    location: Location,
}

#[derive(Debug)]
pub enum LexicalError {
    StringError,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Location {
    row: usize,
    column: usize,
}

impl Location {
    pub fn new(row: usize, column: usize) -> Self {
        Location {
            row: row,
            column: column,
        }
    }

    pub fn get_row(&self) -> usize {
        self.row
    }

    pub fn get_column(&self) -> usize {
        self.column
    }
}

pub type Spanned<Tok> = Result<(Location, Tok, Location), LexicalError>;

impl<'input> Lexer<'input> {
    pub fn new(input: &'input str) -> Self {
        let mut lxr = Lexer {
            chars: input.char_indices(),
            at_begin_of_line: true,
            nesting: 0,
            indentation_stack: vec![0],
            pending: Vec::new(),
            chr0: None,
            location: Location::new(0, 0),
            chr1: None,
        };
        lxr.next_char();
        lxr.next_char();
        // Start at top row (=1) left column (=1)
        lxr.location.row = 1;
        lxr.location.column = 1;
        lxr
    }

    // Lexer helper functions:
    fn lex_identifier(&mut self) -> Spanned<Tok> {
        let mut name = String::new();
        let start_pos = self.get_pos();
        while self.is_char() {
            name.push(self.next_char().unwrap());
        }
        let end_pos = self.get_pos();

        let mut keywords: HashMap<String, Tok> = HashMap::new();

        // Alphabetical keywords:
        keywords.insert(String::from("..."), Tok::Ellipsis);
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

        let start_pos = self.get_pos();
        while self.is_number() {
            value_text.push(self.next_char().unwrap());
        }

        // If float:
        if let Some('.') = self.chr0 {
            value_text.push(self.next_char().unwrap());
            while self.is_number() {
                value_text.push(self.next_char().unwrap());
            }
        }

        let end_pos = self.get_pos();

        let value = value_text;

        return Ok((start_pos, Tok::Number { value: value }, end_pos));
    }

    fn lex_comment(&mut self) {
        // Skip everything until end of line
        self.next_char();
        loop {
            self.next_char();
            match self.chr0 {
                Some('\n') => {
                    return;
                }
                Some('\r') => {
                    return;
                }
                Some(_) => {}
                None => return,
            }
        }
    }

    fn lex_string(&mut self) -> Spanned<Tok> {
        let quote_char = self.next_char().unwrap();
        let mut string_content = String::new();
        let start_pos = self.get_pos();

        // If the next two characters are also the quote character, then we have a triple-quoted
        // string; consume those two characters and ensure that we require a triple-quote to close
        let triple_quoted = if self.chr0 == Some(quote_char) && self.chr1 == Some(quote_char) {
            self.next_char();
            self.next_char();
            true
        } else {
            false
        };

        loop {
            match self.next_char() {
                Some('\\') => {
                    match self.next_char() {
                        Some('\\') => {
                            string_content.push('\\');
                        }
                        Some('\'') => string_content.push('\''),
                        Some('\"') => string_content.push('\"'),
                        Some('\n') => {
                            // Ignore Unix EOL character
                        }
                        Some('\r') => {
                            match self.chr0 {
                                Some('\n') => {
                                    // Ignore Windows EOL characters (2 bytes)
                                    self.next_char();
                                }
                                _ => {
                                    // Ignore Mac EOL character
                                }
                            }
                        }
                        Some('a') => string_content.push('\x07'),
                        Some('b') => string_content.push('\x08'),
                        Some('f') => string_content.push('\x0c'),
                        Some('n') => {
                            string_content.push('\n');
                        }
                        Some('r') => string_content.push('\r'),
                        Some('t') => {
                            string_content.push('\t');
                        }
                        Some('v') => string_content.push('\x0b'),
                        Some(c) => {
                            string_content.push('\\');
                            string_content.push(c);
                        }
                        None => {
                            return Err(LexicalError::StringError);
                        }
                    }
                }
                Some(c) => {
                    if c == quote_char {
                        if triple_quoted {
                            // Look ahead at the next two characters; if we have two more
                            // quote_chars, it's the end of the string; consume the remaining
                            // closing quotes and break the loop
                            if self.chr0 == Some(quote_char) && self.chr1 == Some(quote_char) {
                                self.next_char();
                                self.next_char();
                                break;
                            }
                            string_content.push(c);
                        } else {
                            break;
                        }
                    } else {
                        if c == '\n' {
                            if !triple_quoted {
                                return Err(LexicalError::StringError);
                            }
                            self.new_line();
                        }
                        string_content.push(c);
                    }
                }
                None => {
                    return Err(LexicalError::StringError);
                }
            }
        }
        let end_pos = self.get_pos();

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
            Some('a'...'z') | Some('A'...'Z') | Some('_') | Some('0'...'9') => return true,
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
        self.location.column += 1;
        c
    }

    fn get_pos(&self) -> Location {
        self.location.clone()
    }

    fn new_line(&mut self) {
        self.location.row += 1;
        self.location.column = 1;
    }

    fn inner_next(&mut self) -> Option<Spanned<Tok>> {
        if !self.pending.is_empty() {
            return Some(self.pending.remove(0));
        }

        'top_loop: loop {
            // Detect indentation levels
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
                        Some('#') => {
                            self.lex_comment();
                            self.at_begin_of_line = true;
                            continue 'top_loop;
                        }
                        Some('\r') => {
                            // Empty line!
                            self.next_char();
                            if self.chr0 == Some('\n') {
                                // absorb two bytes if Windows line ending
                                self.next_char();
                            }
                            self.at_begin_of_line = true;
                            self.new_line();
                            continue 'top_loop;
                        }
                        Some('\n') => {
                            // Empty line!
                            self.next_char();
                            self.at_begin_of_line = true;
                            self.new_line();
                            continue 'top_loop;
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
                        let tok_start = self.get_pos();
                        let tok_end = tok_start.clone();
                        return Some(Ok((tok_start, Tok::Indent, tok_end)));
                    } else if col < current_indentation {
                        // One or more dedentations
                        // Pop off other levels until col is found:

                        while col < *self.indentation_stack.last().unwrap() {
                            self.indentation_stack.pop().unwrap();
                            let tok_start = self.get_pos();
                            let tok_end = tok_start.clone();
                            self.pending.push(Ok((tok_start, Tok::Dedent, tok_end)));
                        }

                        if col != *self.indentation_stack.last().unwrap() {
                            // TODO: handle wrong indentations
                            panic!("Non matching indentation levels!");
                        }

                        return Some(self.pending.remove(0));
                    }
                }
            }

            match self.chr0 {
                Some('0'...'9') => return Some(self.lex_number()),
                Some('_') | Some('a'...'z') | Some('A'...'Z') => return Some(self.lex_identifier()),
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
                    let tok_start = self.get_pos();
                    self.next_char();
                    match self.chr0 {
                        Some('=') => {
                            self.next_char();
                            let tok_end = self.get_pos();
                            return Some(Ok((tok_start, Tok::EqEqual, tok_end)));
                        }
                        _ => {
                            let tok_end = self.get_pos();
                            return Some(Ok((tok_start, Tok::Equal, tok_end)));
                        }
                    }
                }
                Some('+') => {
                    let tok_start = self.get_pos();
                    self.next_char();
                    match self.chr0 {
                        Some('=') => {
                            self.next_char();
                            let tok_end = self.get_pos();
                            return Some(Ok((tok_start, Tok::PlusEqual, tok_end)));
                        }
                        _ => {
                            let tok_end = self.get_pos();
                            return Some(Ok((tok_start, Tok::Plus, tok_end)));
                        }
                    }
                }
                Some('*') => {
                    let tok_start = self.get_pos();
                    self.next_char();
                    match self.chr0 {
                        Some('=') => {
                            self.next_char();
                            let tok_end = self.get_pos();
                            return Some(Ok((tok_start, Tok::StarEqual, tok_end)));
                        }
                        Some('*') => {
                            self.next_char();
                            match self.chr0 {
                                Some('=') => {
                                    self.next_char();
                                    let tok_end = self.get_pos();
                                    return Some(Ok((tok_start, Tok::DoubleStarEqual, tok_end)));
                                }
                                _ => {
                                    let tok_end = self.get_pos();
                                    return Some(Ok((tok_start, Tok::DoubleStar, tok_end)));
                                }
                            }
                        }
                        _ => {
                            let tok_end = self.get_pos();
                            return Some(Ok((tok_start, Tok::Star, tok_end)));
                        }
                    }
                }
                Some('/') => {
                    let tok_start = self.get_pos();
                    self.next_char();
                    match self.chr0 {
                        Some('=') => {
                            self.next_char();
                            let tok_end = self.get_pos();
                            return Some(Ok((tok_start, Tok::SlashEqual, tok_end)));
                        }
                        Some('/') => {
                            self.next_char();
                            match self.chr0 {
                                Some('=') => {
                                    self.next_char();
                                    let tok_end = self.get_pos();
                                    return Some(Ok((tok_start, Tok::DoubleSlashEqual, tok_end)));
                                }
                                _ => {
                                    let tok_end = self.get_pos();
                                    return Some(Ok((tok_start, Tok::DoubleSlash, tok_end)));
                                }
                            }
                        }
                        _ => {
                            let tok_end = self.get_pos();
                            return Some(Ok((tok_start, Tok::Slash, tok_end)));
                        }
                    }
                }
                Some('%') => {
                    let tok_start = self.get_pos();
                    self.next_char();
                    match self.chr0 {
                        Some('=') => {
                            self.next_char();
                            let tok_end = self.get_pos();
                            return Some(Ok((tok_start, Tok::PercentEqual, tok_end)));
                        }
                        _ => {
                            let tok_end = self.get_pos();
                            return Some(Ok((tok_start, Tok::Percent, tok_end)));
                        }
                    }
                }
                Some('|') => {
                    let tok_start = self.get_pos();
                    self.next_char();
                    match self.chr0 {
                        Some('=') => {
                            self.next_char();
                            let tok_end = self.get_pos();
                            return Some(Ok((tok_start, Tok::VbarEqual, tok_end)));
                        }
                        _ => {
                            let tok_end = self.get_pos();
                            return Some(Ok((tok_start, Tok::Vbar, tok_end)));
                        }
                    }
                }
                Some('^') => {
                    let tok_start = self.get_pos();
                    self.next_char();
                    match self.chr0 {
                        Some('=') => {
                            self.next_char();
                            let tok_end = self.get_pos();
                            return Some(Ok((tok_start, Tok::CircumflexEqual, tok_end)));
                        }
                        _ => {
                            let tok_end = self.get_pos();
                            return Some(Ok((tok_start, Tok::CircumFlex, tok_end)));
                        }
                    }
                }
                Some('&') => {
                    let tok_start = self.get_pos();
                    self.next_char();
                    match self.chr0 {
                        Some('=') => {
                            self.next_char();
                            let tok_end = self.get_pos();
                            return Some(Ok((tok_start, Tok::AmperEqual, tok_end)));
                        }
                        _ => {
                            let tok_end = self.get_pos();
                            return Some(Ok((tok_start, Tok::Amper, tok_end)));
                        }
                    }
                }
                Some('-') => {
                    let tok_start = self.get_pos();
                    self.next_char();
                    match self.chr0 {
                        Some('=') => {
                            self.next_char();
                            let tok_end = self.get_pos();
                            return Some(Ok((tok_start, Tok::MinusEqual, tok_end)));
                        }
                        Some('>') => {
                            self.next_char();
                            let tok_end = self.get_pos();
                            return Some(Ok((tok_start, Tok::Rarrow, tok_end)));
                        }
                        _ => {
                            let tok_end = self.get_pos();
                            return Some(Ok((tok_start, Tok::Minus, tok_end)));
                        }
                    }
                }
                Some('@') => {
                    let tok_start = self.get_pos();
                    self.next_char();
                    match self.chr0 {
                        Some('=') => {
                            self.next_char();
                            let tok_end = self.get_pos();
                            return Some(Ok((tok_start, Tok::AtEqual, tok_end)));
                        }
                        _ => {
                            let tok_end = self.get_pos();
                            return Some(Ok((tok_start, Tok::At, tok_end)));
                        }
                    }
                }
                Some('!') => {
                    let tok_start = self.get_pos();
                    self.next_char();
                    match self.chr0 {
                        Some('=') => {
                            self.next_char();
                            let tok_end = self.get_pos();
                            return Some(Ok((tok_start, Tok::NotEqual, tok_end)));
                        }
                        _ => panic!("Invalid token '!'"),
                    }
                }
                Some('~') => {
                    return Some(self.eat_single_char(Tok::Tilde));
                }
                Some('(') => {
                    let result = self.eat_single_char(Tok::Lpar);
                    self.nesting += 1;
                    return Some(result);
                }
                Some(')') => {
                    let result = self.eat_single_char(Tok::Rpar);
                    self.nesting -= 1;
                    return Some(result);
                }
                Some('[') => {
                    let result = self.eat_single_char(Tok::Lsqb);
                    self.nesting += 1;
                    return Some(result);
                }
                Some(']') => {
                    let result = self.eat_single_char(Tok::Rsqb);
                    self.nesting -= 1;
                    return Some(result);
                }
                Some('{') => {
                    let result = self.eat_single_char(Tok::Lbrace);
                    self.nesting += 1;
                    return Some(result);
                }
                Some('}') => {
                    let result = self.eat_single_char(Tok::Rbrace);
                    self.nesting -= 1;
                    return Some(result);
                }
                Some(':') => {
                    return Some(self.eat_single_char(Tok::Colon));
                }
                Some(';') => {
                    return Some(self.eat_single_char(Tok::Semi));
                }
                Some('<') => {
                    let tok_start = self.get_pos();
                    self.next_char();
                    match self.chr0 {
                        Some('<') => {
                            self.next_char();
                            match self.chr0 {
                                Some('=') => {
                                    self.next_char();
                                    let tok_end = self.get_pos();
                                    return Some(Ok((tok_start, Tok::LeftShiftEqual, tok_end)));
                                }
                                _ => {
                                    let tok_end = self.get_pos();
                                    return Some(Ok((tok_start, Tok::LeftShift, tok_end)));
                                }
                            }
                        }
                        Some('=') => {
                            self.next_char();
                            let tok_end = self.get_pos();
                            return Some(Ok((tok_start, Tok::LessEqual, tok_end)));
                        }
                        _ => {
                            let tok_end = self.get_pos();
                            return Some(Ok((tok_start, Tok::Less, tok_end)));
                        }
                    }
                }
                Some('>') => {
                    let tok_start = self.get_pos();
                    self.next_char();
                    match self.chr0 {
                        Some('>') => {
                            self.next_char();
                            match self.chr0 {
                                Some('=') => {
                                    self.next_char();
                                    let tok_end = self.get_pos();
                                    return Some(Ok((tok_start, Tok::RightShiftEqual, tok_end)));
                                }
                                _ => {
                                    let tok_end = self.get_pos();
                                    return Some(Ok((tok_start, Tok::RightShift, tok_end)));
                                }
                            }
                        }
                        Some('=') => {
                            self.next_char();
                            let tok_end = self.get_pos();
                            return Some(Ok((tok_start, Tok::GreaterEqual, tok_end)));
                        }
                        _ => {
                            let tok_end = self.get_pos();
                            return Some(Ok((tok_start, Tok::Greater, tok_end)));
                        }
                    }
                }
                Some(',') => {
                    let tok_start = self.get_pos();
                    self.next_char();
                    let tok_end = self.get_pos();
                    return Some(Ok((tok_start, Tok::Comma, tok_end)));
                }
                Some('.') => {
                    let tok_start = self.get_pos();
                    self.next_char();
                    let tok_end = self.get_pos();
                    return Some(Ok((tok_start, Tok::Dot, tok_end)));
                }
                Some('\r') => {
                    let tok_start = self.get_pos();
                    self.next_char();
                    let tok_end = self.get_pos();
                    self.new_line();

                    // Depending on the nesting level, we emit newline or not:
                    if self.nesting == 0 {
                        self.at_begin_of_line = true;
                        return Some(Ok((tok_start, Tok::Newline, tok_end)));
                    } else {
                        continue;
                    }
                }
                Some('\n') => {
                    let tok_start = self.get_pos();
                    self.next_char();
                    let tok_end = self.get_pos();
                    self.new_line();

                    // Depending on the nesting level, we emit newline or not:
                    if self.nesting == 0 {
                        self.at_begin_of_line = true;
                        return Some(Ok((tok_start, Tok::Newline, tok_end)));
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

    fn eat_single_char(&mut self, ty: Tok) -> Spanned<Tok> {
        let tok_start = self.get_pos();
        self.next_char();
        let tok_end = self.get_pos();
        Ok((tok_start, ty, tok_end))
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
        let token = self.inner_next();
        trace!(
            "Lex token {:?}, nesting={:?}, indent stack: {:?}",
            token,
            self.nesting,
            self.indentation_stack
        );
        token
    }
}

#[cfg(test)]
mod tests {
    use super::{Lexer, Tok};
    use std::iter::FromIterator;

    const WINDOWS_EOL: &str = "\r\n";
    const MAC_EOL: &str = "\r";
    const UNIX_EOL: &str = "\n";

    pub fn lex_source(source: &String) -> Vec<Tok> {
        let lexer = Lexer::new(source);
        Vec::from_iter(lexer.map(|x| x.unwrap().1))
    }

    macro_rules! test_line_comment {
        ($($name:ident: $eol:expr,)*) => {
            $(
            #[test]
            fn $name() {
                let source = String::from(format!(r"99232  # {}", $eol));
                let tokens = lex_source(&source);
                assert_eq!(tokens, vec![Tok::Number { value: "99232".to_string() }]);
            }
            )*
        }
    }

    test_line_comment! {
        test_line_comment_long: " foo",
        test_line_comment_whitespace: "  ",
        test_line_comment_single_whitespace: " ",
        test_line_comment_empty: "",
    }

    macro_rules! test_comment_until_eol {
        ($($name:ident: $eol:expr,)*) => {
            $(
            #[test]
            fn $name() {
                let source = String::from(format!("123  # Foo{}456", $eol));
                let tokens = lex_source(&source);
                assert_eq!(
                    tokens,
                    vec![
                        Tok::Number { value: "123".to_string() },
                        Tok::Newline,
                        Tok::Number { value: "456".to_string() },
                    ]
                )
            }
            )*
        }
    }

    test_comment_until_eol! {
        test_comment_until_windows_eol: WINDOWS_EOL,
        test_comment_until_mac_eol: MAC_EOL,
        test_comment_until_unix_eol: UNIX_EOL,
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
                Tok::Number {
                    value: "99".to_string()
                },
                Tok::Plus,
                Tok::Number {
                    value: "2".to_string()
                },
                Tok::Minus,
                Tok::Number {
                    value: "0".to_string()
                },
            ]
        );
    }

    macro_rules! test_indentation_with_eol {
        ($($name:ident: $eol:expr,)*) => {
            $(
            #[test]
            fn $name() {
                let source = String::from(format!("def foo():{}   return 99{}{}", $eol, $eol, $eol));
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
                        Tok::Number { value: "99".to_string() },
                        Tok::Newline,
                        Tok::Dedent,
                    ]
                );
            }
            )*
        };
    }

    test_indentation_with_eol! {
        test_indentation_windows_eol: WINDOWS_EOL,
        test_indentation_mac_eol: MAC_EOL,
        test_indentation_unix_eol: UNIX_EOL,
    }

    macro_rules! test_double_dedent_with_eol {
        ($($name:ident: $eol:expr,)*) => {
        $(
            #[test]
            fn $name() {
                let source = String::from(format!("def foo():{} if x:{}{}  return 99{}{}", $eol, $eol, $eol, $eol, $eol));
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
                        Tok::If,
                        Tok::Name {
                            name: String::from("x"),
                        },
                        Tok::Colon,
                        Tok::Newline,
                        Tok::Indent,
                        Tok::Return,
                        Tok::Number { value: "99".to_string() },
                        Tok::Newline,
                        Tok::Dedent,
                        Tok::Dedent,
                    ]
                );
            }
        )*
        }
    }

    test_double_dedent_with_eol! {
        test_double_dedent_windows_eol: WINDOWS_EOL,
        test_double_dedent_mac_eol: MAC_EOL,
        test_double_dedent_unix_eol: UNIX_EOL,
    }

    macro_rules! test_newline_in_brackets {
        ($($name:ident: $eol:expr,)*) => {
        $(
            #[test]
            fn $name() {
                let source = String::from(format!("x = [{}    1,2{}]{}", $eol, $eol, $eol));
                let tokens = lex_source(&source);
                assert_eq!(
                    tokens,
                    vec![
                        Tok::Name {
                            name: String::from("x"),
                        },
                        Tok::Equal,
                        Tok::Lsqb,
                        Tok::Number { value: "1".to_string() },
                        Tok::Comma,
                        Tok::Number { value: "2".to_string() },
                        Tok::Rsqb,
                        Tok::Newline,
                    ]
                );
            }
        )*
        };
    }

    test_newline_in_brackets! {
        test_newline_in_brackets_windows_eol: WINDOWS_EOL,
        test_newline_in_brackets_mac_eol: MAC_EOL,
        test_newline_in_brackets_unix_eol: UNIX_EOL,
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

    #[test]
    fn test_string() {
        let source = String::from(r#""double" 'single' 'can\'t' "\\\"" '\t\r\n' '\g'"#);
        let tokens = lex_source(&source);
        assert_eq!(
            tokens,
            vec![
                Tok::String {
                    value: String::from("double"),
                },
                Tok::String {
                    value: String::from("single"),
                },
                Tok::String {
                    value: String::from("can't"),
                },
                Tok::String {
                    value: String::from("\\\""),
                },
                Tok::String {
                    value: String::from("\t\r\n"),
                },
                Tok::String {
                    value: String::from("\\g"),
                },
            ]
        );
    }

    macro_rules! test_string_continuation {
        ($($name:ident: $eol:expr,)*) => {
        $(
            #[test]
            fn $name() {
                let source = String::from(format!("\"abc\\{}def\"", $eol));
                let tokens = lex_source(&source);
                assert_eq!(
                    tokens,
                    vec![
                        Tok::String {
                            value: String::from("abcdef"),
                        },
                    ]
                )
            }
        )*
        }
    }

    test_string_continuation! {
        test_string_continuation_windows_eol: WINDOWS_EOL,
        test_string_continuation_mac_eol: MAC_EOL,
        test_string_continuation_unix_eol: UNIX_EOL,
    }
}
