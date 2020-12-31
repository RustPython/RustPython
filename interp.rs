// good luck to those that follow; here be dragons

use super::_sre::{Match, Pattern, MAXREPEAT};
use super::constants::{SreAtCode, SreCatCode, SreFlag, SreOpcode};
use crate::builtins::PyStrRef;
use crate::pyobject::PyRef;
use rustpython_common::borrow::BorrowValue;
use std::collections::HashMap;
use std::convert::TryFrom;

#[derive(Debug)]
pub(crate) struct State<'a> {
    string: &'a str,
    // chars count
    string_len: usize,
    pub start: usize,
    pub end: usize,
    flags: SreFlag,
    pattern_codes: &'a [u32],
    pub marks: Vec<Option<usize>>,
    pub lastindex: isize,
    marks_stack: Vec<(Vec<Option<usize>>, isize)>,
    context_stack: Vec<MatchContext>,
    repeat_stack: Vec<RepeatContext>,
    pub string_position: usize,
}

impl<'a> State<'a> {
    pub(crate) fn new(
        string: &'a str,
        start: usize,
        end: usize,
        flags: SreFlag,
        pattern_codes: &'a [u32],
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
            repeat_stack: Vec::new(),
            marks: Vec::new(),
            string_position: start,
        }
    }

    fn reset(&mut self) {
        self.marks.clear();
        self.lastindex = -1;
        self.marks_stack.clear();
        self.context_stack.clear();
        self.repeat_stack.clear();
    }

    fn set_mark(&mut self, mark_nr: usize, position: usize) {
        if mark_nr & 1 != 0 {
            self.lastindex = mark_nr as isize / 2 + 1;
        }
        if mark_nr >= self.marks.len() {
            self.marks.resize(mark_nr + 1, None);
        }
        self.marks[mark_nr] = Some(position);
    }
    fn get_marks(&self, group_index: usize) -> (Option<usize>, Option<usize>) {
        let marks_index = 2 * group_index;
        if marks_index + 1 < self.marks.len() {
            (self.marks[marks_index], self.marks[marks_index + 1])
        } else {
            (None, None)
        }
    }
    fn marks_push(&mut self) {
        self.marks_stack.push((self.marks.clone(), self.lastindex));
    }
    fn marks_pop(&mut self) {
        let (marks, lastindex) = self.marks_stack.pop().unwrap();
        self.marks = marks;
        self.lastindex = lastindex;
    }
    fn marks_pop_keep(&mut self) {
        let (marks, lastindex) = self.marks_stack.last().unwrap().clone();
        self.marks = marks;
        self.lastindex = lastindex;
    }
    fn marks_pop_discard(&mut self) {
        self.marks_stack.pop();
    }
}

pub(crate) fn pymatch(
    string: PyStrRef,
    start: usize,
    end: usize,
    pattern: PyRef<Pattern>,
) -> Option<Match> {
    let mut state = State::new(
        string.borrow_value(),
        start,
        end,
        pattern.flags,
        &pattern.code,
    );
    let ctx = MatchContext {
        string_position: state.start,
        string_offset: calc_string_offset(state.string, state.start),
        code_position: 0,
        has_matched: None,
    };
    state.context_stack.push(ctx);
    let mut dispatcher = OpcodeDispatcher::new();

    let mut has_matched = None;
    loop {
        if state.context_stack.is_empty() {
            break;
        }
        let ctx_id = state.context_stack.len() - 1;
        let mut drive = StackDrive::drive(ctx_id, state);

        has_matched = dispatcher.pymatch(&mut drive);
        state = drive.take();
        if has_matched.is_some() {
            state.context_stack.pop();
        }
    }

    if has_matched != Some(true) {
        None
    } else {
        Some(Match::new(&state, pattern.clone(), string.clone()))
    }
}

#[derive(Debug, Copy, Clone)]
struct MatchContext {
    string_position: usize,
    string_offset: usize,
    code_position: usize,
    has_matched: Option<bool>,
}

trait MatchContextDrive {
    fn ctx_mut(&mut self) -> &mut MatchContext;
    fn ctx(&self) -> &MatchContext;
    fn state(&self) -> &State;
    fn str(&self) -> &str {
        unsafe {
            std::str::from_utf8_unchecked(
                &self.state().string.as_bytes()[self.ctx().string_offset..],
            )
        }
    }
    fn pattern(&self) -> &[u32] {
        &self.state().pattern_codes[self.ctx().code_position..]
    }
    fn peek_char(&self) -> char {
        self.str().chars().next().unwrap()
    }
    fn peek_code(&self, peek: usize) -> u32 {
        self.state().pattern_codes[self.ctx().code_position + peek]
    }
    fn skip_char(&mut self, skip_count: usize) {
        match self.str().char_indices().nth(skip_count).map(|x| x.0) {
            Some(skipped) => {
                self.ctx_mut().string_position += skip_count;
                self.ctx_mut().string_offset += skipped;
            }
            None => {
                self.ctx_mut().string_position = self.state().end;
                self.ctx_mut().string_offset = self.state().string.len(); // bytes len
            }
        }
    }
    fn skip_code(&mut self, skip_count: usize) {
        self.ctx_mut().code_position += skip_count;
    }
    fn remaining_chars(&self) -> usize {
        self.state().end - self.ctx().string_position
    }
    fn remaining_codes(&self) -> usize {
        self.state().pattern_codes.len() - self.ctx().code_position
    }
    fn at_beginning(&self) -> bool {
        self.ctx().string_position == self.state().start
    }
    fn at_end(&self) -> bool {
        self.ctx().string_position == self.state().end
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
        let bytes = self.state().string.as_bytes();
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
        let bytes = self.state().string.as_bytes();
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
        // TODO: char::from_u32_unchecked is stable from 1.5.0
        unsafe { std::mem::transmute(code) }
    }
    fn back_skip_char(&mut self, skip_count: usize) {
        self.ctx_mut().string_position -= skip_count;
        for _ in 0..skip_count {
            self.ctx_mut().string_offset = self.back_peek_offset();
        }
    }
}

struct StackDrive<'a> {
    state: State<'a>,
    ctx_id: usize,
}
impl<'a> StackDrive<'a> {
    fn id(&self) -> usize {
        self.ctx_id
    }
    fn drive(ctx_id: usize, state: State<'a>) -> Self {
        Self { state, ctx_id }
    }
    fn take(self) -> State<'a> {
        self.state
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
}
impl MatchContextDrive for StackDrive<'_> {
    fn ctx_mut(&mut self) -> &mut MatchContext {
        &mut self.state.context_stack[self.ctx_id]
    }
    fn ctx(&self) -> &MatchContext {
        &self.state.context_stack[self.ctx_id]
    }
    fn state(&self) -> &State {
        &self.state
    }
}

struct WrapDrive<'a> {
    stack_drive: &'a StackDrive<'a>,
    ctx: MatchContext,
}
impl<'a> WrapDrive<'a> {
    fn drive(ctx: MatchContext, stack_drive: &'a StackDrive<'a>) -> Self {
        Self { stack_drive, ctx }
    }
}
impl MatchContextDrive for WrapDrive<'_> {
    fn ctx_mut(&mut self) -> &mut MatchContext {
        &mut self.ctx
    }

    fn ctx(&self) -> &MatchContext {
        &self.ctx
    }

    fn state(&self) -> &State {
        self.stack_drive.state()
    }
}

trait OpcodeExecutor {
    fn next(&mut self, drive: &mut StackDrive) -> Option<()>;
}

struct OpOnce<F> {
    f: Option<F>,
}
impl<F: FnOnce(&mut StackDrive)> OpcodeExecutor for OpOnce<F> {
    fn next(&mut self, drive: &mut StackDrive) -> Option<()> {
        let f = self.f.take()?;
        f(drive);
        None
    }
}
fn once<F: FnOnce(&mut StackDrive)>(f: F) -> Box<OpOnce<F>> {
    Box::new(OpOnce { f: Some(f) })
}

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
    fn pymatch(&mut self, drive: &mut StackDrive) -> Option<bool> {
        while drive.remaining_codes() > 0 && drive.ctx().has_matched.is_none() {
            let code = drive.peek_code(0);
            let opcode = SreOpcode::try_from(code).unwrap();
            if !self.dispatch(opcode, drive) {
                return None;
            }
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
    fn dispatch(&mut self, opcode: SreOpcode, drive: &mut StackDrive) -> bool {
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
            SreOpcode::ASSERT_NOT => Box::new(OpAssertNot::default()),
            SreOpcode::AT => once(|drive| {
                let atcode = SreAtCode::try_from(drive.peek_code(1)).unwrap();
                if !at(drive, atcode) {
                    drive.ctx_mut().has_matched = Some(false);
                } else {
                    drive.skip_code(2);
                }
            }),
            SreOpcode::BRANCH => Box::new(OpBranch::default()),
            SreOpcode::CATEGORY => once(|drive| {
                let catcode = SreCatCode::try_from(drive.peek_code(1)).unwrap();
                if drive.at_end() || !category(catcode, drive.peek_char()) {
                    drive.ctx_mut().has_matched = Some(false);
                } else {
                    drive.skip_code(2);
                    drive.skip_char(1);
                }
            }),
            SreOpcode::IN => once(|drive| {
                general_op_in(drive, |x| x);
            }),
            SreOpcode::IN_IGNORE => once(|drive| {
                general_op_in(drive, lower_ascii);
            }),
            SreOpcode::IN_UNI_IGNORE => once(|drive| {
                general_op_in(drive, lower_unicode);
            }),
            SreOpcode::IN_LOC_IGNORE => once(|drive| {
                let skip = drive.peek_code(1) as usize;
                if drive.at_end() || !charset_loc_ignore(&drive.pattern()[2..], drive.peek_char()) {
                    drive.ctx_mut().has_matched = Some(false);
                } else {
                    drive.skip_code(skip + 1);
                    drive.skip_char(1);
                }
            }),
            SreOpcode::INFO | SreOpcode::JUMP => once(|drive| {
                drive.skip_code(drive.peek_code(1) as usize + 1);
            }),
            SreOpcode::LITERAL => once(|drive| {
                general_op_literal(drive, |code, c| code == c as u32);
            }),
            SreOpcode::NOT_LITERAL => once(|drive| {
                general_op_literal(drive, |code, c| code != c as u32);
            }),
            SreOpcode::LITERAL_IGNORE => once(|drive| {
                general_op_literal(drive, |code, c| code == lower_ascii(c) as u32);
            }),
            SreOpcode::NOT_LITERAL_IGNORE => once(|drive| {
                general_op_literal(drive, |code, c| code != lower_ascii(c) as u32);
            }),
            SreOpcode::LITERAL_UNI_IGNORE => once(|drive| {
                general_op_literal(drive, |code, c| code == lower_unicode(c) as u32);
            }),
            SreOpcode::NOT_LITERAL_UNI_IGNORE => once(|drive| {
                general_op_literal(drive, |code, c| code != lower_unicode(c) as u32);
            }),
            SreOpcode::LITERAL_LOC_IGNORE => once(|drive| {
                general_op_literal(drive, char_loc_ignore);
            }),
            SreOpcode::NOT_LITERAL_LOC_IGNORE => once(|drive| {
                general_op_literal(drive, |code, c| !char_loc_ignore(code, c));
            }),
            SreOpcode::MARK => once(|drive| {
                drive
                    .state
                    .set_mark(drive.peek_code(1) as usize, drive.ctx().string_position);
                drive.skip_code(2);
            }),
            SreOpcode::MAX_UNTIL => Box::new(OpMaxUntil::default()),
            SreOpcode::MIN_UNTIL => Box::new(OpMinUntil::default()),
            SreOpcode::REPEAT => Box::new(OpRepeat::default()),
            SreOpcode::REPEAT_ONE => Box::new(OpRepeatOne::default()),
            SreOpcode::MIN_REPEAT_ONE => Box::new(OpMinRepeatOne::default()),
            SreOpcode::GROUPREF => once(|drive| general_op_groupref(drive, |x| x)),
            SreOpcode::GROUPREF_IGNORE => once(|drive| general_op_groupref(drive, lower_ascii)),
            SreOpcode::GROUPREF_LOC_IGNORE => {
                once(|drive| general_op_groupref(drive, lower_locate))
            }
            SreOpcode::GROUPREF_UNI_IGNORE => {
                once(|drive| general_op_groupref(drive, lower_unicode))
            }
            SreOpcode::GROUPREF_EXISTS => once(|drive| {
                let (group_start, group_end) = drive.state.get_marks(drive.peek_code(1) as usize);
                match (group_start, group_end) {
                    (Some(start), Some(end)) if start <= end => {
                        drive.skip_code(3);
                    }
                    _ => drive.skip_code(drive.peek_code(2) as usize + 1),
                }
            }),
            _ => {
                // TODO python expcetion?
                unreachable!("unexpected opcode")
            }
        }
    }
}

fn calc_string_offset(string: &str, position: usize) -> usize {
    string
        .char_indices()
        .nth(position)
        .map(|(i, _)| i)
        .unwrap_or(0)
}

fn char_loc_ignore(code: u32, c: char) -> bool {
    code == c as u32 || code == lower_locate(c) as u32 || code == upper_locate(c) as u32
}

fn charset_loc_ignore(set: &[u32], c: char) -> bool {
    let lo = lower_locate(c);
    if charset(set, c) {
        return true;
    }
    let up = upper_locate(c);
    up != lo && charset(set, up)
}

fn general_op_groupref<F: FnMut(char) -> char>(drive: &mut StackDrive, mut f: F) {
    let (group_start, group_end) = drive.state.get_marks(drive.peek_code(1) as usize);
    let (group_start, group_end) = match (group_start, group_end) {
        (Some(start), Some(end)) if start <= end => (start, end),
        _ => {
            drive.ctx_mut().has_matched = Some(false);
            return;
        }
    };
    let mut wdrive = WrapDrive::drive(*drive.ctx(), &drive);
    let mut gdrive = WrapDrive::drive(
        MatchContext {
            string_position: group_start,
            // TODO: cache the offset
            string_offset: calc_string_offset(drive.state.string, group_start),
            ..*drive.ctx()
        },
        &drive,
    );
    for _ in group_start..group_end {
        if wdrive.at_end() || f(wdrive.peek_char()) != f(gdrive.peek_char()) {
            drive.ctx_mut().has_matched = Some(false);
            return;
        }
        wdrive.skip_char(1);
        gdrive.skip_char(1);
    }
    let position = wdrive.ctx().string_position;
    let offset = wdrive.ctx().string_offset;
    drive.skip_code(2);
    drive.ctx_mut().string_position = position;
    drive.ctx_mut().string_offset = offset;
}

fn general_op_literal<F: FnOnce(u32, char) -> bool>(drive: &mut StackDrive, f: F) {
    if drive.at_end() || !f(drive.peek_code(1), drive.peek_char()) {
        drive.ctx_mut().has_matched = Some(false);
    } else {
        drive.skip_code(2);
        drive.skip_char(1);
    }
}

fn general_op_in<F: FnOnce(char) -> char>(drive: &mut StackDrive, f: F) {
    let skip = drive.peek_code(1) as usize;
    if drive.at_end() || !charset(&drive.pattern()[2..], f(drive.peek_char())) {
        drive.ctx_mut().has_matched = Some(false);
    } else {
        drive.skip_code(skip + 1);
        drive.skip_char(1);
    }
}

fn at(drive: &StackDrive, atcode: SreAtCode) -> bool {
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
                    let (_, blockindices, _) = unsafe { set[i + 2..].align_to::<u8>() };
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

fn count(drive: &mut StackDrive, maxcount: usize) -> usize {
    let mut count = 0;
    let maxcount = std::cmp::min(maxcount, drive.remaining_chars());

    let save_ctx = *drive.ctx();
    drive.skip_code(4);
    let reset_position = drive.ctx().code_position;

    let mut dispatcher = OpcodeDispatcher::new();
    while count < maxcount {
        drive.ctx_mut().code_position = reset_position;
        dispatcher.dispatch(SreOpcode::try_from(drive.peek_code(0)).unwrap(), drive);
        if drive.ctx().has_matched == Some(false) {
            break;
        }
        count += 1;
    }
    *drive.ctx_mut() = save_ctx;
    count
}

fn _count(stack_drive: &StackDrive, maxcount: usize) -> usize {
    let mut drive = WrapDrive::drive(*stack_drive.ctx(), stack_drive);
    let maxcount = std::cmp::min(maxcount, drive.remaining_chars());
    let end = drive.ctx().string_position + maxcount;
    let opcode = match SreOpcode::try_from(drive.peek_code(1)) {
        Ok(code) => code,
        Err(_) => {
            panic!("FIXME:COUNT1");
        }
    };

    match opcode {
        SreOpcode::ANY => {
            while !drive.ctx().string_position < end && !drive.at_linebreak() {
                drive.skip_char(1);
            }
        }
        SreOpcode::ANY_ALL => {
            drive.skip_char(maxcount);
        }
        SreOpcode::IN => {
            // TODO: pattern[2 or 1..]?
            while !drive.ctx().string_position < end
                && charset(&drive.pattern()[2..], drive.peek_char())
            {
                drive.skip_char(1);
            }
        }
        SreOpcode::LITERAL => {
            general_count_literal(&mut drive, end, |code, c| code == c as u32);
        }
        SreOpcode::NOT_LITERAL => {
            general_count_literal(&mut drive, end, |code, c| code != c as u32);
        }
        SreOpcode::LITERAL_IGNORE => {
            general_count_literal(&mut drive, end, |code, c| code == lower_ascii(c) as u32);
        }
        SreOpcode::NOT_LITERAL_IGNORE => {
            general_count_literal(&mut drive, end, |code, c| code != lower_ascii(c) as u32);
        }
        SreOpcode::LITERAL_LOC_IGNORE => {
            general_count_literal(&mut drive, end, char_loc_ignore);
        }
        SreOpcode::NOT_LITERAL_LOC_IGNORE => {
            general_count_literal(&mut drive, end, |code, c| !char_loc_ignore(code, c));
        }
        SreOpcode::LITERAL_UNI_IGNORE => {
            general_count_literal(&mut drive, end, |code, c| code == lower_unicode(c) as u32);
        }
        SreOpcode::NOT_LITERAL_UNI_IGNORE => {
            general_count_literal(&mut drive, end, |code, c| code != lower_unicode(c) as u32);
        }
        _ => {
            todo!("repeated single character pattern?");
        }
    }

    drive.ctx().string_position - drive.state().string_position
}

fn general_count_literal<F: FnMut(u32, char) -> bool>(drive: &mut WrapDrive, end: usize, mut f: F) {
    let ch = drive.peek_code(1);
    while !drive.ctx().string_position < end && f(ch, drive.peek_char()) {
        drive.skip_char(1);
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
    matches!(
        c,
        '\u{000A}'
            | '\u{000B}'
            | '\u{000C}'
            | '\u{000D}'
            | '\u{001C}'
            | '\u{001D}'
            | '\u{001E}'
            | '\u{0085}'
            | '\u{2028}'
            | '\u{2029}'
    )
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
        Self {
            child_ctx_id: 0,
            jump_id: 0,
        }
    }
}
impl OpcodeExecutor for OpAssert {
    fn next(&mut self, drive: &mut StackDrive) -> Option<()> {
        match self.jump_id {
            0 => self._0(drive),
            1 => self._1(drive),
            _ => unreachable!(),
        }
    }
}
impl OpAssert {
    fn _0(&mut self, drive: &mut StackDrive) -> Option<()> {
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
    fn _1(&mut self, drive: &mut StackDrive) -> Option<()> {
        if drive.state.context_stack[self.child_ctx_id].has_matched == Some(true) {
            drive.skip_code(drive.peek_code(1) as usize + 1);
        } else {
            drive.ctx_mut().has_matched = Some(false);
        }
        None
    }
}

struct OpAssertNot {
    child_ctx_id: usize,
    jump_id: usize,
}
impl Default for OpAssertNot {
    fn default() -> Self {
        Self {
            child_ctx_id: 0,
            jump_id: 0,
        }
    }
}
impl OpcodeExecutor for OpAssertNot {
    fn next(&mut self, drive: &mut StackDrive) -> Option<()> {
        match self.jump_id {
            0 => self._0(drive),
            1 => self._1(drive),
            _ => unreachable!(),
        }
    }
}
impl OpAssertNot {
    fn _0(&mut self, drive: &mut StackDrive) -> Option<()> {
        let back = drive.peek_code(2) as usize;
        if back > drive.ctx().string_position {
            drive.skip_code(drive.peek_code(1) as usize + 1);
            return None;
        }
        drive.state.string_position = drive.ctx().string_position - back;
        self.child_ctx_id = drive.push_new_context(3);
        self.jump_id = 1;
        Some(())
    }
    fn _1(&mut self, drive: &mut StackDrive) -> Option<()> {
        if drive.state.context_stack[self.child_ctx_id].has_matched == Some(true) {
            drive.ctx_mut().has_matched = Some(false);
        } else {
            drive.skip_code(drive.peek_code(1) as usize + 1);
        }
        None
    }
}

struct OpMinRepeatOne {
    jump_id: usize,
    mincount: usize,
    maxcount: usize,
    count: usize,
    child_ctx_id: usize,
}
impl OpcodeExecutor for OpMinRepeatOne {
    fn next(&mut self, drive: &mut StackDrive) -> Option<()> {
        match self.jump_id {
            0 => self._0(drive),
            1 => self._1(drive),
            2 => self._2(drive),
            _ => unreachable!(),
        }
    }
}
impl Default for OpMinRepeatOne {
    fn default() -> Self {
        OpMinRepeatOne {
            jump_id: 0,
            mincount: 0,
            maxcount: 0,
            count: 0,
            child_ctx_id: 0,
        }
    }
}
impl OpMinRepeatOne {
    fn _0(&mut self, drive: &mut StackDrive) -> Option<()> {
        self.mincount = drive.peek_code(2) as usize;
        self.maxcount = drive.peek_code(3) as usize;

        if drive.remaining_chars() < self.mincount {
            drive.ctx_mut().has_matched = Some(false);
            return None;
        }

        drive.state.string_position = drive.ctx().string_position;

        self.count = if self.mincount == 0 {
            0
        } else {
            let count = count(drive, self.mincount);
            if count < self.mincount {
                drive.ctx_mut().has_matched = Some(false);
                return None;
            }
            drive.skip_char(count);
            count
        };

        if drive.peek_code(drive.peek_code(1) as usize + 1) == SreOpcode::SUCCESS as u32 {
            drive.state.string_position = drive.ctx().string_position;
            drive.ctx_mut().has_matched = Some(true);
            return None;
        }

        drive.state.marks_push();
        self.jump_id = 1;
        self._1(drive)
    }
    fn _1(&mut self, drive: &mut StackDrive) -> Option<()> {
        if self.maxcount == MAXREPEAT || self.count <= self.maxcount {
            drive.state.string_position = drive.ctx().string_position;
            self.child_ctx_id = drive.push_new_context(drive.peek_code(1) as usize + 1);
            self.jump_id = 2;
            return Some(());
        }

        drive.state.marks_pop_discard();
        drive.ctx_mut().has_matched = Some(false);
        None
    }
    fn _2(&mut self, drive: &mut StackDrive) -> Option<()> {
        if let Some(true) = drive.state.context_stack[self.child_ctx_id].has_matched {
            drive.ctx_mut().has_matched = Some(true);
            return None;
        }
        drive.state.string_position = drive.ctx().string_position;
        if count(drive, 1) == 0 {
            drive.ctx_mut().has_matched = Some(false);
            return None;
        }
        drive.skip_char(1);
        self.count += 1;
        drive.state.marks_pop_keep();
        self.jump_id = 1;
        self._1(drive)
    }
}

#[derive(Debug, Copy, Clone)]
struct RepeatContext {
    skip: usize,
    mincount: usize,
    maxcount: usize,
    count: isize,
    last_position: isize,
}

struct OpMaxUntil {
    jump_id: usize,
    count: isize,
    save_last_position: isize,
    child_ctx_id: usize,
}
impl Default for OpMaxUntil {
    fn default() -> Self {
        Self {
            jump_id: 0,
            count: 0,
            save_last_position: -1,
            child_ctx_id: 0,
        }
    }
}
impl OpcodeExecutor for OpMaxUntil {
    fn next(&mut self, drive: &mut StackDrive) -> Option<()> {
        match self.jump_id {
            0 => {
                drive.state.string_position = drive.ctx().string_position;
                let repeat = match drive.state.repeat_stack.last_mut() {
                    Some(repeat) => repeat,
                    None => {
                        todo!("Internal re error: MAX_UNTIL without REPEAT.");
                    }
                };
                self.count = repeat.count + 1;

                if self.count < repeat.mincount as isize {
                    // not enough matches
                    repeat.count = self.count;
                    self.child_ctx_id = drive.push_new_context(4);
                    self.jump_id = 1;
                    return Some(());
                }

                if (self.count < repeat.maxcount as isize || repeat.maxcount == MAXREPEAT)
                    && (drive.state.string_position as isize != repeat.last_position)
                {
                    // we may have enough matches, if we can match another item, do so
                    repeat.count = self.count;
                    self.save_last_position = repeat.last_position;
                    repeat.last_position = drive.state.string_position as isize;
                    drive.state.marks_push();
                    self.child_ctx_id = drive.push_new_context(4);
                    self.jump_id = 2;
                    return Some(());
                }

                self.child_ctx_id = drive.push_new_context(1);

                self.jump_id = 3;
                Some(())
            }
            1 => {
                let child_ctx = &drive.state.context_stack[self.child_ctx_id];
                drive.ctx_mut().has_matched = child_ctx.has_matched;
                if drive.ctx().has_matched != Some(true) {
                    drive.state.string_position = drive.ctx().string_position;
                    let repeat = drive.state.repeat_stack.last_mut().unwrap();
                    repeat.count = self.count - 1;
                }
                None
            }
            2 => {
                let repeat = drive.state.repeat_stack.last_mut().unwrap();
                repeat.last_position = drive.state.string_position as isize;
                let child_ctx = &drive.state.context_stack[self.child_ctx_id];
                if child_ctx.has_matched == Some(true) {
                    drive.state.marks_pop_discard();
                    drive.ctx_mut().has_matched = Some(true);
                    return None;
                }
                repeat.count = self.count - 1;
                drive.state.marks_pop();
                drive.state.string_position = drive.ctx().string_position;

                self.child_ctx_id = drive.push_new_context(1);

                self.jump_id = 3;
                Some(())
            }
            3 => {
                // cannot match more repeated items here. make sure the tail matches
                let child_ctx = &drive.state.context_stack[self.child_ctx_id];
                drive.ctx_mut().has_matched = child_ctx.has_matched;
                if drive.ctx().has_matched != Some(true) {
                    drive.state.string_position = drive.ctx().string_position;
                } else {
                    drive.state.repeat_stack.pop();
                }
                None
            }
            _ => unreachable!(),
        }
    }
}

struct OpMinUntil {
    jump_id: usize,
    count: isize,
    child_ctx_id: usize,
}
impl Default for OpMinUntil {
    fn default() -> Self {
        Self {
            jump_id: 0,
            count: 0,
            child_ctx_id: 0,
        }
    }
}
impl OpcodeExecutor for OpMinUntil {
    fn next(&mut self, drive: &mut StackDrive) -> Option<()> {
        match self.jump_id {
            0 => {
                drive.state.string_position = drive.ctx().string_position;
                let repeat = match drive.state.repeat_stack.last_mut() {
                    Some(repeat) => repeat,
                    None => {
                        todo!("Internal re error: MAX_UNTIL without REPEAT.");
                    }
                };
                self.count = repeat.count + 1;

                if self.count < repeat.mincount as isize {
                    // not enough matches
                    repeat.count = self.count;
                    self.child_ctx_id = drive.push_new_context(4);
                    self.jump_id = 1;
                    return Some(());
                }

                // see if the tail matches
                drive.state.marks_push();
                self.child_ctx_id = drive.push_new_context(1);
                self.jump_id = 2;
                Some(())
            }
            1 => {
                let child_ctx = &drive.state.context_stack[self.child_ctx_id];
                drive.ctx_mut().has_matched = child_ctx.has_matched;
                if drive.ctx().has_matched != Some(true) {
                    drive.state.string_position = drive.ctx().string_position;
                    let repeat = drive.state.repeat_stack.last_mut().unwrap();
                    repeat.count = self.count - 1;
                }
                None
            }
            2 => {
                let child_ctx = &drive.state.context_stack[self.child_ctx_id];
                if child_ctx.has_matched == Some(true) {
                    drive.state.repeat_stack.pop();
                    drive.ctx_mut().has_matched = Some(true);
                    return None;
                }
                drive.state.string_position = drive.ctx().string_position;
                drive.state.marks_pop();

                // match more until tail matches
                let repeat = drive.state.repeat_stack.last_mut().unwrap();
                if self.count >= repeat.maxcount as isize && repeat.maxcount != MAXREPEAT {
                    drive.ctx_mut().has_matched = Some(false);
                    return None;
                }
                repeat.count = self.count;
                self.child_ctx_id = drive.push_new_context(4);
                self.jump_id = 1;
                Some(())
            }
            _ => unreachable!(),
        }
    }
}

struct OpBranch {
    jump_id: usize,
    child_ctx_id: usize,
    current_branch_length: usize,
}
impl Default for OpBranch {
    fn default() -> Self {
        Self {
            jump_id: 0,
            child_ctx_id: 0,
            current_branch_length: 0,
        }
    }
}
impl OpcodeExecutor for OpBranch {
    fn next(&mut self, drive: &mut StackDrive) -> Option<()> {
        match self.jump_id {
            0 => {
                drive.state.marks_push();
                // jump out the head
                self.current_branch_length = 1;
                self.jump_id = 1;
                self.next(drive)
            }
            1 => {
                drive.skip_code(self.current_branch_length);
                self.current_branch_length = drive.peek_code(0) as usize;
                if self.current_branch_length == 0 {
                    drive.state.marks_pop_discard();
                    drive.ctx_mut().has_matched = Some(false);
                    return None;
                }
                drive.state.string_position = drive.ctx().string_position;
                self.child_ctx_id = drive.push_new_context(1);
                self.jump_id = 2;
                Some(())
            }
            2 => {
                let child_ctx = &drive.state.context_stack[self.child_ctx_id];
                if child_ctx.has_matched == Some(true) {
                    drive.ctx_mut().has_matched = Some(true);
                    return None;
                }
                drive.state.marks_pop_keep();
                self.jump_id = 1;
                Some(())
            }
            _ => unreachable!(),
        }
    }
}

struct OpRepeat {
    jump_id: usize,
    child_ctx_id: usize,
}
impl Default for OpRepeat {
    fn default() -> Self {
        Self {
            jump_id: 0,
            child_ctx_id: 0,
        }
    }
}
impl OpcodeExecutor for OpRepeat {
    fn next(&mut self, drive: &mut StackDrive) -> Option<()> {
        match self.jump_id {
            0 => {
                let repeat = RepeatContext {
                    skip: drive.peek_code(1) as usize,
                    mincount: drive.peek_code(2) as usize,
                    maxcount: drive.peek_code(3) as usize,
                    count: -1,
                    last_position: -1,
                };
                drive.state.repeat_stack.push(repeat);
                drive.state.string_position = drive.ctx().string_position;
                self.child_ctx_id = drive.push_new_context(drive.peek_code(1) as usize + 1);
                self.jump_id = 1;
                Some(())
            }
            1 => {
                let child_ctx = &drive.state.context_stack[self.child_ctx_id];
                drive.ctx_mut().has_matched = child_ctx.has_matched;
                None
            }
            _ => unreachable!(),
        }
    }
}

struct OpRepeatOne {
    jump_id: usize,
    child_ctx_id: usize,
    mincount: usize,
    maxcount: usize,
    count: isize,
}
impl Default for OpRepeatOne {
    fn default() -> Self {
        Self {
            jump_id: 0,
            child_ctx_id: 0,
            mincount: 0,
            maxcount: 0,
            count: 0,
        }
    }
}
impl OpcodeExecutor for OpRepeatOne {
    fn next(&mut self, drive: &mut StackDrive) -> Option<()> {
        match self.jump_id {
            0 => {
                self.mincount = drive.peek_code(2) as usize;
                self.maxcount = drive.peek_code(3) as usize;

                if drive.remaining_chars() < self.mincount {
                    drive.ctx_mut().has_matched = Some(false);
                    return None;
                }
                drive.state.string_position = drive.ctx().string_position;
                self.count = count(drive, self.maxcount) as isize;
                drive.skip_char(self.count as usize);
                if self.count < self.mincount as isize {
                    drive.ctx_mut().has_matched = Some(false);
                    return None;
                }

                let next_code = drive.peek_code(drive.peek_code(1) as usize + 1);
                if next_code == SreOpcode::SUCCESS as u32 {
                    // tail is empty.  we're finished
                    drive.state.string_position = drive.ctx().string_position;
                    drive.ctx_mut().has_matched = Some(true);
                    return None;
                }

                drive.state.marks_push();
                // TODO:
                // Special case: Tail starts with a literal. Skip positions where
                // the rest of the pattern cannot possibly match.
                self.jump_id = 1;
                self.next(drive)
            }
            1 => {
                // General case: backtracking
                if self.count >= self.mincount as isize {
                    drive.state.string_position = drive.ctx().string_position;
                    self.child_ctx_id = drive.push_new_context(drive.peek_code(1) as usize + 1);
                    self.jump_id = 2;
                    return Some(());
                }

                drive.state.marks_pop_discard();
                drive.ctx_mut().has_matched = Some(false);
                None
            }
            2 => {
                let child_ctx = &drive.state.context_stack[self.child_ctx_id];
                if child_ctx.has_matched == Some(true) {
                    drive.ctx_mut().has_matched = Some(true);
                    return None;
                }
                drive.back_skip_char(1);
                self.count -= 1;
                drive.state.marks_pop_keep();

                self.jump_id = 1;
                Some(())
            }
            _ => unreachable!(),
        }
    }
}
