// good luck to those that follow; here be dragons

use super::_sre::MAXREPEAT;
use super::constants::{SreAtCode, SreCatCode, SreFlag, SreOpcode};
use crate::builtins::PyBytes;
use crate::bytesinner::is_py_ascii_whitespace;
use crate::pyobject::{IntoPyObject, PyObjectRef};
use crate::VirtualMachine;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::unreachable;

#[derive(Debug)]
pub(crate) struct State<'a> {
    pub string: StrDrive<'a>,
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
    popped_context: Option<MatchContext>,
    pub has_matched: Option<bool>,
}

impl<'a> State<'a> {
    pub(crate) fn new(
        string: StrDrive<'a>,
        start: usize,
        end: usize,
        flags: SreFlag,
        pattern_codes: &'a [u32],
    ) -> Self {
        let end = std::cmp::min(end, string.count());
        let start = std::cmp::min(start, end);
        Self {
            string,
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
            popped_context: None,
            has_matched: None,
        }
    }

    pub fn reset(&mut self) {
        self.lastindex = -1;
        self.marks_stack.clear();
        self.context_stack.clear();
        self.repeat_stack.clear();
        self.marks.clear();
        self.string_position = self.start;
        self.popped_context = None;
        self.has_matched = None;
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

    pub fn pymatch(mut self) -> Self {
        let ctx = MatchContext {
            string_position: self.start,
            string_offset: self.string.offset(0, self.start),
            code_position: 0,
            has_matched: None,
        };
        self.context_stack.push(ctx);

        let mut dispatcher = OpcodeDispatcher::new();
        let mut has_matched = None;

        loop {
            if self.context_stack.is_empty() {
                break;
            }
            let ctx_id = self.context_stack.len() - 1;
            let mut drive = StackDrive::drive(ctx_id, self);

            has_matched = dispatcher.pymatch(&mut drive);
            self = drive.take();
            if has_matched.is_some() {
                self.popped_context = self.context_stack.pop();
            }
        }

        self.has_matched = has_matched;
        self
    }

    pub fn search(mut self) -> Self {
        // TODO: optimize by op info and skip prefix
        while self.start <= self.end {
            self = self.pymatch();

            if self.has_matched == Some(true) {
                return self;
            }
            self.start += 1;
            self.reset();
        }

        self
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum StrDrive<'a> {
    Str(&'a str),
    Bytes(&'a [u8]),
}
impl<'a> StrDrive<'a> {
    fn offset(&self, offset: usize, skip: usize) -> usize {
        match *self {
            StrDrive::Str(s) => s
                .get(offset..)
                .and_then(|s| s.char_indices().nth(skip).map(|x| x.0 + offset))
                .unwrap_or_else(|| s.len()),
            StrDrive::Bytes(b) => std::cmp::min(offset + skip, b.len()),
        }
    }

    pub fn count(&self) -> usize {
        match *self {
            StrDrive::Str(s) => s.chars().count(),
            StrDrive::Bytes(b) => b.len(),
        }
    }

    fn peek(&self, offset: usize) -> u32 {
        match *self {
            StrDrive::Str(s) => unsafe { s.get_unchecked(offset..) }.chars().next().unwrap() as u32,
            StrDrive::Bytes(b) => b[offset] as u32,
        }
    }

    fn back_peek(&self, offset: usize) -> u32 {
        match *self {
            StrDrive::Str(s) => {
                let bytes = s.as_bytes();
                let back_offset = utf8_back_peek_offset(bytes, offset);
                match offset - back_offset {
                    1 => u32::from_ne_bytes([0, 0, 0, bytes[offset - 1]]),
                    2 => u32::from_ne_bytes([0, 0, bytes[offset - 2], bytes[offset - 1]]),
                    3 => u32::from_ne_bytes([
                        0,
                        bytes[offset - 3],
                        bytes[offset - 2],
                        bytes[offset - 1],
                    ]),
                    4 => u32::from_ne_bytes([
                        bytes[offset - 4],
                        bytes[offset - 3],
                        bytes[offset - 2],
                        bytes[offset - 1],
                    ]),
                    _ => unreachable!(),
                }
            }
            StrDrive::Bytes(b) => b[offset - 1] as u32,
        }
    }

    fn back_offset(&self, offset: usize, skip: usize) -> usize {
        match *self {
            StrDrive::Str(s) => {
                let bytes = s.as_bytes();
                let mut back_offset = offset;
                for _ in 0..skip {
                    back_offset = utf8_back_peek_offset(bytes, back_offset);
                }
                back_offset
            }
            StrDrive::Bytes(_) => offset - skip,
        }
    }

    pub fn slice_to_pyobject(&self, start: usize, end: usize, vm: &VirtualMachine) -> PyObjectRef {
        match *self {
            StrDrive::Str(s) => s
                .chars()
                .take(end)
                .skip(start)
                .collect::<String>()
                .into_pyobject(vm),
            StrDrive::Bytes(b) => {
                PyBytes::from(b.iter().take(end).skip(start).cloned().collect::<Vec<u8>>())
                    .into_pyobject(vm)
            }
        }
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
    fn repeat_ctx(&self) -> &RepeatContext {
        self.state().repeat_stack.last().unwrap()
    }
    fn pattern(&self) -> &[u32] {
        &self.state().pattern_codes[self.ctx().code_position..]
    }
    fn peek_char(&self) -> u32 {
        self.state().string.peek(self.ctx().string_offset)
    }
    fn peek_code(&self, peek: usize) -> u32 {
        self.state().pattern_codes[self.ctx().code_position + peek]
    }
    fn skip_char(&mut self, skip_count: usize) {
        self.ctx_mut().string_offset = self
            .state()
            .string
            .offset(self.ctx().string_offset, skip_count);
        self.ctx_mut().string_position =
            std::cmp::min(self.ctx().string_position + skip_count, self.state().end);
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
    fn at_boundary<F: FnMut(u32) -> bool>(&self, mut word_checker: F) -> bool {
        if self.at_beginning() && self.at_end() {
            return false;
        }
        let that = !self.at_beginning() && word_checker(self.back_peek_char());
        let this = !self.at_end() && word_checker(self.peek_char());
        this != that
    }
    fn back_peek_char(&self) -> u32 {
        self.state().string.back_peek(self.ctx().string_offset)
    }
    fn back_skip_char(&mut self, skip_count: usize) {
        self.ctx_mut().string_position -= skip_count;
        self.ctx_mut().string_offset = self
            .state()
            .string
            .back_offset(self.ctx().string_offset, skip_count);
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
    fn push_new_context(&mut self, pattern_offset: usize) {
        self.push_new_context_at(self.ctx().code_position + pattern_offset);
    }
    fn push_new_context_at(&mut self, code_position: usize) {
        let mut child_ctx = MatchContext { ..*self.ctx() };
        child_ctx.code_position = code_position;
        self.state.context_stack.push(child_ctx);
    }
    fn repeat_ctx_mut(&mut self) -> &mut RepeatContext {
        self.state.repeat_stack.last_mut().unwrap()
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

// F1 F2 are same identical, but workaround for closure
struct OpTwice<F1, F2> {
    f1: Option<F1>,
    f2: Option<F2>,
}
impl<F1, F2> OpcodeExecutor for OpTwice<F1, F2>
where
    F1: FnOnce(&mut StackDrive),
    F2: FnOnce(&mut StackDrive),
{
    fn next(&mut self, drive: &mut StackDrive) -> Option<()> {
        if let Some(f1) = self.f1.take() {
            f1(drive);
            Some(())
        } else if let Some(f2) = self.f2.take() {
            f2(drive);
            None
        } else {
            unreachable!()
        }
    }
}
fn twice<F1, F2>(f1: F1, f2: F2) -> Box<OpTwice<F1, F2>>
where
    F1: FnOnce(&mut StackDrive),
    F2: FnOnce(&mut StackDrive),
{
    Box::new(OpTwice {
        f1: Some(f1),
        f2: Some(f2),
    })
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
            SreOpcode::ASSERT => twice(
                |drive| {
                    let back = drive.peek_code(2) as usize;
                    if back > drive.ctx().string_position {
                        drive.ctx_mut().has_matched = Some(false);
                        return;
                    }
                    drive.state.string_position = drive.ctx().string_position - back;
                    drive.push_new_context(3);
                },
                |drive| {
                    let child_ctx = drive.state.popped_context.unwrap();
                    if child_ctx.has_matched == Some(true) {
                        drive.skip_code(drive.peek_code(1) as usize + 1);
                    } else {
                        drive.ctx_mut().has_matched = Some(false);
                    }
                },
            ),
            SreOpcode::ASSERT_NOT => twice(
                |drive| {
                    let back = drive.peek_code(2) as usize;
                    if back > drive.ctx().string_position {
                        drive.skip_code(drive.peek_code(1) as usize + 1);
                        return;
                    }
                    drive.state.string_position = drive.ctx().string_position - back;
                    drive.push_new_context(3);
                },
                |drive| {
                    let child_ctx = drive.state.popped_context.unwrap();
                    if child_ctx.has_matched == Some(true) {
                        drive.ctx_mut().has_matched = Some(false);
                    } else {
                        drive.skip_code(drive.peek_code(1) as usize + 1);
                    }
                },
            ),
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
                general_op_in(drive, |set, c| charset(set, c));
            }),
            SreOpcode::IN_IGNORE => once(|drive| {
                general_op_in(drive, |set, c| charset(set, lower_ascii(c)));
            }),
            SreOpcode::IN_UNI_IGNORE => once(|drive| {
                general_op_in(drive, |set, c| charset(set, lower_unicode(c)));
            }),
            SreOpcode::IN_LOC_IGNORE => once(|drive| {
                general_op_in(drive, |set, c| charset_loc_ignore(set, c));
            }),
            SreOpcode::INFO | SreOpcode::JUMP => once(|drive| {
                drive.skip_code(drive.peek_code(1) as usize + 1);
            }),
            SreOpcode::LITERAL => once(|drive| {
                general_op_literal(drive, |code, c| code == c);
            }),
            SreOpcode::NOT_LITERAL => once(|drive| {
                general_op_literal(drive, |code, c| code != c);
            }),
            SreOpcode::LITERAL_IGNORE => once(|drive| {
                general_op_literal(drive, |code, c| code == lower_ascii(c));
            }),
            SreOpcode::NOT_LITERAL_IGNORE => once(|drive| {
                general_op_literal(drive, |code, c| code != lower_ascii(c));
            }),
            SreOpcode::LITERAL_UNI_IGNORE => once(|drive| {
                general_op_literal(drive, |code, c| code == lower_unicode(c));
            }),
            SreOpcode::NOT_LITERAL_UNI_IGNORE => once(|drive| {
                general_op_literal(drive, |code, c| code != lower_unicode(c));
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
            SreOpcode::REPEAT => twice(
                // create repeat context.  all the hard work is done by the UNTIL
                // operator (MAX_UNTIL, MIN_UNTIL)
                // <REPEAT> <skip> <1=min> <2=max> item <UNTIL> tail
                |drive| {
                    let repeat = RepeatContext {
                        count: -1,
                        code_position: drive.ctx().code_position,
                        last_position: std::usize::MAX,
                        mincount: drive.peek_code(2) as usize,
                        maxcount: drive.peek_code(3) as usize,
                    };
                    drive.state.repeat_stack.push(repeat);
                    drive.state.string_position = drive.ctx().string_position;
                    // execute UNTIL operator
                    drive.push_new_context(drive.peek_code(1) as usize + 1);
                },
                |drive| {
                    drive.state.repeat_stack.pop();
                    let child_ctx = drive.state.popped_context.unwrap();
                    drive.ctx_mut().has_matched = child_ctx.has_matched;
                },
            ),
            SreOpcode::MAX_UNTIL => Box::new(OpMaxUntil::default()),
            SreOpcode::MIN_UNTIL => Box::new(OpMinUntil::default()),
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

fn char_loc_ignore(code: u32, c: u32) -> bool {
    code == c || code == lower_locate(c) || code == upper_locate(c)
}

fn charset_loc_ignore(set: &[u32], c: u32) -> bool {
    let lo = lower_locate(c);
    if charset(set, c) {
        return true;
    }
    let up = upper_locate(c);
    up != lo && charset(set, up)
}

fn general_op_groupref<F: FnMut(u32) -> u32>(drive: &mut StackDrive, mut f: F) {
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
            string_offset: drive.state.string.offset(0, group_start),
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

fn general_op_literal<F: FnOnce(u32, u32) -> bool>(drive: &mut StackDrive, f: F) {
    if drive.at_end() || !f(drive.peek_code(1), drive.peek_char()) {
        drive.ctx_mut().has_matched = Some(false);
    } else {
        drive.skip_code(2);
        drive.skip_char(1);
    }
}

fn general_op_in<F: FnOnce(&[u32], u32) -> bool>(drive: &mut StackDrive, f: F) {
    let skip = drive.peek_code(1) as usize;
    if drive.at_end() || !f(&drive.pattern()[2..], drive.peek_char()) {
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

fn category(catcode: SreCatCode, c: u32) -> bool {
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

fn charset(set: &[u32], ch: u32) -> bool {
    /* check if character is a member of the given set */
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
                if category(catcode, ch) {
                    return ok;
                }
                i += 2;
            }
            SreOpcode::CHARSET => {
                /* <CHARSET> <bitmap> */
                let set = &set[1..];
                if ch < 256 && ((set[(ch >> 5) as usize] & (1u32 << (ch & 31))) != 0) {
                    return ok;
                }
                i += 8;
            }
            SreOpcode::BIGCHARSET => {
                /* <BIGCHARSET> <blockcount> <256 blockindices> <blocks> */
                let count = set[i + 1] as usize;
                if ch < 0x10000 {
                    let set = &set[2..];
                    let block_index = ch >> 8;
                    let (_, blockindices, _) = unsafe { set.align_to::<u8>() };
                    let blocks = &set[64..];
                    let block = blockindices[block_index as usize];
                    if blocks[((block as u32 * 256 + (ch & 255)) / 32) as usize]
                        & (1u32 << (ch & 31))
                        != 0
                    {
                        return ok;
                    }
                }
                i += 2 + 64 + count * 8;
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
                let ch = upper_unicode(ch);
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

/* General case */
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

/* TODO: check literal cases should improve the perfermance

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

fn general_count_literal<F: FnMut(u32, u32) -> bool>(drive: &mut WrapDrive, end: usize, mut f: F) {
    let ch = drive.peek_code(1);
    while !drive.ctx().string_position < end && f(ch, drive.peek_char()) {
        drive.skip_char(1);
    }
}

fn eq_loc_ignore(code: u32, ch: u32) -> bool {
    code == ch || code == lower_locate(ch) || code == upper_locate(ch)
}
*/

fn is_word(ch: u32) -> bool {
    ch == '_' as u32
        || u8::try_from(ch)
            .map(|x| x.is_ascii_alphanumeric())
            .unwrap_or(false)
}
fn is_space(ch: u32) -> bool {
    u8::try_from(ch)
        .map(is_py_ascii_whitespace)
        .unwrap_or(false)
}
fn is_digit(ch: u32) -> bool {
    u8::try_from(ch)
        .map(|x| x.is_ascii_digit())
        .unwrap_or(false)
}
fn is_loc_alnum(ch: u32) -> bool {
    // TODO: check with cpython
    u8::try_from(ch)
        .map(|x| x.is_ascii_alphanumeric())
        .unwrap_or(false)
}
fn is_loc_word(ch: u32) -> bool {
    ch == '_' as u32 || is_loc_alnum(ch)
}
fn is_linebreak(ch: u32) -> bool {
    ch == '\n' as u32
}
pub(crate) fn lower_ascii(ch: u32) -> u32 {
    u8::try_from(ch)
        .map(|x| x.to_ascii_lowercase() as u32)
        .unwrap_or(ch)
}
fn lower_locate(ch: u32) -> u32 {
    // TODO: check with cpython
    // https://doc.rust-lang.org/std/primitive.char.html#method.to_lowercase
    lower_ascii(ch)
}
fn upper_locate(ch: u32) -> u32 {
    // TODO: check with cpython
    // https://doc.rust-lang.org/std/primitive.char.html#method.to_uppercase
    u8::try_from(ch)
        .map(|x| x.to_ascii_uppercase() as u32)
        .unwrap_or(ch)
}
fn is_uni_digit(ch: u32) -> bool {
    // TODO: check with cpython
    char::try_from(ch).map(|x| x.is_digit(10)).unwrap_or(false)
}
fn is_uni_space(ch: u32) -> bool {
    // TODO: check with cpython
    is_space(ch)
        || matches!(
            ch,
            0x0009
                | 0x000A
                | 0x000B
                | 0x000C
                | 0x000D
                | 0x001C
                | 0x001D
                | 0x001E
                | 0x001F
                | 0x0020
                | 0x0085
                | 0x00A0
                | 0x1680
                | 0x2000
                | 0x2001
                | 0x2002
                | 0x2003
                | 0x2004
                | 0x2005
                | 0x2006
                | 0x2007
                | 0x2008
                | 0x2009
                | 0x200A
                | 0x2028
                | 0x2029
                | 0x202F
                | 0x205F
                | 0x3000
        )
}
fn is_uni_linebreak(ch: u32) -> bool {
    matches!(
        ch,
        0x000A | 0x000B | 0x000C | 0x000D | 0x001C | 0x001D | 0x001E | 0x0085 | 0x2028 | 0x2029
    )
}
fn is_uni_alnum(ch: u32) -> bool {
    // TODO: check with cpython
    char::try_from(ch)
        .map(|x| x.is_alphanumeric())
        .unwrap_or(false)
}
fn is_uni_word(ch: u32) -> bool {
    ch == '_' as u32 || is_uni_alnum(ch)
}
pub(crate) fn lower_unicode(ch: u32) -> u32 {
    // TODO: check with cpython
    char::try_from(ch)
        .map(|x| x.to_lowercase().next().unwrap() as u32)
        .unwrap_or(ch)
}
pub(crate) fn upper_unicode(ch: u32) -> u32 {
    // TODO: check with cpython
    char::try_from(ch)
        .map(|x| x.to_uppercase().next().unwrap() as u32)
        .unwrap_or(ch)
}

fn is_utf8_first_byte(b: u8) -> bool {
    // In UTF-8, there are three kinds of byte...
    // 0xxxxxxx : ASCII
    // 10xxxxxx : 2nd, 3rd or 4th byte of code
    // 11xxxxxx : 1st byte of multibyte code
    (b & 0b10000000 == 0) || (b & 0b11000000 == 0b11000000)
}

fn utf8_back_peek_offset(bytes: &[u8], offset: usize) -> usize {
    let mut offset = offset - 1;
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

#[derive(Debug, Copy, Clone)]
struct RepeatContext {
    count: isize,
    code_position: usize,
    // zero-width match protection
    last_position: usize,
    mincount: usize,
    maxcount: usize,
}

#[derive(Default)]
struct OpMinRepeatOne {
    jump_id: usize,
    mincount: usize,
    maxcount: usize,
    count: usize,
}
impl OpcodeExecutor for OpMinRepeatOne {
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
                self.next(drive)
            }
            1 => {
                if self.maxcount == MAXREPEAT || self.count <= self.maxcount {
                    drive.state.string_position = drive.ctx().string_position;
                    drive.push_new_context(drive.peek_code(1) as usize + 1);
                    self.jump_id = 2;
                    return Some(());
                }

                drive.state.marks_pop_discard();
                drive.ctx_mut().has_matched = Some(false);
                None
            }
            2 => {
                let child_ctx = drive.state.popped_context.unwrap();
                if child_ctx.has_matched == Some(true) {
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
                self.next(drive)
            }
            _ => unreachable!(),
        }
    }
}

#[derive(Default)]
struct OpMaxUntil {
    jump_id: usize,
    count: isize,
    save_last_position: usize,
}
impl OpcodeExecutor for OpMaxUntil {
    fn next(&mut self, drive: &mut StackDrive) -> Option<()> {
        match self.jump_id {
            0 => {
                let RepeatContext {
                    count,
                    code_position,
                    last_position,
                    mincount,
                    maxcount,
                } = *drive.repeat_ctx();

                drive.state.string_position = drive.ctx().string_position;
                self.count = count + 1;

                if (self.count as usize) < mincount {
                    // not enough matches
                    drive.repeat_ctx_mut().count = self.count;
                    drive.push_new_context_at(code_position + 4);
                    self.jump_id = 1;
                    return Some(());
                }

                if ((self.count as usize) < maxcount || maxcount == MAXREPEAT)
                    && drive.state.string_position != last_position
                {
                    // we may have enough matches, if we can match another item, do so
                    drive.repeat_ctx_mut().count = self.count;
                    drive.state.marks_push();
                    self.save_last_position = last_position;
                    drive.repeat_ctx_mut().last_position = drive.state.string_position;
                    drive.push_new_context_at(code_position + 4);
                    self.jump_id = 2;
                    return Some(());
                }

                self.jump_id = 3;
                self.next(drive)
            }
            1 => {
                let child_ctx = drive.state.popped_context.unwrap();
                drive.ctx_mut().has_matched = child_ctx.has_matched;
                if drive.ctx().has_matched != Some(true) {
                    drive.repeat_ctx_mut().count = self.count - 1;
                    drive.state.string_position = drive.ctx().string_position;
                }
                None
            }
            2 => {
                drive.repeat_ctx_mut().last_position = self.save_last_position;
                let child_ctx = drive.state.popped_context.unwrap();
                if child_ctx.has_matched == Some(true) {
                    drive.state.marks_pop_discard();
                    drive.ctx_mut().has_matched = Some(true);
                    return None;
                }
                drive.state.marks_pop();
                drive.repeat_ctx_mut().count = self.count - 1;
                drive.state.string_position = drive.ctx().string_position;
                self.jump_id = 3;
                self.next(drive)
            }
            3 => {
                // cannot match more repeated items here.  make sure the tail matches
                drive.push_new_context(1);
                self.jump_id = 4;
                Some(())
            }
            4 => {
                let child_ctx = drive.state.popped_context.unwrap();
                drive.ctx_mut().has_matched = child_ctx.has_matched;
                if drive.ctx().has_matched != Some(true) {
                    drive.state.string_position = drive.ctx().string_position;
                }
                None
            }
            _ => unreachable!(),
        }
    }
}

#[derive(Default)]
struct OpMinUntil {
    jump_id: usize,
    count: isize,
    save_repeat: Option<RepeatContext>,
    save_last_position: usize,
}
impl OpcodeExecutor for OpMinUntil {
    fn next(&mut self, drive: &mut StackDrive) -> Option<()> {
        match self.jump_id {
            0 => {
                let RepeatContext {
                    count,
                    code_position,
                    last_position: _,
                    mincount,
                    maxcount: _,
                } = *drive.repeat_ctx();
                drive.state.string_position = drive.ctx().string_position;
                self.count = count + 1;

                if (self.count as usize) < mincount {
                    // not enough matches
                    drive.repeat_ctx_mut().count = self.count;
                    drive.push_new_context_at(code_position + 4);
                    self.jump_id = 1;
                    return Some(());
                }

                // see if the tail matches
                drive.state.marks_push();
                self.save_repeat = drive.state.repeat_stack.pop();
                drive.push_new_context(1);
                self.jump_id = 2;
                Some(())
            }
            1 => {
                let child_ctx = drive.state.popped_context.unwrap();
                drive.ctx_mut().has_matched = child_ctx.has_matched;
                if drive.ctx().has_matched != Some(true) {
                    drive.repeat_ctx_mut().count = self.count - 1;
                    drive.repeat_ctx_mut().last_position = self.save_last_position;
                    drive.state.string_position = drive.ctx().string_position;
                }
                None
            }
            2 => {
                let child_ctx = drive.state.popped_context.unwrap();
                if child_ctx.has_matched == Some(true) {
                    drive.ctx_mut().has_matched = Some(true);
                    return None;
                }
                drive.state.repeat_stack.push(self.save_repeat.unwrap());
                drive.state.string_position = drive.ctx().string_position;
                drive.state.marks_pop();

                // match more unital tail matches
                let RepeatContext {
                    count: _,
                    code_position,
                    last_position,
                    mincount: _,
                    maxcount,
                } = *drive.repeat_ctx();

                if self.count as usize >= maxcount && maxcount != MAXREPEAT
                    || drive.state.string_position == last_position
                {
                    drive.ctx_mut().has_matched = Some(false);
                    return None;
                }
                drive.repeat_ctx_mut().count = self.count;

                /* zero-width match protection */
                self.save_last_position = last_position;
                drive.repeat_ctx_mut().last_position = drive.state.string_position;

                drive.push_new_context_at(code_position + 4);
                self.jump_id = 1;
                Some(())
            }
            _ => unreachable!(),
        }
    }
}

#[derive(Default)]
struct OpBranch {
    jump_id: usize,
    current_branch_length: usize,
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
                drive.push_new_context(1);
                self.jump_id = 2;
                Some(())
            }
            2 => {
                let child_ctx = drive.state.popped_context.unwrap();
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

#[derive(Default)]
struct OpRepeatOne {
    jump_id: usize,
    mincount: usize,
    maxcount: usize,
    count: isize,
}
impl OpcodeExecutor for OpRepeatOne {
    fn next(&mut self, drive: &mut StackDrive) -> Option<()> {
        match self.jump_id {
            0 => {
                self.mincount = drive.peek_code(2) as usize;
                self.maxcount = drive.peek_code(3) as usize;

                if drive.remaining_chars() < self.mincount {
                    drive.ctx_mut().has_matched = Some(false);
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
                    drive.push_new_context(drive.peek_code(1) as usize + 1);
                    self.jump_id = 2;
                    return Some(());
                }

                drive.state.marks_pop_discard();
                drive.ctx_mut().has_matched = Some(false);
                None
            }
            2 => {
                let child_ctx = drive.state.popped_context.unwrap();
                if child_ctx.has_matched == Some(true) {
                    drive.ctx_mut().has_matched = Some(true);
                    return None;
                }
                if self.count <= self.mincount as isize {
                    drive.state.marks_pop_discard();
                    drive.ctx_mut().has_matched = Some(false);
                    return None;
                }

                // TODO: unnesscary double check
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
