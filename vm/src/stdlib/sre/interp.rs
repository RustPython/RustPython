// good luck to those that follow; here be dragons

use super::_sre::MAXREPEAT;
use super::constants::{SreAtCode, SreCatCode, SreFlag, SreOpcode};
use std::collections::HashMap;
use std::convert::TryFrom;

pub struct State<'a> {
    string: &'a str,
    // chars count
    string_len: usize,
    start: usize,
    end: usize,
    flags: SreFlag,
    pattern_codes: Vec<u32>,
    marks: Vec<usize>,
    lastindex: isize,
    marks_stack: Vec<usize>,
    context_stack: Vec<MatchContext>,
    repeat: Option<usize>,
    string_position: usize,
}

impl<'a> State<'a> {
    pub(crate) fn new(
        string: &'a str,
        start: usize,
        end: usize,
        flags: SreFlag,
        pattern_codes: Vec<u32>,
    ) -> Self {
        let string_len = string.chars().count();
        let end = std::cmp::min(end, string_len);
        let start = std::cmp::min(start, end);
        Self {
            string,
            string_len,
            start,
            end,
            flags,
            pattern_codes,
            lastindex: -1,
            marks_stack: Vec::new(),
            context_stack: Vec::new(),
            repeat: None,
            marks: Vec::new(),
            string_position: start,
        }
    }

    fn reset(&mut self) {
        self.marks.clear();
        self.lastindex = -1;
        self.marks_stack.clear();
        self.context_stack.clear();
        self.repeat = None;
    }
}

pub(crate) fn pymatch(mut state: State) -> bool {
    let ctx = MatchContext {
        string_position: state.start,
        string_offset: state.string.char_indices().nth(state.start).unwrap().0,
        code_position: 0,
        has_matched: None,
    };
    state.context_stack.push(ctx);

    let mut has_matched = None;
    loop {
        if state.context_stack.is_empty() {
            break;
        }
        let ctx_id = state.context_stack.len() - 1;
        let mut drive = MatchContextDrive::drive(ctx_id, state);
        let mut dispatcher = OpcodeDispatcher::new();

        has_matched = dispatcher.pymatch(&mut drive);
        state = drive.take();
        if has_matched.is_some() {
            state.context_stack.pop();
        }
    }
    has_matched.unwrap_or(false)
}

#[derive(Debug, Copy, Clone)]
struct MatchContext {
    string_position: usize,
    string_offset: usize,
    code_position: usize,
    has_matched: Option<bool>,
}

struct MatchContextDrive<'a> {
    state: State<'a>,
    ctx_id: usize,
}

impl<'a> MatchContextDrive<'a> {
    fn id(&self) -> usize {
        self.ctx_id
    }
    fn ctx_mut(&mut self) -> &mut MatchContext {
        &mut self.state.context_stack[self.ctx_id]
    }
    fn ctx(&self) -> &MatchContext {
        &self.state.context_stack[self.ctx_id]
    }
    fn push_new_context(&mut self, pattern_offset: usize) -> usize {
        let ctx = self.ctx();
        let child_ctx = MatchContext {
            string_position: ctx.string_position,
            string_offset: ctx.string_offset,
            code_position: ctx.code_position + pattern_offset,
            has_matched: None,
        };
        self.state.context_stack.push(child_ctx);
        self.state.context_stack.len() - 1
    }
    fn drive(ctx_id: usize, state: State<'a>) -> Self {
        Self { state, ctx_id }
    }
    fn take(self) -> State<'a> {
        self.state
    }
    fn str(&self) -> &str {
        unsafe {
            std::str::from_utf8_unchecked(&self.state.string.as_bytes()[self.ctx().string_offset..])
        }
    }
    fn peek_char(&self) -> char {
        self.str().chars().next().unwrap()
    }
    fn peek_code(&self, peek: usize) -> u32 {
        self.state.pattern_codes[self.ctx().code_position + peek]
    }
    fn skip_char(&mut self, skip_count: usize) {
        let skipped = self.str().char_indices().nth(skip_count).unwrap().0;
        self.ctx_mut().string_position += skip_count;
        self.ctx_mut().string_offset += skipped;
    }
    fn skip_code(&mut self, skip_count: usize) {
        self.ctx_mut().code_position += skip_count;
    }
    fn remaining_chars(&self) -> usize {
        self.state.end - self.ctx().string_position
    }
    fn remaining_codes(&self) -> usize {
        self.state.pattern_codes.len() - self.ctx().code_position
    }
    fn at_beginning(&self) -> bool {
        self.ctx().string_position == self.state.start
    }
    fn at_end(&self) -> bool {
        self.ctx().string_position == self.state.end
    }
    fn at_linebreak(&self) -> bool {
        !self.at_end() && is_linebreak(self.peek_char())
    }
    fn at_boundary<F: FnMut(char) -> bool>(&self, mut word_checker: F) -> bool {
        if self.at_beginning() && self.at_end() {
            return false;
        }
        let that = !self.at_beginning() && word_checker(self.back_peek_char());
        let this = !self.at_end() && word_checker(self.peek_char());
        this != that
    }
    fn back_peek_offset(&self) -> usize {
        let bytes = self.state.string.as_bytes();
        let mut offset = self.ctx().string_offset - 1;
        if !is_utf8_first_byte(bytes[offset]) {
            offset -= 1;
            if !is_utf8_first_byte(bytes[offset]) {
                offset -= 1;
                if !is_utf8_first_byte(bytes[offset]) {
                    offset -= 1;
                    if !is_utf8_first_byte(bytes[offset]) {
                        panic!("not utf-8 code point");
                    }
                }
            }
        }
        offset
    }
    fn back_peek_char(&self) -> char {
        let bytes = self.state.string.as_bytes();
        let offset = self.back_peek_offset();
        let current_offset = self.ctx().string_offset;
        let code = match current_offset - offset {
            1 => u32::from_ne_bytes([0, 0, 0, bytes[offset]]),
            2 => u32::from_ne_bytes([0, 0, bytes[offset], bytes[offset + 1]]),
            3 => u32::from_ne_bytes([0, bytes[offset], bytes[offset + 1], bytes[offset + 2]]),
            4 => u32::from_ne_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ]),
            _ => unreachable!(),
        };
        unsafe { std::mem::transmute(code) }
    }
    fn back_skip_char(&mut self, skip_count: usize) {
        self.ctx_mut().string_position -= skip_count;
        for _ in 0..skip_count {
            self.ctx_mut().string_offset = self.back_peek_offset();
        }
    }
}

trait OpcodeExecutor {
    fn next(&mut self, drive: &mut MatchContextDrive) -> Option<()>;
}

struct OpUnimplemented {}
impl OpcodeExecutor for OpUnimplemented {
    fn next(&mut self, drive: &mut MatchContextDrive) -> Option<()> {
        drive.ctx_mut().has_matched = Some(false);
        None
    }
}

struct OpOnce<F> {
    f: Option<F>,
}
impl<F: FnOnce(&mut MatchContextDrive)> OpcodeExecutor for OpOnce<F> {
    fn next(&mut self, drive: &mut MatchContextDrive) -> Option<()> {
        let f = self.f.take()?;
        f(drive);
        None
    }
}
fn once<F: FnOnce(&mut MatchContextDrive)>(f: F) -> Box<OpOnce<F>> {
    Box::new(OpOnce { f: Some(f) })
}

fn unimplemented() -> Box<OpUnimplemented> {
    Box::new(OpUnimplemented {})
}

struct OpMinRepeatOne {
    trace_id: usize,
    mincount: usize,
    maxcount: usize,
    count: usize,
    child_ctx_id: usize,
}
impl OpcodeExecutor for OpMinRepeatOne {
    fn next(&mut self, drive: &mut MatchContextDrive) -> Option<()> {
        None
        // match self.trace_id {
        //     0 => self._0(drive),
        //     _ => unreachable!(),
        // }
    }
}
impl Default for OpMinRepeatOne {
    fn default() -> Self {
        OpMinRepeatOne {
            trace_id: 0,
            mincount: 0,
            maxcount: 0,
            count: 0,
            child_ctx_id: 0,
        }
    }
}
// impl OpMinRepeatOne {
//     fn _0(&mut self, drive: &mut MatchContextDrive) -> Option<()> {
//         self.mincount = drive.peek_code(2) as usize;
//         self.maxcount = drive.peek_code(3) as usize;

//         if drive.remaining_chars() < self.mincount {
//             drive.ctx_mut().has_matched = Some(false);
//             return None;
//         }

//         drive.state.string_position = drive.ctx().string_position;

//         self.count = if self.mincount == 0 {
//             0
//         } else {
//             let count = count_repetitions(drive, self.mincount);
//             if count < self.mincount {
//                 drive.ctx_mut().has_matched = Some(false);
//                 return None;
//             }
//             drive.skip_char(count);
//             count
//         };

//         if drive.peek_code(drive.peek_code(1) as usize + 1) == SreOpcode::SUCCESS as u32 {
//             drive.state.string_position = drive.ctx().string_position;
//             drive.ctx_mut().has_matched = Some(true);
//             return None;
//         }

//         // mark push
//         self.trace_id = 1;
//         self._1(drive)
//     }
//     fn _1(&mut self, drive: &mut MatchContextDrive) -> Option<()> {
//         if self.maxcount == SRE_MAXREPEAT || self.count <= self.maxcount {
//             drive.state.string_position = drive.ctx().string_position;
//             self.child_ctx_id = drive.push_new_context(drive.peek_code(1) as usize + 1);
//             self.trace_id = 2;
//             return Some(());
//         }

//         // mark discard
//         drive.ctx_mut().has_matched = Some(false);
//         None
//     }
//     fn _2(&mut self, drive: &mut MatchContextDrive) -> Option<()> {
//         if let Some(true) = drive.state.context_stack[self.child_ctx_id].has_matched {
//             drive.ctx_mut().has_matched = Some(true);
//             return None;
//         }
//         drive.state.string_position = drive.ctx().string_position;
//         if count_repetitions(drive, 1) == 0 {
//             self.trace_id = 3;
//             return self._3(drive);
//         }
//         drive.skip_char(1);
//         self.count += 1;
//         // marks pop keep
//         self.trace_id = 1;
//         self._1(drive)
//     }
//     fn _3(&mut self, drive: &mut MatchContextDrive) -> Option<()> {
//         // mark discard
//         drive.ctx_mut().has_matched = Some(false);
//         None
//     }
// }

struct OpcodeDispatcher {
    executing_contexts: HashMap<usize, Box<dyn OpcodeExecutor>>,
}
impl OpcodeDispatcher {
    fn new() -> Self {
        Self {
            executing_contexts: HashMap::new(),
        }
    }
    // Returns True if the current context matches, False if it doesn't and
    // None if matching is not finished, ie must be resumed after child
    // contexts have been matched.
    fn pymatch(&mut self, drive: &mut MatchContextDrive) -> Option<bool> {
        while drive.remaining_codes() > 0 && drive.ctx().has_matched.is_none() {
            let code = drive.peek_code(0);
            let opcode = SreOpcode::try_from(code).unwrap();
            self.dispatch(opcode, drive);
        }
        match drive.ctx().has_matched {
            Some(matched) => Some(matched),
            None => {
                drive.ctx_mut().has_matched = Some(false);
                Some(false)
            }
        }
    }

    // Dispatches a context on a given opcode. Returns True if the context
    // is done matching, False if it must be resumed when next encountered.
    fn dispatch(&mut self, opcode: SreOpcode, drive: &mut MatchContextDrive) -> bool {
        let mut executor = match self.executing_contexts.remove_entry(&drive.id()) {
            Some((_, executor)) => executor,
            None => self.dispatch_table(opcode),
        };
        if let Some(()) = executor.next(drive) {
            self.executing_contexts.insert(drive.id(), executor);
            false
        } else {
            true
        }
    }

    fn dispatch_table(&mut self, opcode: SreOpcode) -> Box<dyn OpcodeExecutor> {
        match opcode {
            SreOpcode::FAILURE => once(|drive| {
                drive.ctx_mut().has_matched = Some(false);
            }),
            SreOpcode::SUCCESS => once(|drive| {
                drive.state.string_position = drive.ctx().string_position;
                drive.ctx_mut().has_matched = Some(true);
            }),
            SreOpcode::ANY => once(|drive| {
                if drive.at_end() || drive.at_linebreak() {
                    drive.ctx_mut().has_matched = Some(false);
                } else {
                    drive.skip_code(1);
                    drive.skip_char(1);
                }
            }),
            SreOpcode::ANY_ALL => once(|drive| {
                if drive.at_end() {
                    drive.ctx_mut().has_matched = Some(false);
                } else {
                    drive.skip_code(1);
                    drive.skip_char(1);
                }
            }),
            SreOpcode::ASSERT => Box::new(OpAssert::default()),
            SreOpcode::ASSERT_NOT => unimplemented(),
            SreOpcode::AT => once(|drive| {
                let atcode = SreAtCode::try_from(drive.peek_code(1)).unwrap();
                if !at(drive, atcode) {
                    drive.ctx_mut().has_matched = Some(false);
                } else {
                    drive.skip_code(2);
                }
            }),
            SreOpcode::BRANCH => unimplemented(),
            SreOpcode::CALL => unimplemented(),
            SreOpcode::CATEGORY => unimplemented(),
            SreOpcode::CHARSET => unimplemented(),
            SreOpcode::BIGCHARSET => unimplemented(),
            SreOpcode::GROUPREF => unimplemented(),
            SreOpcode::GROUPREF_EXISTS => unimplemented(),
            SreOpcode::GROUPREF_IGNORE => unimplemented(),
            SreOpcode::IN => unimplemented(),
            SreOpcode::IN_IGNORE => unimplemented(),
            SreOpcode::INFO | SreOpcode::JUMP => once(|drive| {
                drive.skip_code(drive.peek_code(1) as usize + 1);
            }),
            SreOpcode::LITERAL => once(|drive| {
                if drive.at_end() || drive.peek_char() as u32 != drive.peek_code(1) {
                    drive.ctx_mut().has_matched = Some(false);
                }
                drive.skip_code(2);
                drive.skip_char(1);
            }),
            SreOpcode::LITERAL_IGNORE => once(|drive| {
                let code = drive.peek_code(1);
                let c = drive.peek_char();
                if drive.at_end()
                    || (c.to_ascii_lowercase() as u32 != code
                        && c.to_ascii_uppercase() as u32 != code)
                {
                    drive.ctx_mut().has_matched = Some(false);
                }
                drive.skip_code(2);
                drive.skip_char(1);
            }),
            SreOpcode::MARK => unimplemented(),
            SreOpcode::MAX_UNTIL => unimplemented(),
            SreOpcode::MIN_UNTIL => unimplemented(),
            SreOpcode::NOT_LITERAL => once(|drive| {
                if drive.at_end() || drive.peek_char() as u32 == drive.peek_code(1) {
                    drive.ctx_mut().has_matched = Some(false);
                }
                drive.skip_code(2);
                drive.skip_char(1);
            }),
            SreOpcode::NOT_LITERAL_IGNORE => once(|drive| {
                let code = drive.peek_code(1);
                let c = drive.peek_char();
                if drive.at_end()
                    || (c.to_ascii_lowercase() as u32 == code
                        || c.to_ascii_uppercase() as u32 == code)
                {
                    drive.ctx_mut().has_matched = Some(false);
                }
                drive.skip_code(2);
                drive.skip_char(1);
            }),
            SreOpcode::NEGATE => unimplemented(),
            SreOpcode::RANGE => unimplemented(),
            SreOpcode::REPEAT => unimplemented(),
            SreOpcode::REPEAT_ONE => unimplemented(),
            SreOpcode::SUBPATTERN => unimplemented(),
            SreOpcode::MIN_REPEAT_ONE => Box::new(OpMinRepeatOne::default()),
            SreOpcode::GROUPREF_LOC_IGNORE => unimplemented(),
            SreOpcode::IN_LOC_IGNORE => unimplemented(),
            SreOpcode::LITERAL_LOC_IGNORE => unimplemented(),
            SreOpcode::NOT_LITERAL_LOC_IGNORE => unimplemented(),
            SreOpcode::GROUPREF_UNI_IGNORE => unimplemented(),
            SreOpcode::IN_UNI_IGNORE => unimplemented(),
            SreOpcode::LITERAL_UNI_IGNORE => unimplemented(),
            SreOpcode::NOT_LITERAL_UNI_IGNORE => unimplemented(),
            SreOpcode::RANGE_UNI_IGNORE => unimplemented(),
        }
    }

    // Returns the number of repetitions of a single item, starting from the
    // current string position. The code pointer is expected to point to a
    // REPEAT_ONE operation (with the repeated 4 ahead).
    fn count_repetitions(&mut self, drive: &mut MatchContextDrive, maxcount: usize) -> usize {
        let mut count = 0;
        let mut real_maxcount = drive.remaining_chars();
        if maxcount < real_maxcount && maxcount != MAXREPEAT {
            real_maxcount = maxcount;
        }
        let code_position = drive.ctx().code_position;
        let string_position = drive.ctx().string_position;
        drive.skip_code(4);
        let reset_position = drive.ctx().code_position;
        while count < real_maxcount {
            drive.ctx_mut().code_position = reset_position;
            let opcode = SreOpcode::try_from(drive.peek_code(1)).unwrap();
            self.dispatch(opcode, drive);
            if drive.ctx().has_matched == Some(false) {
                break;
            }
            count += 1;
        }
        drive.ctx_mut().has_matched = None;
        drive.ctx_mut().code_position = code_position;
        drive.ctx_mut().string_position = string_position;
        count
    }
}

fn at(drive: &mut MatchContextDrive, atcode: SreAtCode) -> bool {
    match atcode {
        SreAtCode::BEGINNING | SreAtCode::BEGINNING_STRING => drive.at_beginning(),
        SreAtCode::BEGINNING_LINE => drive.at_beginning() || is_linebreak(drive.back_peek_char()),
        SreAtCode::BOUNDARY => drive.at_boundary(is_word),
        SreAtCode::NON_BOUNDARY => !drive.at_boundary(is_word),
        SreAtCode::END => (drive.remaining_chars() == 1 && drive.at_linebreak()) || drive.at_end(),
        SreAtCode::END_LINE => drive.at_linebreak() || drive.at_end(),
        SreAtCode::END_STRING => drive.at_end(),
        SreAtCode::LOC_BOUNDARY => drive.at_boundary(is_loc_word),
        SreAtCode::LOC_NON_BOUNDARY => !drive.at_boundary(is_loc_word),
        SreAtCode::UNI_BOUNDARY => drive.at_boundary(is_uni_word),
        SreAtCode::UNI_NON_BOUNDARY => !drive.at_boundary(is_uni_word),
    }
}

fn category(catcode: SreCatCode, c: char) -> bool {
    match catcode {
        SreCatCode::DIGIT => is_digit(c),
        SreCatCode::NOT_DIGIT => !is_digit(c),
        SreCatCode::SPACE => is_space(c),
        SreCatCode::NOT_SPACE => !is_space(c),
        SreCatCode::WORD => is_word(c),
        SreCatCode::NOT_WORD => !is_word(c),
        SreCatCode::LINEBREAK => is_linebreak(c),
        SreCatCode::NOT_LINEBREAK => !is_linebreak(c),
        SreCatCode::LOC_WORD => is_loc_word(c),
        SreCatCode::LOC_NOT_WORD => !is_loc_word(c),
        SreCatCode::UNI_DIGIT => is_uni_digit(c),
        SreCatCode::UNI_NOT_DIGIT => !is_uni_digit(c),
        SreCatCode::UNI_SPACE => is_uni_space(c),
        SreCatCode::UNI_NOT_SPACE => !is_uni_space(c),
        SreCatCode::UNI_WORD => is_uni_word(c),
        SreCatCode::UNI_NOT_WORD => !is_uni_word(c),
        SreCatCode::UNI_LINEBREAK => is_uni_linebreak(c),
        SreCatCode::UNI_NOT_LINEBREAK => !is_uni_linebreak(c),
    }
}

fn charset(set: &[u32], c: char) -> bool {
    /* check if character is a member of the given set */
    let ch = c as u32;
    let mut ok = true;
    let mut i = 0;
    while i < set.len() {
        let opcode = match SreOpcode::try_from(set[i]) {
            Ok(code) => code,
            Err(_) => {
                break;
            }
        };
        match opcode {
            SreOpcode::FAILURE => {
                return !ok;
            }
            SreOpcode::CATEGORY => {
                /* <CATEGORY> <code> */
                let catcode = match SreCatCode::try_from(set[i + 1]) {
                    Ok(code) => code,
                    Err(_) => {
                        break;
                    }
                };
                if category(catcode, c) {
                    return ok;
                }
                i += 2;
            }
            SreOpcode::CHARSET => {
                /* <CHARSET> <bitmap> */
                if ch < 256 && (set[(ch / 32) as usize] & (1 << (32 - 1))) != 0 {
                    return ok;
                }
                i += 8;
            }
            SreOpcode::BIGCHARSET => {
                /* <BIGCHARSET> <blockcount> <256 blockindices> <blocks> */
                let count = set[i + 1];
                if ch < 0x10000 {
                    let blockindices: &[u8] = unsafe { std::mem::transmute(&set[i + 2..]) };
                    let block = blockindices[(ch >> 8) as usize];
                    if set[2 + 64 + ((block as u32 * 256 + (ch & 255)) / 32) as usize]
                        & (1 << (ch & (32 - 1)))
                        != 0
                    {
                        return ok;
                    }
                }
                i += 2 + 64 + count as usize * 8;
            }
            SreOpcode::LITERAL => {
                /* <LITERAL> <code> */
                if ch == set[i + 1] {
                    return ok;
                }
                i += 2;
            }
            SreOpcode::NEGATE => {
                ok = !ok;
                i += 1;
            }
            SreOpcode::RANGE => {
                /* <RANGE> <lower> <upper> */
                if set[i + 1] <= ch && ch <= set[i + 2] {
                    return ok;
                }
                i += 3;
            }
            SreOpcode::RANGE_UNI_IGNORE => {
                /* <RANGE_UNI_IGNORE> <lower> <upper> */
                if set[i + 1] <= ch && ch <= set[i + 2] {
                    return ok;
                }
                let ch = upper_unicode(c) as u32;
                if set[i + 1] <= ch && ch <= set[i + 2] {
                    return ok;
                }
                i += 3;
            }
            _ => {
                break;
            }
        }
    }
    /* internal error -- there's not much we can do about it
    here, so let's just pretend it didn't match... */
    false
}

fn count(drive: MatchContextDrive, maxcount: usize) -> usize {
    let string_position = drive.state.string_position;
    let maxcount = std::cmp::min(maxcount, drive.remaining_chars());

    let opcode = SreOpcode::try_from(drive.peek_code(1)).unwrap();
    match opcode {
        SreOpcode::FAILURE => {}
        SreOpcode::SUCCESS => {}
        SreOpcode::ANY => {}
        SreOpcode::ANY_ALL => {}
        SreOpcode::ASSERT => {}
        SreOpcode::ASSERT_NOT => {}
        SreOpcode::AT => {}
        SreOpcode::BRANCH => {}
        SreOpcode::CALL => {}
        SreOpcode::CATEGORY => {}
        SreOpcode::CHARSET => {}
        SreOpcode::BIGCHARSET => {}
        SreOpcode::GROUPREF => {}
        SreOpcode::GROUPREF_EXISTS => {}
        SreOpcode::IN => {
        }
        SreOpcode::INFO => {}
        SreOpcode::JUMP => {}
        SreOpcode::LITERAL => {}
        SreOpcode::MARK => {}
        SreOpcode::MAX_UNTIL => {}
        SreOpcode::MIN_UNTIL => {}
        SreOpcode::NOT_LITERAL => {}
        SreOpcode::NEGATE => {}
        SreOpcode::RANGE => {}
        SreOpcode::REPEAT => {}
        SreOpcode::REPEAT_ONE => {}
        SreOpcode::SUBPATTERN => {}
        SreOpcode::MIN_REPEAT_ONE => {}
        SreOpcode::GROUPREF_IGNORE => {}
        SreOpcode::IN_IGNORE => {}
        SreOpcode::LITERAL_IGNORE => {}
        SreOpcode::NOT_LITERAL_IGNORE => {}
        SreOpcode::GROUPREF_LOC_IGNORE => {}
        SreOpcode::IN_LOC_IGNORE => {}
        SreOpcode::LITERAL_LOC_IGNORE => {}
        SreOpcode::NOT_LITERAL_LOC_IGNORE => {}
        SreOpcode::GROUPREF_UNI_IGNORE => {}
        SreOpcode::IN_UNI_IGNORE => {}
        SreOpcode::LITERAL_UNI_IGNORE => {}
        SreOpcode::NOT_LITERAL_UNI_IGNORE => {}
        SreOpcode::RANGE_UNI_IGNORE => {}
    }
}

fn eq_loc_ignore(code: u32, c: char) -> bool {
    code == c as u32 || code == lower_locate(c) as u32 || code == upper_locate(c) as u32
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}
fn is_space(c: char) -> bool {
    c.is_ascii_whitespace()
}
fn is_digit(c: char) -> bool {
    c.is_ascii_digit()
}
fn is_loc_alnum(c: char) -> bool {
    // TODO: check with cpython
    c.is_alphanumeric()
}
fn is_loc_word(c: char) -> bool {
    is_loc_alnum(c) || c == '_'
}
fn is_linebreak(c: char) -> bool {
    c == '\n'
}
pub(crate) fn lower_ascii(c: char) -> char {
    c.to_ascii_lowercase()
}
fn lower_locate(c: char) -> char {
    // TODO: check with cpython
    // https://doc.rust-lang.org/std/primitive.char.html#method.to_lowercase
    c.to_lowercase().next().unwrap()
}
fn upper_locate(c: char) -> char {
    // TODO: check with cpython
    // https://doc.rust-lang.org/std/primitive.char.html#method.to_uppercase
    c.to_uppercase().next().unwrap()
}
fn is_uni_digit(c: char) -> bool {
    // TODO: check with cpython
    c.is_digit(10)
}
fn is_uni_space(c: char) -> bool {
    // TODO: check with cpython
    c.is_whitespace()
}
fn is_uni_linebreak(c: char) -> bool {
    match c {
        '\u{000A}' | '\u{000B}' | '\u{000C}' | '\u{000D}' | '\u{001C}' | '\u{001D}'
        | '\u{001E}' | '\u{0085}' | '\u{2028}' | '\u{2029}' => true,
        _ => false,
    }
}
fn is_uni_alnum(c: char) -> bool {
    // TODO: check with cpython
    c.is_alphanumeric()
}
fn is_uni_word(c: char) -> bool {
    is_uni_alnum(c) || c == '_'
}
pub(crate) fn lower_unicode(c: char) -> char {
    // TODO: check with cpython
    c.to_lowercase().next().unwrap()
}
pub(crate) fn upper_unicode(c: char) -> char {
    // TODO: check with cpython
    c.to_uppercase().next().unwrap()
}

fn is_utf8_first_byte(b: u8) -> bool {
    // In UTF-8, there are three kinds of byte...
    // 0xxxxxxx : ASCII
    // 10xxxxxx : 2nd, 3rd or 4th byte of code
    // 11xxxxxx : 1st byte of multibyte code
    (b & 0b10000000 == 0) || (b & 0b11000000 == 0b11000000)
}

struct OpAssert {
    child_ctx_id: usize,
    jump_id: usize,
}
impl Default for OpAssert {
    fn default() -> Self {
        OpAssert {
            child_ctx_id: 0,
            jump_id: 0,
        }
    }
}
impl OpcodeExecutor for OpAssert {
    fn next(&mut self, drive: &mut MatchContextDrive) -> Option<()> {
        match self.jump_id {
            0 => self._0(drive),
            1 => self._1(drive),
            _ => unreachable!(),
        }
    }
}
impl OpAssert {
    fn _0(&mut self, drive: &mut MatchContextDrive) -> Option<()> {
        let back = drive.peek_code(2) as usize;
        if back > drive.ctx().string_position {
            drive.ctx_mut().has_matched = Some(false);
            return None;
        }
        drive.state.string_position = drive.ctx().string_position - back;
        self.child_ctx_id = drive.push_new_context(3);
        self.jump_id = 1;
        Some(())
    }
    fn _1(&mut self, drive: &mut MatchContextDrive) -> Option<()> {
        if drive.state.context_stack[self.child_ctx_id].has_matched == Some(true) {
            drive.skip_code(drive.peek_code(1) as usize + 1);
        } else {
            drive.ctx_mut().has_matched = Some(false);
        }
        None
    }
}
