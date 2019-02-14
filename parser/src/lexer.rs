//! This module takes care of lexing python source text. This means source
//! code is translated into separate tokens.

pub use super::token::Tok;
use num_bigint::BigInt;
use num_traits::Num;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::str::FromStr;

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
    pending: Vec<Spanned<Tok>>,
    chr0: Option<char>,
    chr1: Option<char>,
    location: Location,
}

#[derive(Debug)]
pub enum LexicalError {
    StringError,
    NestingError,
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

pub type Spanned<Tok> = Result<(Location, Tok, Location), LexicalError>;

pub fn make_tokenizer<'a>(source: &'a str) -> impl Iterator<Item = Spanned<Tok>> + 'a {
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

        while self.is_char() {
            name.push(self.next_char().unwrap());
        }
        let end_pos = self.get_pos();

        let mut keywords = get_keywords();

        if keywords.contains_key(&name) {
            Ok((start_pos, keywords.remove(&name).unwrap(), end_pos))
        } else {
            Ok((start_pos, Tok::Name { name }, end_pos))
        }
    }

    fn lex_number(&mut self) -> Spanned<Tok> {
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

    fn lex_number_radix(&mut self, start_pos: Location, radix: u32) -> Spanned<Tok> {
        let mut value_text = String::new();

        loop {
            if self.is_number(radix) {
                value_text.push(self.next_char().unwrap());
            } else if self.chr0 == Some('_') {
                self.next_char();
            } else {
                break;
            }
        }

        let end_pos = self.get_pos();
        let value = BigInt::from_str_radix(&value_text, radix).unwrap();
        Ok((start_pos, Tok::Int { value }, end_pos))
    }

    fn lex_normal_number(&mut self) -> Spanned<Tok> {
        let start_pos = self.get_pos();

        let mut value_text = String::new();

        // Normal number:
        while self.is_number(10) {
            value_text.push(self.next_char().unwrap());
        }

        // If float:
        if self.chr0 == Some('.') || self.chr0 == Some('e') {
            // Take '.':
            if self.chr0 == Some('.') {
                value_text.push(self.next_char().unwrap());
                while self.is_number(10) {
                    value_text.push(self.next_char().unwrap());
                }
            }

            // 1e6 for example:
            if self.chr0 == Some('e') {
                value_text.push(self.next_char().unwrap());

                // Optional +/-
                if self.chr0 == Some('-') || self.chr0 == Some('+') {
                    value_text.push(self.next_char().unwrap());
                }

                while self.is_number(10) {
                    value_text.push(self.next_char().unwrap());
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
    ) -> Spanned<Tok> {
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
                    if is_raw {
                        string_content.push('\\');
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
                                return Err(LexicalError::StringError);
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

        let tok = if is_bytes {
            Tok::Bytes {
                value: string_content.as_bytes().to_vec(),
            }
        } else {
            Tok::String {
                value: string_content,
                is_fstring,
            }
        };

        Ok((start_pos, tok, end_pos))
    }

    fn is_char(&self) -> bool {
        match self.chr0 {
            Some('a'...'z') | Some('A'...'Z') | Some('_') | Some('0'...'9') => true,
            _ => false,
        }
    }

    fn is_number(&self, radix: u32) -> bool {
        match radix {
            2 => match self.chr0 {
                Some('0'...'1') => true,
                _ => false,
            },
            8 => match self.chr0 {
                Some('0'...'7') => true,
                _ => false,
            },
            10 => match self.chr0 {
                Some('0'...'9') => true,
                _ => false,
            },
            16 => match self.chr0 {
                Some('0'...'9') | Some('a'...'f') | Some('A'...'F') => true,
                _ => false,
            },
            x => unimplemented!("Radix not implemented: {}", x),
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

    fn inner_next(&mut self) -> Option<Spanned<Tok>> {
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
                            self.next_char();
                            spaces += 1;
                        }
                        Some('\t') => {
                            if spaces != 0 {
                                // Don't allow tabs after spaces as part of indentation.
                                // This is technically stricter than python3 but spaces before
                                // tabs is even more insane than mixing spaces and tabs.
                                panic!("Tabs not allowed as part of indentation after spaces");
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
                                        panic!("inconsistent use of tabs and spaces in indentation")
                                    }
                                    _ => {
                                        break;
                                    }
                                };
                            }

                            if indentation_level != *self.indentation_stack.last().unwrap() {
                                // TODO: handle wrong indentations
                                panic!("Non matching indentation levels!");
                            }

                            return Some(self.pending.remove(0));
                        }
                        None => panic!("inconsistent use of tabs and spaces in indentation"),
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
                    return Some(self.lex_string(false, false, false, false));
                }
                Some('\'') => {
                    return Some(self.lex_string(false, false, false, false));
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
                    if self.nesting == 0 {
                        return Some(Err(LexicalError::NestingError));
                    }
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
                    if self.nesting == 0 {
                        return Some(Err(LexicalError::NestingError));
                    }
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
                    if self.nesting == 0 {
                        return Some(Err(LexicalError::NestingError));
                    }
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
impl<T> Iterator for Lexer<T>
where
    T: Iterator<Item = char>,
{
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
        let source = String::from(r#""double" 'single' 'can\'t' "\\\"" '\t\r\n' '\g'"#);
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
}
