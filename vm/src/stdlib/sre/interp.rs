// good luck to those that follow; here be dragons

use super::constants::{SreFlag, SreOpcode, SRE_MAXREPEAT};
use crate::builtins::PyStrRef;
use rustpython_common::borrow::BorrowValue;
use std::collections::HashMap;
use std::convert::TryFrom;

pub struct State<'a> {
    // py_string: PyStrRef,
    string: &'a str,
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
        // py_string: PyStrRef,
        string: &'a str,
        start: usize,
        end: usize,
        flags: SreFlag,
        pattern_codes: Vec<u32>,
    ) -> Self {
        // let string = py_string.borrow_value();
        Self {
            // py_string,
            string,
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
            std::str::from_utf8_unchecked(
                &self.state.string.as_bytes()[self.ctx().string_position..],
            )
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
        self.ctx_mut().string_position += skipped;
    }
    fn skip_code(&mut self, skip_count: usize) {
        self.ctx_mut().code_position += skip_count;
    }
    fn remaining_chars(&self) -> usize {
        let end = self.state.end;
        end - self.ctx().string_position + self.str().len()
    }
    fn remaining_codes(&self) -> usize {
        self.state.pattern_codes.len() - self.ctx().code_position
    }
    fn at_beginning(&self) -> bool {
        self.ctx().string_position == 0
    }
    fn at_end(&self) -> bool {
        self.str().is_empty()
    }
    fn at_linebreak(&self) -> bool {
        match self.str().chars().next() {
            Some(c) => c == '\n',
            None => false,
        }
    }
}

struct OpcodeDispatcher {
    executing_contexts: HashMap<usize, Box<dyn OpcodeExecutor>>,
}

macro_rules! once {
    ($val:expr) => {
        Box::new(OpEmpty {})
    };
}

trait OpcodeExecutor {
    fn next(&mut self, drive: &mut MatchContextDrive) -> Option<()>;
}

struct OpFailure {}
impl OpcodeExecutor for OpFailure {
    fn next(&mut self, drive: &mut MatchContextDrive) -> Option<()> {
        drive.ctx_mut().has_matched = Some(false);
        None
    }
}

struct OpEmpty {}
impl OpcodeExecutor for OpEmpty {
    fn next(&mut self, drive: &mut MatchContextDrive) -> Option<()> {
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

struct OpMinRepeatOne {
    trace_id: usize,
    mincount: usize,
    maxcount: usize,
    count: usize,
    child_ctx_id: usize,
}
impl OpcodeExecutor for OpMinRepeatOne {
    fn next(&mut self, drive: &mut MatchContextDrive) -> Option<()> {
        match self.trace_id {
            0 => self._0(drive),
            _ => unreachable!(),
        }
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
impl OpMinRepeatOne {
    fn _0(&mut self, drive: &mut MatchContextDrive) -> Option<()> {
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
            let count = count_repetitions(drive, self.mincount);
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

        // mark push
        self.trace_id = 1;
        self._1(drive)
    }
    fn _1(&mut self, drive: &mut MatchContextDrive) -> Option<()> {
        if self.maxcount == SRE_MAXREPEAT || self.count <= self.maxcount {
            drive.state.string_position = drive.ctx().string_position;
            self.child_ctx_id = drive.push_new_context(drive.peek_code(1) as usize + 1);
            self.trace_id = 2;
            return Some(());
        }

        // mark discard
        drive.ctx_mut().has_matched = Some(false);
        None
    }
    fn _2(&mut self, drive: &mut MatchContextDrive) -> Option<()> {
        if let Some(true) = drive.state.context_stack[self.child_ctx_id].has_matched {
            drive.ctx_mut().has_matched = Some(true);
            return None;
        }
        drive.state.string_position = drive.ctx().string_position;
        if count_repetitions(drive, 1) == 0 {
            self.trace_id = 3;
            return self._3(drive);
        }
        drive.skip_char(1);
        self.count += 1;
        // marks pop keep
        self.trace_id = 1;
        self._1(drive)
    }
    fn _3(&mut self, drive: &mut MatchContextDrive) -> Option<()> {
        // mark discard
        drive.ctx_mut().has_matched = Some(false);
        None
    }
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
            // self.drive = self.drive;
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
            Some((_, mut executor)) => executor,
            None => self.dispatch_table(opcode, drive),
        };
        if let Some(()) = executor.next(drive) {
            self.executing_contexts.insert(drive.id(), executor);
            false
        } else {
            true
        }
    }

    fn dispatch_table(
        &mut self,
        opcode: SreOpcode,
        drive: &mut MatchContextDrive,
    ) -> Box<dyn OpcodeExecutor> {
        // move || {
        match opcode {
            SreOpcode::FAILURE => {
                Box::new(OpFailure {})
            }
            SreOpcode::SUCCESS => once(|drive| {
                drive.state.string_position = drive.ctx().string_position;
                drive.ctx_mut().has_matched = Some(true);
            }),
            SreOpcode::ANY => once!(true),
            SreOpcode::ANY_ALL => once!(true),
            SreOpcode::ASSERT => once!(true),
            SreOpcode::ASSERT_NOT => once!(true),
            SreOpcode::AT => once!(true),
            SreOpcode::BRANCH => once!(true),
            SreOpcode::CALL => once!(true),
            SreOpcode::CATEGORY => once!(true),
            SreOpcode::CHARSET => once!(true),
            SreOpcode::BIGCHARSET => once!(true),
            SreOpcode::GROUPREF => once!(true),
            SreOpcode::GROUPREF_EXISTS => once!(true),
            SreOpcode::GROUPREF_IGNORE => once!(true),
            SreOpcode::IN => once!(true),
            SreOpcode::IN_IGNORE => once!(true),
            SreOpcode::INFO => once!(true),
            SreOpcode::JUMP => once!(true),
            SreOpcode::LITERAL => {
                if drive.at_end() || drive.peek_char() as u32 != drive.peek_code(1) {
                    drive.ctx_mut().has_matched = Some(false);
                } else {
                    drive.skip_char(1);
                }
                drive.skip_code(2);
                once!(true)
            }
            SreOpcode::LITERAL_IGNORE => once!(true),
            SreOpcode::MARK => once!(true),
            SreOpcode::MAX_UNTIL => once!(true),
            SreOpcode::MIN_UNTIL => once!(true),
            SreOpcode::NOT_LITERAL => once!(true),
            SreOpcode::NOT_LITERAL_IGNORE => once!(true),
            SreOpcode::NEGATE => once!(true),
            SreOpcode::RANGE => once!(true),
            SreOpcode::REPEAT => once!(true),
            SreOpcode::REPEAT_ONE => once!(true),
            SreOpcode::SUBPATTERN => once!(true),
            SreOpcode::MIN_REPEAT_ONE => Box::new(OpMinRepeatOne::default()),
            SreOpcode::RANGE_IGNORE => once!(true),
        }
    }
}

// Returns the number of repetitions of a single item, starting from the
// current string position. The code pointer is expected to point to a
// REPEAT_ONE operation (with the repeated 4 ahead).
fn count_repetitions(drive: &mut MatchContextDrive, maxcount: usize) -> usize {
    let mut count = 0;
    let mut real_maxcount = drive.state.end - drive.ctx().string_position;
    if maxcount < real_maxcount && maxcount != SRE_MAXREPEAT {
        real_maxcount = maxcount;
    }
    count
}
