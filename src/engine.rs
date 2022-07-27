// good luck to those that follow; here be dragons

use super::constants::{SreAtCode, SreCatCode, SreFlag, SreOpcode};
use super::MAXREPEAT;
use std::convert::TryFrom;

const fn is_py_ascii_whitespace(b: u8) -> bool {
    matches!(b, b'\t' | b'\n' | b'\x0C' | b'\r' | b' ' | b'\x0B')
}

#[derive(Debug)]
pub struct State<'a> {
    pub string: StrDrive<'a>,
    pub start: usize,
    pub end: usize,
    _flags: SreFlag,
    pattern_codes: &'a [u32],
    pub marks: Vec<Option<usize>>,
    pub lastindex: isize,
    marks_stack: Vec<(Vec<Option<usize>>, isize)>,
    context_stack: Vec<MatchContext>,
    _stacks: Option<Box<Stacks>>,
    pub string_position: usize,
    popped_context: Option<MatchContext>,
    pub has_matched: bool,
    pub match_all: bool,
    pub must_advance: bool,
}

impl<'a> State<'a> {
    pub fn new(
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
            _flags: flags,
            pattern_codes,
            marks: Vec::new(),
            lastindex: -1,
            marks_stack: Vec::new(),
            context_stack: Vec::new(),
            _stacks: Default::default(),
            string_position: start,
            popped_context: None,
            has_matched: false,
            match_all: false,
            must_advance: false,
        }
    }

    pub fn reset(&mut self) {
        self.lastindex = -1;
        self.marks.clear();
        self.marks_stack.clear();
        self.context_stack.clear();
        if let Some(x) = self._stacks.as_mut() {
            x.clear()
        };
        self.string_position = self.start;
        self.popped_context = None;
        self.has_matched = false;
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

    fn _match(mut self, stacks: &mut Stacks) -> Self {
        while let Some(ctx) = self.context_stack.pop() {
            let mut drive = StateContext {
                state: self,
                ctx,
                next_ctx: None,
            };

            if let Some(handler) = drive.ctx.handler {
                handler(&mut drive, stacks);
            } else if drive.remaining_codes() > 0 {
                let code = drive.peek_code(0);
                let code = SreOpcode::try_from(code).unwrap();
                dispatch(code, &mut drive, stacks);
            } else {
                drive.failure();
            }

            let StateContext {
                mut state,
                ctx,
                next_ctx,
            } = drive;

            if ctx.has_matched.is_some() {
                state.popped_context = Some(ctx);
            } else {
                state.context_stack.push(ctx);
                if let Some(next_ctx) = next_ctx {
                    state.context_stack.push(next_ctx);
                }
            }
            self = state
        }
        self.has_matched = self.popped_context.unwrap().has_matched == Some(true);
        self
    }

    pub fn pymatch(mut self) -> Self {
        let mut stacks = self._stacks.take().unwrap_or_default();

        let ctx = MatchContext {
            string_position: self.start,
            string_offset: self.string.offset(0, self.start),
            code_position: 0,
            has_matched: None,
            toplevel: true,
            handler: None,
            repeat_ctx_id: usize::MAX,
        };
        self.context_stack.push(ctx);

        self = self._match(&mut stacks);
        self._stacks = Some(stacks);
        self
    }

    pub fn search(mut self) -> Self {
        let mut stacks = self._stacks.take().unwrap_or_default();
        // TODO: optimize by op info and skip prefix

        if self.start > self.end {
            return self;
        }

        let mut start_offset = self.string.offset(0, self.start);

        let ctx = MatchContext {
            string_position: self.start,
            string_offset: start_offset,
            code_position: 0,
            has_matched: None,
            toplevel: true,
            handler: None,
            repeat_ctx_id: usize::MAX,
        };
        self.context_stack.push(ctx);
        self = self._match(&mut stacks);

        self.must_advance = false;
        while !self.has_matched && self.start < self.end {
            self.start += 1;
            start_offset = self.string.offset(start_offset, 1);
            self.reset();
            stacks.clear();

            let ctx = MatchContext {
                string_position: self.start,
                string_offset: start_offset,
                code_position: 0,
                has_matched: None,
                toplevel: false,
                handler: None,
                repeat_ctx_id: usize::MAX,
            };
            self.context_stack.push(ctx);
            self = self._match(&mut stacks);
        }

        self._stacks = Some(stacks);
        self
    }
}

fn dispatch(opcode: SreOpcode, drive: &mut StateContext, stacks: &mut Stacks) {
    match opcode {
        SreOpcode::FAILURE => {
            drive.failure();
        }
        SreOpcode::SUCCESS => {
            drive.ctx.has_matched = Some(drive.can_success());
            if drive.ctx.has_matched == Some(true) {
                drive.state.string_position = drive.ctx.string_position;
            }
        }
        SreOpcode::ANY => {
            if drive.at_end() || drive.at_linebreak() {
                drive.failure();
            } else {
                drive.skip_code(1);
                drive.skip_char(1);
            }
        }
        SreOpcode::ANY_ALL => {
            if drive.at_end() {
                drive.failure();
            } else {
                drive.skip_code(1);
                drive.skip_char(1);
            }
        }
        SreOpcode::ASSERT => op_assert(drive),
        SreOpcode::ASSERT_NOT => op_assert_not(drive),
        SreOpcode::AT => {
            let atcode = SreAtCode::try_from(drive.peek_code(1)).unwrap();
            if at(drive, atcode) {
                drive.skip_code(2);
            } else {
                drive.failure();
            }
        }
        SreOpcode::BRANCH => op_branch(drive, stacks),
        SreOpcode::CATEGORY => {
            let catcode = SreCatCode::try_from(drive.peek_code(1)).unwrap();
            if drive.at_end() || !category(catcode, drive.peek_char()) {
                drive.failure();
            } else {
                drive.skip_code(2);
                drive.skip_char(1);
            }
        }
        SreOpcode::IN => general_op_in(drive, charset),
        SreOpcode::IN_IGNORE => general_op_in(drive, |set, c| charset(set, lower_ascii(c))),
        SreOpcode::IN_UNI_IGNORE => general_op_in(drive, |set, c| charset(set, lower_unicode(c))),
        SreOpcode::IN_LOC_IGNORE => general_op_in(drive, charset_loc_ignore),
        SreOpcode::INFO | SreOpcode::JUMP => drive.skip_code_from(1),
        SreOpcode::LITERAL => general_op_literal(drive, |code, c| code == c),
        SreOpcode::NOT_LITERAL => general_op_literal(drive, |code, c| code != c),
        SreOpcode::LITERAL_IGNORE => general_op_literal(drive, |code, c| code == lower_ascii(c)),
        SreOpcode::NOT_LITERAL_IGNORE => {
            general_op_literal(drive, |code, c| code != lower_ascii(c))
        }
        SreOpcode::LITERAL_UNI_IGNORE => {
            general_op_literal(drive, |code, c| code == lower_unicode(c))
        }
        SreOpcode::NOT_LITERAL_UNI_IGNORE => {
            general_op_literal(drive, |code, c| code != lower_unicode(c))
        }
        SreOpcode::LITERAL_LOC_IGNORE => general_op_literal(drive, char_loc_ignore),
        SreOpcode::NOT_LITERAL_LOC_IGNORE => {
            general_op_literal(drive, |code, c| !char_loc_ignore(code, c))
        }
        SreOpcode::MARK => {
            drive
                .state
                .set_mark(drive.peek_code(1) as usize, drive.ctx.string_position);
            drive.skip_code(2);
        }
        SreOpcode::MAX_UNTIL => op_max_until(drive, stacks),
        SreOpcode::MIN_UNTIL => op_min_until(drive, stacks),
        SreOpcode::REPEAT => op_repeat(drive, stacks),
        SreOpcode::REPEAT_ONE => op_repeat_one(drive, stacks),
        SreOpcode::MIN_REPEAT_ONE => op_min_repeat_one(drive, stacks),
        SreOpcode::GROUPREF => general_op_groupref(drive, |x| x),
        SreOpcode::GROUPREF_IGNORE => general_op_groupref(drive, lower_ascii),
        SreOpcode::GROUPREF_LOC_IGNORE => general_op_groupref(drive, lower_locate),
        SreOpcode::GROUPREF_UNI_IGNORE => general_op_groupref(drive, lower_unicode),
        SreOpcode::GROUPREF_EXISTS => {
            let (group_start, group_end) = drive.state.get_marks(drive.peek_code(1) as usize);
            match (group_start, group_end) {
                (Some(start), Some(end)) if start <= end => {
                    drive.skip_code(3);
                }
                _ => drive.skip_code_from(2),
            }
        }
        _ => unreachable!("unexpected opcode"),
    }
}

/* assert subpattern */
/* <ASSERT> <skip> <back> <pattern> */
fn op_assert(drive: &mut StateContext) {
    let back = drive.peek_code(2) as usize;

    if drive.ctx.string_position < back {
        return drive.failure();
    }

    let offset = drive
        .state
        .string
        .back_offset(drive.ctx.string_offset, back);
    let position = drive.ctx.string_position - back;

    drive.state.string_position = position;

    let next_ctx = drive.next_ctx(3, |drive, _| {
        if drive.popped_ctx().has_matched == Some(true) {
            drive.ctx.handler = None;
            drive.skip_code_from(1);
        } else {
            drive.failure();
        }
    });
    next_ctx.string_position = position;
    next_ctx.string_offset = offset;
    next_ctx.toplevel = false;
}

/* assert not subpattern */
/* <ASSERT_NOT> <skip> <back> <pattern> */
fn op_assert_not(drive: &mut StateContext) {
    let back = drive.peek_code(2) as usize;

    if drive.ctx.string_position < back {
        return drive.skip_code_from(1);
    }

    let offset = drive
        .state
        .string
        .back_offset(drive.ctx.string_offset, back);
    let position = drive.ctx.string_position - back;

    drive.state.string_position = position;

    let next_ctx = drive.next_ctx(3, |drive, _| {
        if drive.popped_ctx().has_matched == Some(true) {
            drive.failure();
        } else {
            drive.ctx.handler = None;
            drive.skip_code_from(1);
        }
    });
    next_ctx.string_position = position;
    next_ctx.string_offset = offset;
    next_ctx.toplevel = false;
}

#[derive(Debug)]
struct BranchContext {
    branch_offset: usize,
}

// alternation
// <BRANCH> <0=skip> code <JUMP> ... <NULL>
fn op_branch(drive: &mut StateContext, stacks: &mut Stacks) {
    drive.state.marks_push();
    stacks.branch.push(BranchContext { branch_offset: 1 });
    create_context(drive, stacks);

    fn create_context(drive: &mut StateContext, stacks: &mut Stacks) {
        let branch_offset = stacks.branch_last().branch_offset;
        let next_length = drive.peek_code(branch_offset) as usize;
        if next_length == 0 {
            drive.state.marks_pop_discard();
            stacks.branch.pop();
            return drive.failure();
        }

        drive.sync_string_position();

        stacks.branch_last().branch_offset += next_length;
        drive.next_ctx(branch_offset + 1, callback);
    }

    fn callback(drive: &mut StateContext, stacks: &mut Stacks) {
        if drive.popped_ctx().has_matched == Some(true) {
            stacks.branch.pop();
            return drive.success();
        }
        drive.state.marks_pop_keep();
        drive.ctx.handler = Some(create_context)
    }
}

#[derive(Debug, Copy, Clone)]
struct MinRepeatOneContext {
    count: usize,
    max_count: usize,
}

/* <MIN_REPEAT_ONE> <skip> <1=min> <2=max> item <SUCCESS> tail */
fn op_min_repeat_one(drive: &mut StateContext, stacks: &mut Stacks) {
    let min_count = drive.peek_code(2) as usize;
    let max_count = drive.peek_code(3) as usize;

    if drive.remaining_chars() < min_count {
        return drive.failure();
    }

    drive.sync_string_position();

    let count = if min_count == 0 {
        0
    } else {
        let count = _count(drive, stacks, min_count);
        if count < min_count {
            return drive.failure();
        }
        drive.skip_char(count);
        count
    };

    let next_code = drive.peek_code(drive.peek_code(1) as usize + 1);
    if next_code == SreOpcode::SUCCESS as u32 && drive.can_success() {
        // tail is empty. we're finished
        drive.sync_string_position();
        return drive.success();
    }

    drive.state.marks_push();
    stacks
        .min_repeat_one
        .push(MinRepeatOneContext { count, max_count });
    create_context(drive, stacks);

    fn create_context(drive: &mut StateContext, stacks: &mut Stacks) {
        let MinRepeatOneContext { count, max_count } = *stacks.min_repeat_one_last();

        if max_count == MAXREPEAT || count <= max_count {
            drive.sync_string_position();
            drive.next_ctx_from(1, callback);
        } else {
            drive.state.marks_pop_discard();
            stacks.min_repeat_one.pop();
            drive.failure();
        }
    }

    fn callback(drive: &mut StateContext, stacks: &mut Stacks) {
        if drive.popped_ctx().has_matched == Some(true) {
            stacks.min_repeat_one.pop();
            return drive.success();
        }

        drive.sync_string_position();

        if _count(drive, stacks, 1) == 0 {
            drive.state.marks_pop_discard();
            stacks.min_repeat_one.pop();
            return drive.failure();
        }

        drive.skip_char(1);
        stacks.min_repeat_one_last().count += 1;
        drive.state.marks_pop_keep();
        create_context(drive, stacks);
    }
}

#[derive(Debug, Copy, Clone)]
struct RepeatOneContext {
    count: usize,
    min_count: usize,
    following_literal: Option<u32>,
}

/* match repeated sequence (maximizing regexp) */

/* this operator only works if the repeated item is
exactly one character wide, and we're not already
collecting backtracking points.  for other cases,
use the MAX_REPEAT operator */

/* <REPEAT_ONE> <skip> <1=min> <2=max> item <SUCCESS> tail */
fn op_repeat_one(drive: &mut StateContext, stacks: &mut Stacks) {
    let min_count = drive.peek_code(2) as usize;
    let max_count = drive.peek_code(3) as usize;

    if drive.remaining_chars() < min_count {
        return drive.failure();
    }

    drive.sync_string_position();

    let count = _count(drive, stacks, max_count);
    drive.skip_char(count);
    if count < min_count {
        return drive.failure();
    }

    let next_code = drive.peek_code(drive.peek_code(1) as usize + 1);
    if next_code == SreOpcode::SUCCESS as u32 && drive.can_success() {
        // tail is empty. we're finished
        drive.sync_string_position();
        return drive.success();
    }

    // Special case: Tail starts with a literal. Skip positions where
    // the rest of the pattern cannot possibly match.
    let following_literal = (next_code == SreOpcode::LITERAL as u32)
        .then(|| drive.peek_code(drive.peek_code(1) as usize + 2));

    drive.state.marks_push();
    stacks.repeat_one.push(RepeatOneContext {
        count,
        min_count,
        following_literal,
    });
    create_context(drive, stacks);

    fn create_context(drive: &mut StateContext, stacks: &mut Stacks) {
        let RepeatOneContext {
            mut count,
            min_count,
            following_literal,
        } = *stacks.repeat_one_last();

        if let Some(c) = following_literal {
            while drive.at_end() || drive.peek_char() != c {
                if count <= min_count {
                    drive.state.marks_pop_discard();
                    stacks.repeat_one.pop();
                    return drive.failure();
                }
                drive.back_skip_char(1);
                count -= 1;
            }
        }
        stacks.repeat_one_last().count = count;

        drive.sync_string_position();

        // General case: backtracking
        drive.next_ctx_from(1, callback);
    }

    fn callback(drive: &mut StateContext, stacks: &mut Stacks) {
        if drive.popped_ctx().has_matched == Some(true) {
            stacks.repeat_one.pop();
            return drive.success();
        }

        let RepeatOneContext {
            count,
            min_count,
            following_literal: _,
        } = stacks.repeat_one_last();

        if count <= min_count {
            drive.state.marks_pop_discard();
            stacks.repeat_one.pop();
            return drive.failure();
        }

        drive.back_skip_char(1);
        *count -= 1;

        drive.state.marks_pop_keep();
        create_context(drive, stacks);
    }
}

#[derive(Debug, Clone, Copy)]
struct RepeatContext {
    count: isize,
    min_count: usize,
    max_count: usize,
    code_position: usize,
    last_position: usize,
    prev_id: usize,
}

/* create repeat context.  all the hard work is done
by the UNTIL operator (MAX_UNTIL, MIN_UNTIL) */
/* <REPEAT> <skip> <1=min> <2=max> item <UNTIL> tail */
fn op_repeat(drive: &mut StateContext, stacks: &mut Stacks) {
    let repeat_ctx = RepeatContext {
        count: -1,
        min_count: drive.peek_code(2) as usize,
        max_count: drive.peek_code(3) as usize,
        code_position: drive.ctx.code_position,
        last_position: std::usize::MAX,
        prev_id: drive.ctx.repeat_ctx_id,
    };

    stacks.repeat.push(repeat_ctx);

    drive.sync_string_position();

    let next_ctx = drive.next_ctx_from(1, |drive, stacks| {
        drive.ctx.has_matched = drive.popped_ctx().has_matched;
        stacks.repeat.pop();
    });
    next_ctx.repeat_ctx_id = stacks.repeat.len() - 1;
}

#[derive(Debug, Clone, Copy)]
struct MinUntilContext {
    save_repeat_ctx_id: usize,
}

/* minimizing repeat */
fn op_min_until(drive: &mut StateContext, stacks: &mut Stacks) {
    let repeat_ctx = stacks.repeat.last_mut().unwrap();

    drive.sync_string_position();

    repeat_ctx.count += 1;

    if (repeat_ctx.count as usize) < repeat_ctx.min_count {
        // not enough matches
        drive.next_ctx_at(repeat_ctx.code_position + 4, |drive, stacks| {
            if drive.popped_ctx().has_matched == Some(true) {
                drive.success();
            } else {
                stacks.repeat[drive.ctx.repeat_ctx_id].count -= 1;
                drive.sync_string_position();
                drive.failure();
            }
        });
        return;
    }

    drive.state.marks_push();

    stacks.min_until.push(MinUntilContext {
        save_repeat_ctx_id: drive.ctx.repeat_ctx_id,
    });

    // see if the tail matches
    let next_ctx = drive.next_ctx(1, |drive, stacks| {
        drive.ctx.repeat_ctx_id = stacks.min_until.pop().unwrap().save_repeat_ctx_id;

        let repeat_ctx = &mut stacks.repeat[drive.ctx.repeat_ctx_id];

        if drive.popped_ctx().has_matched == Some(true) {
            return drive.success();
        }

        drive.sync_string_position();

        drive.state.marks_pop();

        // match more until tail matches

        if repeat_ctx.count as usize >= repeat_ctx.max_count && repeat_ctx.max_count != MAXREPEAT
            || drive.state.string_position == repeat_ctx.last_position
        {
            repeat_ctx.count -= 1;
            return drive.failure();
        }

        /* zero-width match protection */
        repeat_ctx.last_position = drive.state.string_position;

        drive.next_ctx_at(repeat_ctx.code_position + 4, |drive, stacks| {
            if drive.popped_ctx().has_matched == Some(true) {
                drive.success();
            } else {
                stacks.repeat[drive.ctx.repeat_ctx_id].count -= 1;
                drive.sync_string_position();
                drive.failure();
            }
        });
    });
    next_ctx.repeat_ctx_id = repeat_ctx.prev_id;
}

#[derive(Debug, Clone, Copy)]
struct MaxUntilContext {
    save_last_position: usize,
}

/* maximizing repeat */
fn op_max_until(drive: &mut StateContext, stacks: &mut Stacks) {
    let repeat_ctx = &mut stacks.repeat[drive.ctx.repeat_ctx_id];

    drive.sync_string_position();

    repeat_ctx.count += 1;

    if (repeat_ctx.count as usize) < repeat_ctx.min_count {
        // not enough matches
        drive.next_ctx_at(repeat_ctx.code_position + 4, |drive, stacks| {
            if drive.popped_ctx().has_matched == Some(true) {
                drive.success();
            } else {
                stacks.repeat[drive.ctx.repeat_ctx_id].count -= 1;
                drive.sync_string_position();
                drive.failure();
            }
        });
        return;
    }

    stacks.max_until.push(MaxUntilContext {
        save_last_position: repeat_ctx.last_position,
    });

    if ((repeat_ctx.count as usize) < repeat_ctx.max_count || repeat_ctx.max_count == MAXREPEAT)
        && drive.state.string_position != repeat_ctx.last_position
    {
        /* we may have enough matches, but if we can
        match another item, do so */
        repeat_ctx.last_position = drive.state.string_position;

        drive.state.marks_push();

        drive.next_ctx_at(repeat_ctx.code_position + 4, |drive, stacks| {
            let save_last_position = stacks.max_until.pop().unwrap().save_last_position;
            let repeat_ctx = &mut stacks.repeat[drive.ctx.repeat_ctx_id];
            repeat_ctx.last_position = save_last_position;

            if drive.popped_ctx().has_matched == Some(true) {
                drive.state.marks_pop_discard();
                return drive.success();
            }

            drive.state.marks_pop();
            repeat_ctx.count -= 1;
            drive.sync_string_position();

            /* cannot match more repeated items here.  make sure the
            tail matches */
            let next_ctx = drive.next_ctx(1, tail_callback);
            next_ctx.repeat_ctx_id = repeat_ctx.prev_id;
        });
        return;
    }

    /* cannot match more repeated items here.  make sure the
    tail matches */
    let next_ctx = drive.next_ctx(1, tail_callback);
    next_ctx.repeat_ctx_id = repeat_ctx.prev_id;

    fn tail_callback(drive: &mut StateContext, _stacks: &mut Stacks) {
        if drive.popped_ctx().has_matched == Some(true) {
            drive.success();
        } else {
            drive.sync_string_position();
            drive.failure();
        }
    }
}

#[derive(Debug, Default)]
struct Stacks {
    branch: Vec<BranchContext>,
    min_repeat_one: Vec<MinRepeatOneContext>,
    repeat_one: Vec<RepeatOneContext>,
    repeat: Vec<RepeatContext>,
    min_until: Vec<MinUntilContext>,
    max_until: Vec<MaxUntilContext>,
}

impl Stacks {
    fn clear(&mut self) {
        self.branch.clear();
        self.min_repeat_one.clear();
        self.repeat_one.clear();
        self.repeat.clear();
        self.min_until.clear();
        self.max_until.clear();
    }

    fn branch_last(&mut self) -> &mut BranchContext {
        self.branch.last_mut().unwrap()
    }
    fn min_repeat_one_last(&mut self) -> &mut MinRepeatOneContext {
        self.min_repeat_one.last_mut().unwrap()
    }
    fn repeat_one_last(&mut self) -> &mut RepeatOneContext {
        self.repeat_one.last_mut().unwrap()
    }
}

#[derive(Debug, Clone, Copy)]
pub enum StrDrive<'a> {
    Str(&'a str),
    Bytes(&'a [u8]),
}

impl<'a> From<&'a str> for StrDrive<'a> {
    fn from(s: &'a str) -> Self {
        Self::Str(s)
    }
}
impl<'a> From<&'a [u8]> for StrDrive<'a> {
    fn from(b: &'a [u8]) -> Self {
        Self::Bytes(b)
    }
}

impl<'a> StrDrive<'a> {
    fn offset(&self, offset: usize, skip: usize) -> usize {
        match *self {
            StrDrive::Str(s) => s
                .get(offset..)
                .and_then(|s| s.char_indices().nth(skip).map(|x| x.0 + offset))
                .unwrap_or(s.len()),
            StrDrive::Bytes(_) => offset + skip,
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
                    1 => u32::from_be_bytes([0, 0, 0, bytes[offset - 1]]),
                    2 => u32::from_be_bytes([0, 0, bytes[offset - 2], bytes[offset - 1]]),
                    3 => u32::from_be_bytes([
                        0,
                        bytes[offset - 3],
                        bytes[offset - 2],
                        bytes[offset - 1],
                    ]),
                    4 => u32::from_be_bytes([
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
}

type OpcodeHandler = fn(&mut StateContext, &mut Stacks);

#[derive(Clone, Copy)]
struct MatchContext {
    string_position: usize,
    string_offset: usize,
    code_position: usize,
    has_matched: Option<bool>,
    toplevel: bool,
    handler: Option<OpcodeHandler>,
    repeat_ctx_id: usize,
}

impl std::fmt::Debug for MatchContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MatchContext")
            .field("string_position", &self.string_position)
            .field("string_offset", &self.string_offset)
            .field("code_position", &self.code_position)
            .field("has_matched", &self.has_matched)
            .field("toplevel", &self.toplevel)
            .field("handler", &self.handler.map(|x| x as usize))
            .finish()
    }
}

trait ContextDrive {
    fn ctx(&self) -> &MatchContext;
    fn ctx_mut(&mut self) -> &mut MatchContext;
    fn state(&self) -> &State;

    fn popped_ctx(&self) -> &MatchContext {
        self.state().popped_context.as_ref().unwrap()
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

    fn skip_char(&mut self, skip_count: usize) {
        self.ctx_mut().string_offset = self
            .state()
            .string
            .offset(self.ctx().string_offset, skip_count);
        self.ctx_mut().string_position += skip_count;
    }
    fn skip_code(&mut self, skip_count: usize) {
        self.ctx_mut().code_position += skip_count;
    }
    fn skip_code_from(&mut self, peek: usize) {
        self.skip_code(self.peek_code(peek) as usize + 1);
    }

    fn remaining_chars(&self) -> usize {
        self.state().end - self.ctx().string_position
    }
    fn remaining_codes(&self) -> usize {
        self.state().pattern_codes.len() - self.ctx().code_position
    }

    fn at_beginning(&self) -> bool {
        // self.ctx().string_position == self.state().start
        self.ctx().string_position == 0
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
    fn at_non_boundary<F: FnMut(u32) -> bool>(&self, mut word_checker: F) -> bool {
        if self.at_beginning() && self.at_end() {
            return false;
        }
        let that = !self.at_beginning() && word_checker(self.back_peek_char());
        let this = !self.at_end() && word_checker(self.peek_char());
        this == that
    }

    fn can_success(&self) -> bool {
        if !self.ctx().toplevel {
            return true;
        }
        if self.state().match_all && !self.at_end() {
            return false;
        }
        if self.state().must_advance && self.ctx().string_position == self.state().start {
            return false;
        }
        true
    }

    fn success(&mut self) {
        self.ctx_mut().has_matched = Some(true);
    }

    fn failure(&mut self) {
        self.ctx_mut().has_matched = Some(false);
    }
}

struct StateContext<'a> {
    state: State<'a>,
    ctx: MatchContext,
    next_ctx: Option<MatchContext>,
}

impl ContextDrive for StateContext<'_> {
    fn ctx(&self) -> &MatchContext {
        &self.ctx
    }
    fn ctx_mut(&mut self) -> &mut MatchContext {
        &mut self.ctx
    }
    fn state(&self) -> &State {
        &self.state
    }
}

impl StateContext<'_> {
    fn next_ctx_from(&mut self, peek: usize, handler: OpcodeHandler) -> &mut MatchContext {
        self.next_ctx(self.peek_code(peek) as usize + 1, handler)
    }
    fn next_ctx(&mut self, offset: usize, handler: OpcodeHandler) -> &mut MatchContext {
        self.next_ctx_at(self.ctx.code_position + offset, handler)
    }
    fn next_ctx_at(&mut self, code_position: usize, handler: OpcodeHandler) -> &mut MatchContext {
        self.next_ctx = Some(MatchContext {
            code_position,
            has_matched: None,
            handler: None,
            ..self.ctx
        });
        self.ctx.handler = Some(handler);
        self.next_ctx.as_mut().unwrap()
    }

    fn sync_string_position(&mut self) {
        self.state.string_position = self.ctx.string_position;
    }
}

struct StateRefContext<'a> {
    entity: &'a StateContext<'a>,
    ctx: MatchContext,
}

impl ContextDrive for StateRefContext<'_> {
    fn ctx(&self) -> &MatchContext {
        &self.ctx
    }
    fn ctx_mut(&mut self) -> &mut MatchContext {
        &mut self.ctx
    }
    fn state(&self) -> &State {
        &self.entity.state
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

fn general_op_groupref<F: FnMut(u32) -> u32>(drive: &mut StateContext, mut f: F) {
    let (group_start, group_end) = drive.state.get_marks(drive.peek_code(1) as usize);
    let (group_start, group_end) = match (group_start, group_end) {
        (Some(start), Some(end)) if start <= end => (start, end),
        _ => {
            return drive.failure();
        }
    };

    let mut wdrive = StateRefContext {
        entity: drive,
        ctx: drive.ctx,
    };
    let mut gdrive = StateRefContext {
        entity: drive,
        ctx: MatchContext {
            string_position: group_start,
            // TODO: cache the offset
            string_offset: drive.state.string.offset(0, group_start),
            ..drive.ctx
        },
    };

    for _ in group_start..group_end {
        if wdrive.at_end() || f(wdrive.peek_char()) != f(gdrive.peek_char()) {
            return drive.failure();
        }
        wdrive.skip_char(1);
        gdrive.skip_char(1);
    }

    let position = wdrive.ctx.string_position;
    let offset = wdrive.ctx.string_offset;
    drive.skip_code(2);
    drive.ctx.string_position = position;
    drive.ctx.string_offset = offset;
}

fn general_op_literal<F: FnOnce(u32, u32) -> bool>(drive: &mut StateContext, f: F) {
    if drive.at_end() || !f(drive.peek_code(1), drive.peek_char()) {
        drive.failure();
    } else {
        drive.skip_code(2);
        drive.skip_char(1);
    }
}

fn general_op_in<F: FnOnce(&[u32], u32) -> bool>(drive: &mut StateContext, f: F) {
    if drive.at_end() || !f(&drive.pattern()[2..], drive.peek_char()) {
        drive.failure();
    } else {
        drive.skip_code_from(1);
        drive.skip_char(1);
    }
}

fn at(drive: &StateContext, atcode: SreAtCode) -> bool {
    match atcode {
        SreAtCode::BEGINNING | SreAtCode::BEGINNING_STRING => drive.at_beginning(),
        SreAtCode::BEGINNING_LINE => drive.at_beginning() || is_linebreak(drive.back_peek_char()),
        SreAtCode::BOUNDARY => drive.at_boundary(is_word),
        SreAtCode::NON_BOUNDARY => drive.at_non_boundary(is_word),
        SreAtCode::END => (drive.remaining_chars() == 1 && drive.at_linebreak()) || drive.at_end(),
        SreAtCode::END_LINE => drive.at_linebreak() || drive.at_end(),
        SreAtCode::END_STRING => drive.at_end(),
        SreAtCode::LOC_BOUNDARY => drive.at_boundary(is_loc_word),
        SreAtCode::LOC_NON_BOUNDARY => drive.at_non_boundary(is_loc_word),
        SreAtCode::UNI_BOUNDARY => drive.at_boundary(is_uni_word),
        SreAtCode::UNI_NON_BOUNDARY => drive.at_non_boundary(is_uni_word),
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
                let set = &set[i + 1..];
                if ch < 256 && ((set[(ch >> 5) as usize] & (1u32 << (ch & 31))) != 0) {
                    return ok;
                }
                i += 1 + 8;
            }
            SreOpcode::BIGCHARSET => {
                /* <BIGCHARSET> <blockcount> <256 blockindices> <blocks> */
                let count = set[i + 1] as usize;
                if ch < 0x10000 {
                    let set = &set[i + 2..];
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
fn general_count(drive: &mut StateContext, stacks: &mut Stacks, max_count: usize) -> usize {
    let mut count = 0;
    let max_count = std::cmp::min(max_count, drive.remaining_chars());

    let save_ctx = drive.ctx;
    drive.skip_code(4);
    let reset_position = drive.ctx.code_position;

    while count < max_count {
        drive.ctx.code_position = reset_position;
        let code = drive.peek_code(0);
        let code = SreOpcode::try_from(code).unwrap();
        dispatch(code, drive, stacks);
        if drive.ctx.has_matched == Some(false) {
            break;
        }
        count += 1;
    }
    drive.ctx = save_ctx;
    count
}

fn _count(drive: &mut StateContext, stacks: &mut Stacks, max_count: usize) -> usize {
    let save_ctx = drive.ctx;
    let max_count = std::cmp::min(max_count, drive.remaining_chars());
    let end = drive.ctx.string_position + max_count;
    let opcode = SreOpcode::try_from(drive.peek_code(0)).unwrap();

    match opcode {
        SreOpcode::ANY => {
            while !drive.ctx.string_position < end && !drive.at_linebreak() {
                drive.skip_char(1);
            }
        }
        SreOpcode::ANY_ALL => {
            drive.skip_char(max_count);
        }
        SreOpcode::IN => {
            while !drive.ctx.string_position < end
                && charset(&drive.pattern()[2..], drive.peek_char())
            {
                drive.skip_char(1);
            }
        }
        SreOpcode::LITERAL => {
            general_count_literal(drive, end, |code, c| code == c as u32);
        }
        SreOpcode::NOT_LITERAL => {
            general_count_literal(drive, end, |code, c| code != c as u32);
        }
        SreOpcode::LITERAL_IGNORE => {
            general_count_literal(drive, end, |code, c| code == lower_ascii(c) as u32);
        }
        SreOpcode::NOT_LITERAL_IGNORE => {
            general_count_literal(drive, end, |code, c| code != lower_ascii(c) as u32);
        }
        SreOpcode::LITERAL_LOC_IGNORE => {
            general_count_literal(drive, end, char_loc_ignore);
        }
        SreOpcode::NOT_LITERAL_LOC_IGNORE => {
            general_count_literal(drive, end, |code, c| !char_loc_ignore(code, c));
        }
        SreOpcode::LITERAL_UNI_IGNORE => {
            general_count_literal(drive, end, |code, c| code == lower_unicode(c) as u32);
        }
        SreOpcode::NOT_LITERAL_UNI_IGNORE => {
            general_count_literal(drive, end, |code, c| code != lower_unicode(c) as u32);
        }
        _ => {
            return general_count(drive, stacks, max_count);
        }
    }

    let count = drive.ctx.string_position - drive.state.string_position;
    drive.ctx = save_ctx;
    count
}

fn general_count_literal<F: FnMut(u32, u32) -> bool>(
    drive: &mut StateContext,
    end: usize,
    mut f: F,
) {
    let ch = drive.peek_code(1);
    while !drive.ctx.string_position < end && f(ch, drive.peek_char()) {
        drive.skip_char(1);
    }
}

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
    // FIXME: Ignore the locales
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
pub fn lower_ascii(ch: u32) -> u32 {
    u8::try_from(ch)
        .map(|x| x.to_ascii_lowercase() as u32)
        .unwrap_or(ch)
}
fn lower_locate(ch: u32) -> u32 {
    // FIXME: Ignore the locales
    lower_ascii(ch)
}
fn upper_locate(ch: u32) -> u32 {
    // FIXME: Ignore the locales
    u8::try_from(ch)
        .map(|x| x.to_ascii_uppercase() as u32)
        .unwrap_or(ch)
}
fn is_uni_digit(ch: u32) -> bool {
    // TODO: check with cpython
    char::try_from(ch)
        .map(|x| x.is_ascii_digit())
        .unwrap_or(false)
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
pub fn lower_unicode(ch: u32) -> u32 {
    // TODO: check with cpython
    char::try_from(ch)
        .map(|x| x.to_lowercase().next().unwrap() as u32)
        .unwrap_or(ch)
}
pub fn upper_unicode(ch: u32) -> u32 {
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
