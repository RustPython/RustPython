//! This module takes care of lexing python source text. This means source
//! code is translated into separate tokens.

extern crate unic_emoji_char;
extern crate unicode_xid;

pub use super::token::Tok;
use num_bigint::BigInt;
use num_traits::Num;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::str::FromStr;
use unic_emoji_char::is_emoji_presentation;
use unicode_xid::UnicodeXID;

#[derive(Clone, Copy, PartialEq, Debug)]
struct IndentationLevel {
    tabs: usize,
    spaces: usize,
}

impl IndentationLevel {
    fn new() -> IndentationLevel {
        IndentationLevel { tabs: 0, spaces: 0 }
    }
    fn compare_strict(&self, other: &IndentationLevel) -> Option<Ordering> {
        // We only know for sure that we're smaller or bigger if tabs
        // and spaces both differ in the same direction. Otherwise we're
        // dependent on the size of tabs.
        if self.tabs < other.tabs {
            if self.spaces <= other.spaces {
                Some(Ordering::Less)
            } else {
                None
            }
        } else if self.tabs > other.tabs {
            if self.spaces >= other.spaces {
                Some(Ordering::Greater)
            } else {
                None
            }
        } else {
            Some(self.spaces.cmp(&other.spaces))
        }
    }
}

pub struct Lexer<T: Iterator<Item = char>> {
    chars: T,
    at_begin_of_line: bool,
    nesting: usize, // Amount of parenthesis
    indentation_stack: Vec<IndentationLevel>,
    pending: Vec<LexResult>,
    chr0: Option<char>,
    chr1: Option<char>,
    location: Location,
    keywords: HashMap<String, Tok>,
}

#[derive(Debug)]
pub struct LexicalError {
    pub error: LexicalErrorType,
    pub location: Location,
}

#[derive(Debug)]
pub enum LexicalErrorType {
    StringError,
    NestingError,
    UnrecognizedToken { tok: char },
    OtherError(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Location {
    row: usize,
    column: usize,
}

impl Location {
    pub fn new(row: usize, column: usize) -> Self {
        Location { row, column }
    }

    pub fn get_row(&self) -> usize {
        self.row
    }

    pub fn get_column(&self) -> usize {
        self.column
    }
}

pub fn get_keywords() -> HashMap<String, Tok> {
    let mut keywords: HashMap<String, Tok> = HashMap::new();

    // Alphabetical keywords:
    keywords.insert(String::from("..."), Tok::Ellipsis);
    keywords.insert(String::from("False"), Tok::False);
    keywords.insert(String::from("None"), Tok::None);
    keywords.insert(String::from("True"), Tok::True);

    keywords.insert(String::from("and"), Tok::And);
    keywords.insert(String::from("as"), Tok::As);
    keywords.insert(String::from("assert"), Tok::Assert);
    keywords.insert(String::from("async"), Tok::Async);
    keywords.insert(String::from("await"), Tok::Await);
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
    keywords
}

pub type Spanned = (Location, Tok, Location);
pub type LexResult = Result<Spanned, LexicalError>;

pub fn make_tokenizer<'a>(source: &'a str) -> impl Iterator<Item = LexResult> + 'a {
    let nlh = NewlineHandler::new(source.chars());
    let lch = LineContinationHandler::new(nlh);
    Lexer::new(lch)
}

// The newline handler is an iterator which collapses different newline
// types into \n always.
pub struct NewlineHandler<T: Iterator<Item = char>> {
    source: T,
    chr0: Option<char>,
    chr1: Option<char>,
}

impl<T> NewlineHandler<T>
where
    T: Iterator<Item = char>,
{
    pub fn new(source: T) -> Self {
        let mut nlh = NewlineHandler {
            source,
            chr0: None,
            chr1: None,
        };
        nlh.shift();
        nlh.shift();
        nlh
    }

    fn shift(&mut self) -> Option<char> {
        let result = self.chr0;
        self.chr0 = self.chr1;
        self.chr1 = self.source.next();
        result
    }
}

impl<T> Iterator for NewlineHandler<T>
where
    T: Iterator<Item = char>,
{
    type Item = char;

    fn next(&mut self) -> Option<Self::Item> {
        // Collapse \r\n into \n
        loop {
            if self.chr0 == Some('\r') {
                if self.chr1 == Some('\n') {
                    // Transform windows EOL into \n
                    self.shift();
                } else {
                    // Transform MAC EOL into \n
                    self.chr0 = Some('\n')
                }
            } else {
                break;
            }
        }

        self.shift()
    }
}

// Glues \ and \n into a single line:
pub struct LineContinationHandler<T: Iterator<Item = char>> {
    source: T,
    chr0: Option<char>,
    chr1: Option<char>,
}

impl<T> LineContinationHandler<T>
where
    T: Iterator<Item = char>,
{
    pub fn new(source: T) -> Self {
        let mut nlh = LineContinationHandler {
            source,
            chr0: None,
            chr1: None,
        };
        nlh.shift();
        nlh.shift();
        nlh
    }

    fn shift(&mut self) -> Option<char> {
        let result = self.chr0;
        self.chr0 = self.chr1;
        self.chr1 = self.source.next();
        result
    }
}

impl<T> Iterator for LineContinationHandler<T>
where
    T: Iterator<Item = char>,
{
    type Item = char;

    fn next(&mut self) -> Option<Self::Item> {
        // Collapse \r\n into \n
        loop {
            if self.chr0 == Some('\\') && self.chr1 == Some('\n') {
                // Skip backslash and newline
                self.shift();
                self.shift();
            // Idea: insert trailing newline here:
            // } else if self.chr0 != Some('\n') && self.chr1.is_none() {
            //     self.chr1 = Some('\n');
            } else {
                break;
            }
        }

        self.shift()
    }
}

impl<T> Lexer<T>
where
    T: Iterator<Item = char>,
{
    pub fn new(input: T) -> Self {
        let mut lxr = Lexer {
            chars: input,
            at_begin_of_line: true,
            nesting: 0,
            indentation_stack: vec![IndentationLevel::new()],
            pending: Vec::new(),
            chr0: None,
            location: Location::new(0, 0),
            chr1: None,
            keywords: get_keywords(),
        };
        lxr.next_char();
        lxr.next_char();
        // Start at top row (=1) left column (=1)
        lxr.location.row = 1;
        lxr.location.column = 1;
        lxr
    }

    // Lexer helper functions:
    fn lex_identifier(&mut self) -> LexResult {
        let mut name = String::new();
        let start_pos = self.get_pos();

        // Detect potential string like rb'' b'' f'' u'' r''
        let mut saw_b = false;
        let mut saw_r = false;
        let mut saw_u = false;
        let mut saw_f = false;
        loop {
            // Detect r"", f"", b"" and u""
            if !(saw_b || saw_u || saw_f) && (self.chr0 == Some('b') || self.chr0 == Some('B')) {
                saw_b = true;
            } else if !(saw_b || saw_r || saw_u || saw_f)
                && (self.chr0 == Some('u') || self.chr0 == Some('U'))
            {
                saw_u = true;
            } else if !(saw_r || saw_u) && (self.chr0 == Some('r') || self.chr0 == Some('R')) {
                saw_r = true;
            } else if !(saw_b || saw_u || saw_f)
                && (self.chr0 == Some('f') || self.chr0 == Some('F'))
            {
                saw_f = true;
            } else {
                break;
            }

            // Take up char into name:
            name.push(self.next_char().unwrap());

            // Check if we have a string:
            if self.chr0 == Some('"') || self.chr0 == Some('\'') {
                return self.lex_string(saw_b, saw_r, saw_u, saw_f);
            }
        }

        while self.is_identifier_continuation() {
            name.push(self.next_char().unwrap());
        }
        let end_pos = self.get_pos();

        if self.keywords.contains_key(&name) {
            Ok((start_pos, self.keywords[&name].clone(), end_pos))
        } else {
            Ok((start_pos, Tok::Name { name }, end_pos))
        }
    }

    fn lex_number(&mut self) -> LexResult {
        let start_pos = self.get_pos();
        if self.chr0 == Some('0') {
            if self.chr1 == Some('x') || self.chr1 == Some('X') {
                // Hex!
                self.next_char();
                self.next_char();
                self.lex_number_radix(start_pos, 16)
            } else if self.chr1 == Some('o') || self.chr1 == Some('O') {
                // Octal style!
                self.next_char();
                self.next_char();
                self.lex_number_radix(start_pos, 8)
            } else if self.chr1 == Some('b') || self.chr1 == Some('B') {
                // Binary!
                self.next_char();
                self.next_char();
                self.lex_number_radix(start_pos, 2)
            } else {
                self.lex_normal_number()
            }
        } else {
            self.lex_normal_number()
        }
    }

    fn lex_number_radix(&mut self, start_pos: Location, radix: u32) -> LexResult {
        let mut value_text = String::new();

        loop {
            if let Some(c) = self.take_number(radix) {
                value_text.push(c);
            } else if self.chr0 == Some('_') {
                self.next_char();
            } else {
                break;
            }
        }

        let end_pos = self.get_pos();
        let value = BigInt::from_str_radix(&value_text, radix).map_err(|e| LexicalError {
            error: LexicalErrorType::OtherError(format!("{:?}", e)),
            location: start_pos.clone(),
        })?;
        Ok((start_pos, Tok::Int { value }, end_pos))
    }

    fn lex_normal_number(&mut self) -> LexResult {
        let start_pos = self.get_pos();

        let mut value_text = String::new();

        // Normal number:
        while let Some(c) = self.take_number(10) {
            value_text.push(c);
        }

        // If float:
        if self.chr0 == Some('.') || self.chr0 == Some('e') {
            // Take '.':
            if self.chr0 == Some('.') {
                value_text.push(self.next_char().unwrap());
                while let Some(c) = self.take_number(10) {
                    value_text.push(c);
                }
            }

            // 1e6 for example:
            if self.chr0 == Some('e') {
                value_text.push(self.next_char().unwrap());

                // Optional +/-
                if self.chr0 == Some('-') || self.chr0 == Some('+') {
                    value_text.push(self.next_char().unwrap());
                }

                while let Some(c) = self.take_number(10) {
                    value_text.push(c);
                }
            }

            let value = f64::from_str(&value_text).unwrap();
            // Parse trailing 'j':
            if self.chr0 == Some('j') {
                self.next_char();
                let end_pos = self.get_pos();
                Ok((
                    start_pos,
                    Tok::Complex {
                        real: 0.0,
                        imag: value,
                    },
                    end_pos,
                ))
            } else {
                let end_pos = self.get_pos();
                Ok((start_pos, Tok::Float { value }, end_pos))
            }
        } else {
            // Parse trailing 'j':
            if self.chr0 == Some('j') {
                self.next_char();
                let end_pos = self.get_pos();
                let imag = f64::from_str(&value_text).unwrap();
                Ok((start_pos, Tok::Complex { real: 0.0, imag }, end_pos))
            } else {
                let end_pos = self.get_pos();
                let value = value_text.parse::<BigInt>().unwrap();
                Ok((start_pos, Tok::Int { value }, end_pos))
            }
        }
    }

    fn lex_comment(&mut self) {
        // Skip everything until end of line
        self.next_char();
        loop {
            match self.chr0 {
                Some('\n') => return,
                Some(_) => {}
                None => return,
            }
            self.next_char();
        }
    }

    fn lex_string(
        &mut self,
        is_bytes: bool,
        is_raw: bool,
        _is_unicode: bool,
        is_fstring: bool,
    ) -> LexResult {
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
                    if self.chr0 == Some(quote_char) {
                        string_content.push(quote_char);
                        self.next_char();
                    } else if is_raw {
                        string_content.push('\\');
                        if let Some(c) = self.next_char() {
                            string_content.push(c)
                        } else {
                            return Err(LexicalError {
                                error: LexicalErrorType::StringError,
                                location: self.get_pos(),
                            });
                        }
                    } else {
                        match self.next_char() {
                            Some('\\') => {
                                string_content.push('\\');
                            }
                            Some('\'') => string_content.push('\''),
                            Some('\"') => string_content.push('\"'),
                            Some('\n') => {
                                // Ignore Unix EOL character
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
                                return Err(LexicalError {
                                    error: LexicalErrorType::StringError,
                                    location: self.get_pos(),
                                });
                            }
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
                                return Err(LexicalError {
                                    error: LexicalErrorType::StringError,
                                    location: self.get_pos(),
                                });
                            }
                            self.new_line();
                        }
                        string_content.push(c);
                    }
                }
                None => {
                    return Err(LexicalError {
                        error: LexicalErrorType::StringError,
                        location: self.get_pos(),
                    });
                }
            }
        }
        let end_pos = self.get_pos();

        let tok = if is_bytes {
            if string_content.is_ascii() {
                Tok::Bytes {
                    value: lex_byte(string_content)?,
                }
            } else {
                return Err(LexicalError {
                    error: LexicalErrorType::StringError,
                    location: self.get_pos(),
                });
            }
        } else {
            Tok::String {
                value: string_content,
                is_fstring,
            }
        };

        Ok((start_pos, tok, end_pos))
    }

    fn is_identifier_start(&self, c: char) -> bool {
        match c {
            '_' => true,
            c => UnicodeXID::is_xid_start(c),
        }
    }

    fn is_identifier_continuation(&self) -> bool {
        if let Some(c) = self.chr0 {
            match c {
                '_' | '0'..='9' => true,
                c => UnicodeXID::is_xid_continue(c),
            }
        } else {
            false
        }
    }

    fn take_number(&mut self, radix: u32) -> Option<char> {
        let take_char = match radix {
            2 => match self.chr0 {
                Some('0'..='1') => true,
                _ => false,
            },
            8 => match self.chr0 {
                Some('0'..='7') => true,
                _ => false,
            },
            10 => match self.chr0 {
                Some('0'..='9') => true,
                _ => false,
            },
            16 => match self.chr0 {
                Some('0'..='9') | Some('a'..='f') | Some('A'..='F') => true,
                _ => false,
            },
            x => unimplemented!("Radix not implemented: {}", x),
        };

        if take_char {
            Some(self.next_char().unwrap())
        } else {
            None
        }
    }

    fn next_char(&mut self) -> Option<char> {
        let c = self.chr0;
        let nxt = self.chars.next();
        self.chr0 = self.chr1;
        self.chr1 = nxt;
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

    fn inner_next(&mut self) -> Option<LexResult> {
        if !self.pending.is_empty() {
            return Some(self.pending.remove(0));
        }

        'top_loop: loop {
            // Detect indentation levels
            if self.at_begin_of_line {
                self.at_begin_of_line = false;

                // Determine indentation:
                let mut spaces: usize = 0;
                let mut tabs: usize = 0;
                loop {
                    match self.chr0 {
                        Some(' ') => {
                            /*
                            if tabs != 0 {
                                // Don't allow spaces after tabs as part of indentation.
                                // This is technically stricter than python3 but spaces after
                                // tabs is even more insane than mixing spaces and tabs.
                                return Some(Err(LexicalError {
                                    error: LexicalErrorType::OtherError("Spaces not allowed as part of indentation after tabs".to_string()),
                                    location: self.get_pos(),
                                }));
                            }
                            */
                            self.next_char();
                            spaces += 1;
                        }
                        Some('\t') => {
                            if spaces != 0 {
                                // Don't allow tabs after spaces as part of indentation.
                                // This is technically stricter than python3 but spaces before
                                // tabs is even more insane than mixing spaces and tabs.
                                return Some(Err(LexicalError {
                                    error: LexicalErrorType::OtherError(
                                        "Tabs not allowed as part of indentation after spaces"
                                            .to_string(),
                                    ),
                                    location: self.get_pos(),
                                }));
                            }
                            self.next_char();
                            tabs += 1;
                        }
                        Some('#') => {
                            self.lex_comment();
                            self.at_begin_of_line = true;
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

                let indentation_level = IndentationLevel { spaces, tabs };

                if self.nesting == 0 {
                    // Determine indent or dedent:
                    let current_indentation = *self.indentation_stack.last().unwrap();
                    let ordering = indentation_level.compare_strict(&current_indentation);
                    match ordering {
                        Some(Ordering::Equal) => {
                            // Same same
                        }
                        Some(Ordering::Greater) => {
                            // New indentation level:
                            self.indentation_stack.push(indentation_level);
                            let tok_start = self.get_pos();
                            let tok_end = tok_start.clone();
                            return Some(Ok((tok_start, Tok::Indent, tok_end)));
                        }
                        Some(Ordering::Less) => {
                            // One or more dedentations
                            // Pop off other levels until col is found:

                            loop {
                                let ordering = indentation_level
                                    .compare_strict(self.indentation_stack.last().unwrap());
                                match ordering {
                                    Some(Ordering::Less) => {
                                        self.indentation_stack.pop();
                                        let tok_start = self.get_pos();
                                        let tok_end = tok_start.clone();
                                        self.pending.push(Ok((tok_start, Tok::Dedent, tok_end)));
                                    }
                                    None => {
                                        return Some(Err(LexicalError {
                                            error: LexicalErrorType::OtherError("inconsistent use of tabs and spaces in indentation".to_string()),
                                            location: self.get_pos(),
                                        }));
                                    }
                                    _ => {
                                        break;
                                    }
                                };
                            }

                            if indentation_level != *self.indentation_stack.last().unwrap() {
                                // TODO: handle wrong indentations
                                return Some(Err(LexicalError {
                                    error: LexicalErrorType::OtherError(
                                        "Non matching indentation levels!".to_string(),
                                    ),
                                    location: self.get_pos(),
                                }));
                            }

                            return Some(self.pending.remove(0));
                        }
                        None => {
                            return Some(Err(LexicalError {
                                error: LexicalErrorType::OtherError(
                                    "inconsistent use of tabs and spaces in indentation"
                                        .to_string(),
                                ),
                                location: self.get_pos(),
                            }));
                        }
                    }
                }
            }

            // Check if we have some character:
            if let Some(c) = self.chr0 {
                // First check identifier:
                if self.is_identifier_start(c) {
                    return Some(self.lex_identifier());
                } else if is_emoji_presentation(c) {
                    let tok_start = self.get_pos();
                    self.next_char();
                    let tok_end = self.get_pos();
                    return Some(Ok((
                        tok_start,
                        Tok::Name {
                            name: c.to_string(),
                        },
                        tok_end,
                    )));
                } else {
                    match c {
                        '0'..='9' => return Some(self.lex_number()),
                        '#' => {
                            self.lex_comment();
                            continue;
                        }
                        '"' => {
                            return Some(self.lex_string(false, false, false, false));
                        }
                        '\'' => {
                            return Some(self.lex_string(false, false, false, false));
                        }
                        '=' => {
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
                        '+' => {
                            let tok_start = self.get_pos();
                            self.next_char();
                            if let Some('=') = self.chr0 {
                                self.next_char();
                                let tok_end = self.get_pos();
                                return Some(Ok((tok_start, Tok::PlusEqual, tok_end)));
                            } else {
                                let tok_end = self.get_pos();
                                return Some(Ok((tok_start, Tok::Plus, tok_end)));
                            }
                        }
                        '*' => {
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
                                            return Some(Ok((
                                                tok_start,
                                                Tok::DoubleStarEqual,
                                                tok_end,
                                            )));
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
                        '/' => {
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
                                            return Some(Ok((
                                                tok_start,
                                                Tok::DoubleSlashEqual,
                                                tok_end,
                                            )));
                                        }
                                        _ => {
                                            let tok_end = self.get_pos();
                                            return Some(Ok((
                                                tok_start,
                                                Tok::DoubleSlash,
                                                tok_end,
                                            )));
                                        }
                                    }
                                }
                                _ => {
                                    let tok_end = self.get_pos();
                                    return Some(Ok((tok_start, Tok::Slash, tok_end)));
                                }
                            }
                        }
                        '%' => {
                            let tok_start = self.get_pos();
                            self.next_char();
                            if let Some('=') = self.chr0 {
                                self.next_char();
                                let tok_end = self.get_pos();
                                return Some(Ok((tok_start, Tok::PercentEqual, tok_end)));
                            } else {
                                let tok_end = self.get_pos();
                                return Some(Ok((tok_start, Tok::Percent, tok_end)));
                            }
                        }
                        '|' => {
                            let tok_start = self.get_pos();
                            self.next_char();
                            if let Some('=') = self.chr0 {
                                self.next_char();
                                let tok_end = self.get_pos();
                                return Some(Ok((tok_start, Tok::VbarEqual, tok_end)));
                            } else {
                                let tok_end = self.get_pos();
                                return Some(Ok((tok_start, Tok::Vbar, tok_end)));
                            }
                        }
                        '^' => {
                            let tok_start = self.get_pos();
                            self.next_char();
                            if let Some('=') = self.chr0 {
                                self.next_char();
                                let tok_end = self.get_pos();
                                return Some(Ok((tok_start, Tok::CircumflexEqual, tok_end)));
                            } else {
                                let tok_end = self.get_pos();
                                return Some(Ok((tok_start, Tok::CircumFlex, tok_end)));
                            }
                        }
                        '&' => {
                            let tok_start = self.get_pos();
                            self.next_char();
                            if let Some('=') = self.chr0 {
                                self.next_char();
                                let tok_end = self.get_pos();
                                return Some(Ok((tok_start, Tok::AmperEqual, tok_end)));
                            } else {
                                let tok_end = self.get_pos();
                                return Some(Ok((tok_start, Tok::Amper, tok_end)));
                            }
                        }
                        '-' => {
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
                        '@' => {
                            let tok_start = self.get_pos();
                            self.next_char();
                            if let Some('=') = self.chr0 {
                                self.next_char();
                                let tok_end = self.get_pos();
                                return Some(Ok((tok_start, Tok::AtEqual, tok_end)));
                            } else {
                                let tok_end = self.get_pos();
                                return Some(Ok((tok_start, Tok::At, tok_end)));
                            }
                        }
                        '!' => {
                            let tok_start = self.get_pos();
                            self.next_char();
                            if let Some('=') = self.chr0 {
                                self.next_char();
                                let tok_end = self.get_pos();
                                return Some(Ok((tok_start, Tok::NotEqual, tok_end)));
                            } else {
                                return Some(Err(LexicalError {
                                    error: LexicalErrorType::UnrecognizedToken { tok: '!' },
                                    location: tok_start,
                                }));
                            }
                        }
                        '~' => {
                            return Some(self.eat_single_char(Tok::Tilde));
                        }
                        '(' => {
                            let result = self.eat_single_char(Tok::Lpar);
                            self.nesting += 1;
                            return Some(result);
                        }
                        ')' => {
                            let result = self.eat_single_char(Tok::Rpar);
                            if self.nesting == 0 {
                                return Some(Err(LexicalError {
                                    error: LexicalErrorType::NestingError,
                                    location: self.get_pos(),
                                }));
                            }
                            self.nesting -= 1;
                            return Some(result);
                        }
                        '[' => {
                            let result = self.eat_single_char(Tok::Lsqb);
                            self.nesting += 1;
                            return Some(result);
                        }
                        ']' => {
                            let result = self.eat_single_char(Tok::Rsqb);
                            if self.nesting == 0 {
                                return Some(Err(LexicalError {
                                    error: LexicalErrorType::NestingError,
                                    location: self.get_pos(),
                                }));
                            }
                            self.nesting -= 1;
                            return Some(result);
                        }
                        '{' => {
                            let result = self.eat_single_char(Tok::Lbrace);
                            self.nesting += 1;
                            return Some(result);
                        }
                        '}' => {
                            let result = self.eat_single_char(Tok::Rbrace);
                            if self.nesting == 0 {
                                return Some(Err(LexicalError {
                                    error: LexicalErrorType::NestingError,
                                    location: self.get_pos(),
                                }));
                            }
                            self.nesting -= 1;
                            return Some(result);
                        }
                        ':' => {
                            return Some(self.eat_single_char(Tok::Colon));
                        }
                        ';' => {
                            return Some(self.eat_single_char(Tok::Semi));
                        }
                        '<' => {
                            let tok_start = self.get_pos();
                            self.next_char();
                            match self.chr0 {
                                Some('<') => {
                                    self.next_char();
                                    match self.chr0 {
                                        Some('=') => {
                                            self.next_char();
                                            let tok_end = self.get_pos();
                                            return Some(Ok((
                                                tok_start,
                                                Tok::LeftShiftEqual,
                                                tok_end,
                                            )));
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
                        '>' => {
                            let tok_start = self.get_pos();
                            self.next_char();
                            match self.chr0 {
                                Some('>') => {
                                    self.next_char();
                                    match self.chr0 {
                                        Some('=') => {
                                            self.next_char();
                                            let tok_end = self.get_pos();
                                            return Some(Ok((
                                                tok_start,
                                                Tok::RightShiftEqual,
                                                tok_end,
                                            )));
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
                        ',' => {
                            let tok_start = self.get_pos();
                            self.next_char();
                            let tok_end = self.get_pos();
                            return Some(Ok((tok_start, Tok::Comma, tok_end)));
                        }
                        '.' => {
                            if let Some('0'..='9') = self.chr1 {
                                return Some(self.lex_number());
                            } else {
                                let tok_start = self.get_pos();
                                self.next_char();
                                if let (Some('.'), Some('.')) = (&self.chr0, &self.chr1) {
                                    self.next_char();
                                    self.next_char();
                                    let tok_end = self.get_pos();
                                    return Some(Ok((tok_start, Tok::Ellipsis, tok_end)));
                                } else {
                                    let tok_end = self.get_pos();
                                    return Some(Ok((tok_start, Tok::Dot, tok_end)));
                                }
                            }
                        }
                        '\n' => {
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
                        ' ' => {
                            // Skip whitespaces
                            self.next_char();
                            continue;
                        }
                        _ => {
                            let c = self.next_char();
                            return Some(Err(LexicalError {
                                error: LexicalErrorType::UnrecognizedToken { tok: c.unwrap() },
                                location: self.get_pos(),
                            }));
                        } // Ignore all the rest..
                    }
                }
            } else {
                return None;
            }
        }
    }

    fn eat_single_char(&mut self, ty: Tok) -> LexResult {
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
impl<T> Iterator for Lexer<T>
where
    T: Iterator<Item = char>,
{
    type Item = LexResult;

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

fn lex_byte(s: String) -> Result<Vec<u8>, LexicalError> {
    let mut res = vec![];
    let mut escape = false; //flag if previous was \
    let mut hex_on = false; // hex mode on or off
    let mut hex_value = String::new();

    for c in s.chars() {
        if hex_on {
            if c.is_ascii_hexdigit() {
                if hex_value.is_empty() {
                    hex_value.push(c);
                    continue;
                } else {
                    hex_value.push(c);
                    res.push(u8::from_str_radix(&hex_value, 16).unwrap());
                    hex_on = false;
                    hex_value.clear();
                }
            } else {
                return Err(LexicalError {
                    error: LexicalErrorType::StringError,
                    location: Default::default(),
                });
            }
        } else {
            match (c, escape) {
                ('\\', true) => res.push(b'\\'),
                ('\\', false) => {
                    escape = true;
                    continue;
                }
                ('x', true) => hex_on = true,
                ('x', false) => res.push(b'x'),
                ('t', true) => res.push(b'\t'),
                ('t', false) => res.push(b't'),
                ('n', true) => res.push(b'\n'),
                ('n', false) => res.push(b'n'),
                ('r', true) => res.push(b'\r'),
                ('r', false) => res.push(b'r'),
                (x, true) => {
                    res.push(b'\\');
                    res.push(x as u8);
                }
                (x, false) => res.push(x as u8),
            }
            escape = false;
        }
    }
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::{make_tokenizer, NewlineHandler, Tok};
    use num_bigint::BigInt;
    use std::iter::FromIterator;
    use std::iter::Iterator;

    const WINDOWS_EOL: &str = "\r\n";
    const MAC_EOL: &str = "\r";
    const UNIX_EOL: &str = "\n";

    pub fn lex_source(source: &String) -> Vec<Tok> {
        let lexer = make_tokenizer(source);
        Vec::from_iter(lexer.map(|x| x.unwrap().1))
    }

    #[test]
    fn test_newline_processor() {
        // Escape \ followed by \n (by removal):
        let src = "b\\\r\n";
        assert_eq!(4, src.len());
        let nlh = NewlineHandler::new(src.chars());
        let x: Vec<char> = nlh.collect();
        assert_eq!(vec!['b', '\\', '\n'], x);
    }

    #[test]
    fn test_raw_string() {
        let source = String::from("r\"\\\\\" \"\\\\\"");
        let tokens = lex_source(&source);
        assert_eq!(
            tokens,
            vec![
                Tok::String {
                    value: "\\\\".to_string(),
                    is_fstring: false,
                },
                Tok::String {
                    value: "\\".to_string(),
                    is_fstring: false,
                }
            ]
        );
    }

    #[test]
    fn test_numbers() {
        let source = String::from("0x2f 0b1101 0 123 0.2 2j 2.2j");
        let tokens = lex_source(&source);
        assert_eq!(
            tokens,
            vec![
                Tok::Int {
                    value: BigInt::from(47),
                },
                Tok::Int {
                    value: BigInt::from(13),
                },
                Tok::Int {
                    value: BigInt::from(0),
                },
                Tok::Int {
                    value: BigInt::from(123),
                },
                Tok::Float { value: 0.2 },
                Tok::Complex {
                    real: 0.0,
                    imag: 2.0,
                },
                Tok::Complex {
                    real: 0.0,
                    imag: 2.2,
                },
            ]
        );
    }

    macro_rules! test_line_comment {
        ($($name:ident: $eol:expr,)*) => {
            $(
            #[test]
            fn $name() {
                let source = String::from(format!(r"99232  # {}", $eol));
                let tokens = lex_source(&source);
                assert_eq!(tokens, vec![Tok::Int { value: BigInt::from(99232) }]);
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
                        Tok::Int { value: BigInt::from(123) },
                        Tok::Newline,
                        Tok::Int { value: BigInt::from(456) },
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
                Tok::Int {
                    value: BigInt::from(99)
                },
                Tok::Plus,
                Tok::Int {
                    value: BigInt::from(2)
                },
                Tok::Minus,
                Tok::Int {
                    value: BigInt::from(0)
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
                        Tok::Int { value: BigInt::from(99) },
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
                        Tok::Int { value: BigInt::from(99) },
                        Tok::Newline,
                        Tok::Dedent,
                        Tok::Dedent,
                    ]
                );
            }
        )*
        }
    }

    macro_rules! test_double_dedent_with_tabs {
        ($($name:ident: $eol:expr,)*) => {
        $(
            #[test]
            fn $name() {
                let source = String::from(format!("def foo():{}\tif x:{}{}\t return 99{}{}", $eol, $eol, $eol, $eol, $eol));
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
                        Tok::Int { value: BigInt::from(99) },
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

    test_double_dedent_with_tabs! {
        test_double_dedent_tabs_windows_eol: WINDOWS_EOL,
        test_double_dedent_tabs_mac_eol: MAC_EOL,
        test_double_dedent_tabs_unix_eol: UNIX_EOL,
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
                        Tok::Int { value: BigInt::from(1) },
                        Tok::Comma,
                        Tok::Int { value: BigInt::from(2) },
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
        let source = String::from(r#""double" 'single' 'can\'t' "\\\"" '\t\r\n' '\g' r'raw\''"#);
        let tokens = lex_source(&source);
        assert_eq!(
            tokens,
            vec![
                Tok::String {
                    value: String::from("double"),
                    is_fstring: false,
                },
                Tok::String {
                    value: String::from("single"),
                    is_fstring: false,
                },
                Tok::String {
                    value: String::from("can't"),
                    is_fstring: false,
                },
                Tok::String {
                    value: String::from("\\\""),
                    is_fstring: false,
                },
                Tok::String {
                    value: String::from("\t\r\n"),
                    is_fstring: false,
                },
                Tok::String {
                    value: String::from("\\g"),
                    is_fstring: false,
                },
                Tok::String {
                    value: String::from("raw\'"),
                    is_fstring: false,
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
                            is_fstring: false,
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

    #[test]
    fn test_byte() {
        // single quote
        let all = r##"b'\x00\x01\x02\x03\x04\x05\x06\x07\x08\t\n\x0b\x0c\r\x0e\x0f\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1a\x1b\x1c\x1d\x1e\x1f !"#$%&\'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~\x7f\x80\x81\x82\x83\x84\x85\x86\x87\x88\x89\x8a\x8b\x8c\x8d\x8e\x8f\x90\x91\x92\x93\x94\x95\x96\x97\x98\x99\x9a\x9b\x9c\x9d\x9e\x9f\xa0\xa1\xa2\xa3\xa4\xa5\xa6\xa7\xa8\xa9\xaa\xab\xac\xad\xae\xaf\xb0\xb1\xb2\xb3\xb4\xb5\xb6\xb7\xb8\xb9\xba\xbb\xbc\xbd\xbe\xbf\xc0\xc1\xc2\xc3\xc4\xc5\xc6\xc7\xc8\xc9\xca\xcb\xcc\xcd\xce\xcf\xd0\xd1\xd2\xd3\xd4\xd5\xd6\xd7\xd8\xd9\xda\xdb\xdc\xdd\xde\xdf\xe0\xe1\xe2\xe3\xe4\xe5\xe6\xe7\xe8\xe9\xea\xeb\xec\xed\xee\xef\xf0\xf1\xf2\xf3\xf4\xf5\xf6\xf7\xf8\xf9\xfa\xfb\xfc\xfd\xfe\xff'"##;
        let source = String::from(all);
        let tokens = lex_source(&source);
        let res = (0..=255).collect::<Vec<u8>>();
        assert_eq!(tokens, vec![Tok::Bytes { value: res }]);

        // double quote
        let all = r##"b"\x00\x01\x02\x03\x04\x05\x06\x07\x08\t\n\x0b\x0c\r\x0e\x0f\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1a\x1b\x1c\x1d\x1e\x1f !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~\x7f\x80\x81\x82\x83\x84\x85\x86\x87\x88\x89\x8a\x8b\x8c\x8d\x8e\x8f\x90\x91\x92\x93\x94\x95\x96\x97\x98\x99\x9a\x9b\x9c\x9d\x9e\x9f\xa0\xa1\xa2\xa3\xa4\xa5\xa6\xa7\xa8\xa9\xaa\xab\xac\xad\xae\xaf\xb0\xb1\xb2\xb3\xb4\xb5\xb6\xb7\xb8\xb9\xba\xbb\xbc\xbd\xbe\xbf\xc0\xc1\xc2\xc3\xc4\xc5\xc6\xc7\xc8\xc9\xca\xcb\xcc\xcd\xce\xcf\xd0\xd1\xd2\xd3\xd4\xd5\xd6\xd7\xd8\xd9\xda\xdb\xdc\xdd\xde\xdf\xe0\xe1\xe2\xe3\xe4\xe5\xe6\xe7\xe8\xe9\xea\xeb\xec\xed\xee\xef\xf0\xf1\xf2\xf3\xf4\xf5\xf6\xf7\xf8\xf9\xfa\xfb\xfc\xfd\xfe\xff""##;
        let source = String::from(all);
        let tokens = lex_source(&source);
        let res = (0..=255).collect::<Vec<u8>>();
        assert_eq!(tokens, vec![Tok::Bytes { value: res }]);

        // backslash doesnt escape
        let all = r##"b"omkmok\Xaa""##;
        let source = String::from(all);
        let tokens = lex_source(&source);
        let res = vec![111, 109, 107, 109, 111, 107, 92, 88, 97, 97];
        assert_eq!(tokens, vec![Tok::Bytes { value: res }]);
    }
}
