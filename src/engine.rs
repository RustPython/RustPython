// good luck to those that follow; here be dragons

use super::constants::{SreAtCode, SreCatCode, SreFlag, SreOpcode};
use super::MAXREPEAT;
use std::convert::TryFrom;

const fn is_py_ascii_whitespace(b: u8) -> bool {
    matches!(b, b'\t' | b'\n' | b'\x0C' | b'\r' | b' ' | b'\x0B')
}

#[derive(Debug)]
pub struct State<'a, S: StrDrive> {
    pub string: S,
    pub start: usize,
    pub end: usize,
    _flags: SreFlag,
    pattern_codes: &'a [u32],
    pub marks: Vec<Option<usize>>,
    pub lastindex: isize,
    marks_stack: Vec<(Vec<Option<usize>>, isize)>,
    context_stack: Vec<MatchContext<'a, S>>,
    repeat_stack: Vec<RepeatContext>,
    pub string_position: usize,
    next_context: Option<MatchContext<'a, S>>,
    popped_has_matched: bool,
    pub has_matched: bool,
    pub match_all: bool,
    pub must_advance: bool,
}

macro_rules! next_ctx {
    (offset $offset:expr, $state:expr, $ctx:expr, $handler:expr) => {
        next_ctx!(position $ctx.code_position + $offset, $state, $ctx, $handler)
    };
    (from $peek:expr, $state:expr, $ctx:expr, $handler:expr) => {
        next_ctx!(position $ctx.peek_code($state, $peek) as usize + 1, $state, $ctx, $handler)
    };
    (position $position:expr, $state:expr, $ctx:expr, $handler:expr) => {{
        $ctx.handler = Some($handler);
        $state.next_context.insert(MatchContext {
            code_position: $position,
            has_matched: None,
            handler: None,
            count: -1,
            ..*$ctx
        })
    }};
}

macro_rules! mark {
    (push, $state:expr) => {
        $state
            .marks_stack
            .push(($state.marks.clone(), $state.lastindex))
    };
    (pop, $state:expr) => {
        let (marks, lastindex) = $state.marks_stack.pop().unwrap();
        $state.marks = marks;
        $state.lastindex = lastindex;
    };
}

impl<'a, S: StrDrive> State<'a, S> {
    pub fn new(
        string: S,
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
            repeat_stack: Vec::new(),
            string_position: start,
            next_context: None,
            popped_has_matched: false,
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
        self.repeat_stack.clear();
        self.string_position = self.start;
        self.next_context = None;
        self.popped_has_matched = false;
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
    // fn marks_push(&mut self) {
    //     self.marks_stack.push((self.marks.clone(), self.lastindex));
    // }
    // fn marks_pop(&mut self) {
    //     let (marks, lastindex) = self.marks_stack.pop().unwrap();
    //     self.marks = marks;
    //     self.lastindex = lastindex;
    // }
    fn marks_pop_keep(&mut self) {
        let (marks, lastindex) = self.marks_stack.last().unwrap().clone();
        self.marks = marks;
        self.lastindex = lastindex;
    }
    fn marks_pop_discard(&mut self) {
        self.marks_stack.pop();
    }

    fn _match(&mut self) {
        while let Some(mut ctx) = self.context_stack.pop() {
            if let Some(handler) = ctx.handler.take() {
                handler(self, &mut ctx);
            } else if ctx.remaining_codes(self) > 0 {
                let code = ctx.peek_code(self, 0);
                let code = SreOpcode::try_from(code).unwrap();
                dispatch(self, &mut ctx, code);
            } else {
                ctx.failure();
            }

            if let Some(has_matched) = ctx.has_matched {
                self.popped_has_matched = has_matched;
            } else {
                self.context_stack.push(ctx);
                if let Some(next_ctx) = self.next_context.take() {
                    self.context_stack.push(next_ctx);
                }
            }
        }
        self.has_matched = self.popped_has_matched;
    }

    pub fn pymatch(&mut self) {
        let ctx = MatchContext {
            string_position: self.start,
            string_offset: self.string.offset(0, self.start),
            code_position: 0,
            has_matched: None,
            toplevel: true,
            handler: None,
            repeat_ctx_id: usize::MAX,
            count: -1,
        };
        self.context_stack.push(ctx);

        self._match();
    }

    pub fn search(&mut self) {
        // TODO: optimize by op info and skip prefix

        if self.start > self.end {
            return;
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
            count: -1,
        };
        self.context_stack.push(ctx);
        self._match();

        self.must_advance = false;
        while !self.has_matched && self.start < self.end {
            self.start += 1;
            start_offset = self.string.offset(start_offset, 1);
            self.reset();

            let ctx = MatchContext {
                string_position: self.start,
                string_offset: start_offset,
                code_position: 0,
                has_matched: None,
                toplevel: false,
                handler: None,
                repeat_ctx_id: usize::MAX,
                count: -1,
            };
            self.context_stack.push(ctx);
            self._match();
        }
    }
}

fn dispatch<'a, S: StrDrive>(
    state: &mut State<'a, S>,
    ctx: &mut MatchContext<'a, S>,
    opcode: SreOpcode,
) {
    match opcode {
        SreOpcode::FAILURE => {
            ctx.failure();
        }
        SreOpcode::SUCCESS => {
            if ctx.can_success(state) {
                state.string_position = ctx.string_position;
                ctx.success();
            } else {
                ctx.failure();
            }
        }
        SreOpcode::ANY => {
            if ctx.at_end(state) || ctx.at_linebreak(state) {
                ctx.failure();
            } else {
                ctx.skip_code(1);
                ctx.skip_char(state, 1);
            }
        }
        SreOpcode::ANY_ALL => {
            if ctx.at_end(state) {
                ctx.failure();
            } else {
                ctx.skip_code(1);
                ctx.skip_char(state, 1);
            }
        }
        /* assert subpattern */
        /* <ASSERT> <skip> <back> <pattern> */
        SreOpcode::ASSERT => op_assert(state, ctx),
        SreOpcode::ASSERT_NOT => op_assert_not(state, ctx),
        SreOpcode::AT => {
            let atcode = SreAtCode::try_from(ctx.peek_code(state, 1)).unwrap();
            if at(state, ctx, atcode) {
                ctx.skip_code(2);
            } else {
                ctx.failure();
            }
        }
        SreOpcode::BRANCH => op_branch(state, ctx),
        SreOpcode::CATEGORY => {
            let catcode = SreCatCode::try_from(ctx.peek_code(state, 1)).unwrap();
            if ctx.at_end(state) || !category(catcode, ctx.peek_char(state)) {
                ctx.failure();
            } else {
                ctx.skip_code(2);
                ctx.skip_char(state, 1);
            }
        }
        SreOpcode::IN => general_op_in(state, ctx, charset),
        SreOpcode::IN_IGNORE => general_op_in(state, ctx, |set, c| charset(set, lower_ascii(c))),
        SreOpcode::IN_UNI_IGNORE => {
            general_op_in(state, ctx, |set, c| charset(set, lower_unicode(c)))
        }
        SreOpcode::IN_LOC_IGNORE => general_op_in(state, ctx, charset_loc_ignore),
        SreOpcode::INFO | SreOpcode::JUMP => ctx.skip_code_from(state, 1),
        SreOpcode::LITERAL => general_op_literal(state, ctx, |code, c| code == c),
        SreOpcode::NOT_LITERAL => general_op_literal(state, ctx, |code, c| code != c),
        SreOpcode::LITERAL_IGNORE => {
            general_op_literal(state, ctx, |code, c| code == lower_ascii(c))
        }
        SreOpcode::NOT_LITERAL_IGNORE => {
            general_op_literal(state, ctx, |code, c| code != lower_ascii(c))
        }
        SreOpcode::LITERAL_UNI_IGNORE => {
            general_op_literal(state, ctx, |code, c| code == lower_unicode(c))
        }
        SreOpcode::NOT_LITERAL_UNI_IGNORE => {
            general_op_literal(state, ctx, |code, c| code != lower_unicode(c))
        }
        SreOpcode::LITERAL_LOC_IGNORE => general_op_literal(state, ctx, char_loc_ignore),
        SreOpcode::NOT_LITERAL_LOC_IGNORE => {
            general_op_literal(state, ctx, |code, c| !char_loc_ignore(code, c))
        }
        SreOpcode::MARK => {
            state.set_mark(ctx.peek_code(state, 1) as usize, ctx.string_position);
            ctx.skip_code(2);
        }
        SreOpcode::MAX_UNTIL => op_max_until(state, ctx),
        SreOpcode::MIN_UNTIL => op_min_until(state, ctx),
        SreOpcode::REPEAT => op_repeat(state, ctx),
        SreOpcode::REPEAT_ONE => op_repeat_one(state, ctx),
        SreOpcode::MIN_REPEAT_ONE => op_min_repeat_one(state, ctx),
        SreOpcode::GROUPREF => general_op_groupref(state, ctx, |x| x),
        SreOpcode::GROUPREF_IGNORE => general_op_groupref(state, ctx, lower_ascii),
        SreOpcode::GROUPREF_LOC_IGNORE => general_op_groupref(state, ctx, lower_locate),
        SreOpcode::GROUPREF_UNI_IGNORE => general_op_groupref(state, ctx, lower_unicode),
        SreOpcode::GROUPREF_EXISTS => {
            let (group_start, group_end) = state.get_marks(ctx.peek_code(state, 1) as usize);
            match (group_start, group_end) {
                (Some(start), Some(end)) if start <= end => {
                    ctx.skip_code(3);
                }
                _ => ctx.skip_code_from(state, 2),
            }
        }
        _ => unreachable!("unexpected opcode"),
    }
}

/* assert subpattern */
/* <ASSERT> <skip> <back> <pattern> */
fn op_assert<'a, S: StrDrive>(state: &mut State<'a, S>, ctx: &mut MatchContext<'a, S>) {
    let back = ctx.peek_code(state, 2) as usize;
    if ctx.string_position < back {
        return ctx.failure();
    }

    // let next_ctx = state.next_ctx(ctx, 3, |state, ctx| {
    let next_ctx = next_ctx!(offset 3, state, ctx, |state, ctx| {
        if state.popped_has_matched {
            ctx.skip_code_from(state, 1);
        } else {
            ctx.failure();
        }
    });
    next_ctx.back_skip_char(&state.string, back);
    state.string_position = next_ctx.string_position;
    next_ctx.toplevel = false;
}

/* assert not subpattern */
/* <ASSERT_NOT> <skip> <back> <pattern> */
fn op_assert_not<'a, S: StrDrive>(state: &mut State<'a, S>, ctx: &mut MatchContext<'a, S>) {
    let back = ctx.peek_code(state, 2) as usize;

    if ctx.string_position < back {
        return ctx.skip_code_from(state, 1);
    }

    let next_ctx = next_ctx!(offset 3, state, ctx, |state, ctx| {
        if state.popped_has_matched {
            ctx.failure();
        } else {
            ctx.skip_code_from(state, 1);
        }
    });
    next_ctx.back_skip_char(&state.string, back);
    state.string_position = next_ctx.string_position;
    next_ctx.toplevel = false;
}

// alternation
// <BRANCH> <0=skip> code <JUMP> ... <NULL>
fn op_branch<'a, S: StrDrive>(state: &mut State<'a, S>, ctx: &mut MatchContext<'a, S>) {
    // state.marks_push();
    mark!(push, state);

    ctx.count = 1;
    create_context(state, ctx);

    fn create_context<'a, S: StrDrive>(state: &mut State<'a, S>, ctx: &mut MatchContext<'a, S>) {
        let branch_offset = ctx.count as usize;
        let next_length = ctx.peek_code(state, branch_offset) as isize;
        if next_length == 0 {
            state.marks_pop_discard();
            return ctx.failure();
        }

        state.string_position = ctx.string_position;

        ctx.count += next_length;
        next_ctx!(offset branch_offset + 1, state, ctx, callback);
    }

    fn callback<'a, S: StrDrive>(state: &mut State<'a, S>, ctx: &mut MatchContext<'a, S>) {
        if state.popped_has_matched {
            return ctx.success();
        }
        state.marks_pop_keep();
        create_context(state, ctx);
    }
}

/* <MIN_REPEAT_ONE> <skip> <1=min> <2=max> item <SUCCESS> tail */
fn op_min_repeat_one<'a, S: StrDrive>(state: &mut State<'a, S>, ctx: &mut MatchContext<'a, S>) {
    let min_count = ctx.peek_code(state, 2) as usize;
    // let max_count = ctx.peek_code(state, 3) as usize;

    if ctx.remaining_chars(state) < min_count {
        return ctx.failure();
    }

    state.string_position = ctx.string_position;

    ctx.count = if min_count == 0 {
        0
    } else {
        let count = _count(state, ctx, min_count);
        if count < min_count {
            return ctx.failure();
        }
        ctx.skip_char(state, count);
        count as isize
    };

    let next_code = ctx.peek_code(state, ctx.peek_code(state, 1) as usize + 1);
    if next_code == SreOpcode::SUCCESS as u32 && ctx.can_success(state) {
        // tail is empty. we're finished
        state.string_position = ctx.string_position;
        return ctx.success();
    }

    mark!(push, state);
    create_context(state, ctx);

    fn create_context<'a, S: StrDrive>(state: &mut State<'a, S>, ctx: &mut MatchContext<'a, S>) {
        let max_count = ctx.peek_code(state, 3) as usize;

        if max_count == MAXREPEAT || ctx.count as usize <= max_count {
            state.string_position = ctx.string_position;
            next_ctx!(from 1, state, ctx, callback);
        } else {
            state.marks_pop_discard();
            ctx.failure();
        }
    }

    fn callback<'a, S: StrDrive>(state: &mut State<'a, S>, ctx: &mut MatchContext<'a, S>) {
        if state.popped_has_matched {
            return ctx.success();
        }

        state.string_position = ctx.string_position;

        if _count(state, ctx, 1) == 0 {
            state.marks_pop_discard();
            return ctx.failure();
        }

        ctx.skip_char(state, 1);
        ctx.count += 1;
        state.marks_pop_keep();
        create_context(state, ctx);
    }
}

/* match repeated sequence (maximizing regexp) */
/* this operator only works if the repeated item is
exactly one character wide, and we're not already
collecting backtracking points.  for other cases,
use the MAX_REPEAT operator */
/* <REPEAT_ONE> <skip> <1=min> <2=max> item <SUCCESS> tail */
fn op_repeat_one<'a, S: StrDrive>(state: &mut State<'a, S>, ctx: &mut MatchContext<'a, S>) {
    let min_count = ctx.peek_code(state, 2) as usize;
    let max_count = ctx.peek_code(state, 3) as usize;

    if ctx.remaining_chars(state) < min_count {
        return ctx.failure();
    }

    state.string_position = ctx.string_position;

    let count = _count(state, ctx, max_count);
    ctx.skip_char(state, count);
    if count < min_count {
        return ctx.failure();
    }

    let next_code = ctx.peek_code(state, ctx.peek_code(state, 1) as usize + 1);
    if next_code == SreOpcode::SUCCESS as u32 && ctx.can_success(state) {
        // tail is empty. we're finished
        state.string_position = ctx.string_position;
        return ctx.success();
    }

    mark!(push, state);
    ctx.count = count as isize;
    create_context(state, ctx);

    fn create_context<'a, S: StrDrive>(state: &mut State<'a, S>, ctx: &mut MatchContext<'a, S>) {
        let min_count = ctx.peek_code(state, 2) as isize;
        let next_code = ctx.peek_code(state, ctx.peek_code(state, 1) as usize + 1);
        if next_code == SreOpcode::LITERAL as u32 {
            // Special case: Tail starts with a literal. Skip positions where
            // the rest of the pattern cannot possibly match.
            let c = ctx.peek_code(state, ctx.peek_code(state, 1) as usize + 2);
            while ctx.at_end(state) || ctx.peek_char(state) != c {
                if ctx.count <= min_count {
                    state.marks_pop_discard();
                    return ctx.failure();
                }
                ctx.back_skip_char(&state.string, 1);
                ctx.count -= 1;
            }
        }

        state.string_position = ctx.string_position;

        // General case: backtracking
        next_ctx!(from 1, state, ctx, callback);
    }

    fn callback<'a, S: StrDrive>(state: &mut State<'a, S>, ctx: &mut MatchContext<'a, S>) {
        if state.popped_has_matched {
            return ctx.success();
        }

        let min_count = ctx.peek_code(state, 2) as isize;

        if ctx.count <= min_count {
            state.marks_pop_discard();
            return ctx.failure();
        }

        ctx.back_skip_char(&state.string, 1);
        ctx.count -= 1;

        state.marks_pop_keep();
        create_context(state, ctx);
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
fn op_repeat<'a, S: StrDrive>(state: &mut State<'a, S>, ctx: &mut MatchContext<'a, S>) {
    let repeat_ctx = RepeatContext {
        count: -1,
        min_count: ctx.peek_code(state, 2) as usize,
        max_count: ctx.peek_code(state, 3) as usize,
        code_position: ctx.code_position,
        last_position: std::usize::MAX,
        prev_id: ctx.repeat_ctx_id,
    };

    state.repeat_stack.push(repeat_ctx);

    state.string_position = ctx.string_position;

    let next_ctx = next_ctx!(from 1, state, ctx, |state, ctx| {
        ctx.has_matched = Some(state.popped_has_matched);
        state.repeat_stack.pop();
    });
    next_ctx.repeat_ctx_id = state.repeat_stack.len() - 1;
}

/* minimizing repeat */
fn op_min_until<'a, S: StrDrive>(state: &mut State<'a, S>, ctx: &mut MatchContext<'a, S>) {
    let repeat_ctx = state.repeat_stack.last_mut().unwrap();

    state.string_position = ctx.string_position;

    repeat_ctx.count += 1;

    if (repeat_ctx.count as usize) < repeat_ctx.min_count {
        // not enough matches
        next_ctx!(position repeat_ctx.code_position + 4, state, ctx, |state, ctx| {
            if state.popped_has_matched {
                ctx.success();
            } else {
                state.repeat_stack[ctx.repeat_ctx_id].count -= 1;
                state.string_position = ctx.string_position;
                ctx.failure();
            }
        });
        return;
    }

    mark!(push, state);

    ctx.count = ctx.repeat_ctx_id as isize;

    // see if the tail matches
    let next_ctx = next_ctx!(offset 1, state, ctx, |state, ctx| {
        if state.popped_has_matched {
            return ctx.success();
        }

        ctx.repeat_ctx_id = ctx.count as usize;

        let repeat_ctx = &mut state.repeat_stack[ctx.repeat_ctx_id];

        state.string_position = ctx.string_position;

        mark!(pop, state);

        // match more until tail matches

        if repeat_ctx.count as usize >= repeat_ctx.max_count && repeat_ctx.max_count != MAXREPEAT
            || state.string_position == repeat_ctx.last_position
        {
            repeat_ctx.count -= 1;
            return ctx.failure();
        }

        /* zero-width match protection */
        repeat_ctx.last_position = state.string_position;

        next_ctx!(position repeat_ctx.code_position + 4, state, ctx, |state, ctx| {
            if state.popped_has_matched {
                ctx.success();
            } else {
                state.repeat_stack[ctx.repeat_ctx_id].count -= 1;
                state.string_position = ctx.string_position;
                ctx.failure();
            }
        });
    });
    next_ctx.repeat_ctx_id = repeat_ctx.prev_id;
}

/* maximizing repeat */
fn op_max_until<'a, S: StrDrive>(state: &mut State<'a, S>, ctx: &mut MatchContext<'a, S>) {
    let repeat_ctx = &mut state.repeat_stack[ctx.repeat_ctx_id];

    state.string_position = ctx.string_position;

    repeat_ctx.count += 1;

    if (repeat_ctx.count as usize) < repeat_ctx.min_count {
        // not enough matches
        next_ctx!(position repeat_ctx.code_position + 4, state, ctx, |state, ctx| {
            if state.popped_has_matched {
                ctx.success();
            } else {
                state.repeat_stack[ctx.repeat_ctx_id].count -= 1;
                state.string_position = ctx.string_position;
                ctx.failure();
            }
        });
        return;
    }

    if ((repeat_ctx.count as usize) < repeat_ctx.max_count || repeat_ctx.max_count == MAXREPEAT)
        && state.string_position != repeat_ctx.last_position
    {
        /* we may have enough matches, but if we can
        match another item, do so */
        mark!(push, state);

        ctx.count = repeat_ctx.last_position as isize;
        repeat_ctx.last_position = state.string_position;

        next_ctx!(position repeat_ctx.code_position + 4, state, ctx, |state, ctx| {
            let save_last_position = ctx.count as usize;
            let repeat_ctx = &mut state.repeat_stack[ctx.repeat_ctx_id];
            repeat_ctx.last_position = save_last_position;

            if state.popped_has_matched {
                state.marks_pop_discard();
                return ctx.success();
            }

            mark!(pop, state);
            repeat_ctx.count -= 1;

            state.string_position = ctx.string_position;

            /* cannot match more repeated items here.  make sure the
            tail matches */
            let next_ctx = next_ctx!(offset 1, state, ctx, tail_callback);
            next_ctx.repeat_ctx_id = repeat_ctx.prev_id;
        });
        return;
    }

    /* cannot match more repeated items here.  make sure the
    tail matches */
    let next_ctx = next_ctx!(offset 1, state, ctx, tail_callback);
    next_ctx.repeat_ctx_id = repeat_ctx.prev_id;

    fn tail_callback<'a, S: StrDrive>(state: &mut State<'a, S>, ctx: &mut MatchContext<'a, S>) {
        if state.popped_has_matched {
            ctx.success();
        } else {
            state.string_position = ctx.string_position;
            ctx.failure();
        }
    }
}

pub trait StrDrive: Copy {
    fn offset(&self, offset: usize, skip: usize) -> usize;
    fn count(&self) -> usize;
    fn peek(&self, offset: usize) -> u32;
    fn back_peek(&self, offset: usize) -> u32;
    fn back_offset(&self, offset: usize, skip: usize) -> usize;
}

impl<'a> StrDrive for &'a str {
    fn offset(&self, offset: usize, skip: usize) -> usize {
        self.get(offset..)
            .and_then(|s| s.char_indices().nth(skip).map(|x| x.0 + offset))
            .unwrap_or(self.len())
    }

    fn count(&self) -> usize {
        self.chars().count()
    }

    fn peek(&self, offset: usize) -> u32 {
        unsafe { self.get_unchecked(offset..) }
            .chars()
            .next()
            .unwrap() as u32
    }

    fn back_peek(&self, offset: usize) -> u32 {
        let bytes = self.as_bytes();
        let back_offset = utf8_back_peek_offset(bytes, offset);
        match offset - back_offset {
            1 => u32::from_be_bytes([0, 0, 0, bytes[offset - 1]]),
            2 => u32::from_be_bytes([0, 0, bytes[offset - 2], bytes[offset - 1]]),
            3 => u32::from_be_bytes([0, bytes[offset - 3], bytes[offset - 2], bytes[offset - 1]]),
            4 => u32::from_be_bytes([
                bytes[offset - 4],
                bytes[offset - 3],
                bytes[offset - 2],
                bytes[offset - 1],
            ]),
            _ => unreachable!(),
        }
    }

    fn back_offset(&self, offset: usize, skip: usize) -> usize {
        let bytes = self.as_bytes();
        let mut back_offset = offset;
        for _ in 0..skip {
            back_offset = utf8_back_peek_offset(bytes, back_offset);
        }
        back_offset
    }
}

impl<'a> StrDrive for &'a [u8] {
    fn offset(&self, offset: usize, skip: usize) -> usize {
        offset + skip
    }

    fn count(&self) -> usize {
        self.len()
    }

    fn peek(&self, offset: usize) -> u32 {
        self[offset] as u32
    }

    fn back_peek(&self, offset: usize) -> u32 {
        self[offset - 1] as u32
    }

    fn back_offset(&self, offset: usize, skip: usize) -> usize {
        offset - skip
    }
}

// type OpcodeHandler = for<'a>fn(&mut StateContext<'a, S>, &mut Stacks);

#[derive(Clone, Copy)]
struct MatchContext<'a, S: StrDrive> {
    string_position: usize,
    string_offset: usize,
    code_position: usize,
    has_matched: Option<bool>,
    toplevel: bool,
    handler: Option<fn(&mut State<'a, S>, &mut Self)>,
    repeat_ctx_id: usize,
    count: isize,
}

impl<'a, S: StrDrive> std::fmt::Debug for MatchContext<'a, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MatchContext")
            .field("string_position", &self.string_position)
            .field("string_offset", &self.string_offset)
            .field("code_position", &self.code_position)
            .field("has_matched", &self.has_matched)
            .field("toplevel", &self.toplevel)
            .field("handler", &self.handler.map(|x| x as usize))
            .field("count", &self.count)
            .finish()
    }
}

impl<'a, S: StrDrive> MatchContext<'a, S> {
    fn pattern(&self, state: &State<'a, S>) -> &[u32] {
        &state.pattern_codes[self.code_position..]
    }

    fn remaining_codes(&self, state: &State<'a, S>) -> usize {
        state.pattern_codes.len() - self.code_position
    }

    fn remaining_chars(&self, state: &State<'a, S>) -> usize {
        state.end - self.string_position
    }

    fn peek_char(&self, state: &State<'a, S>) -> u32 {
        state.string.peek(self.string_offset)
    }

    fn skip_char(&mut self, state: &State<'a, S>, skip: usize) {
        self.string_position += skip;
        self.string_offset = state.string.offset(self.string_offset, skip);
    }

    fn back_peek_char(&self, state: &State<'a, S>) -> u32 {
        state.string.back_peek(self.string_offset)
    }

    fn back_skip_char(&mut self, string: &S, skip: usize) {
        self.string_position -= skip;
        self.string_offset = string.back_offset(self.string_offset, skip);
    }

    fn peek_code(&self, state: &State<'a, S>, peek: usize) -> u32 {
        state.pattern_codes[self.code_position + peek]
    }

    fn skip_code(&mut self, skip: usize) {
        self.code_position += skip;
    }

    fn skip_code_from(&mut self, state: &State<'a, S>, peek: usize) {
        self.skip_code(self.peek_code(state, peek) as usize + 1);
    }

    fn at_beginning(&self) -> bool {
        // self.ctx().string_position == self.state().start
        self.string_position == 0
    }

    fn at_end(&self, state: &State<'a, S>) -> bool {
        self.string_position == state.end
    }

    fn at_linebreak(&self, state: &State<'a, S>) -> bool {
        !self.at_end(state) && is_linebreak(self.peek_char(state))
    }

    fn at_boundary<F: FnMut(u32) -> bool>(
        &self,
        state: &State<'a, S>,
        mut word_checker: F,
    ) -> bool {
        if self.at_beginning() && self.at_end(state) {
            return false;
        }
        let that = !self.at_beginning() && word_checker(self.back_peek_char(state));
        let this = !self.at_end(state) && word_checker(self.peek_char(state));
        this != that
    }

    fn at_non_boundary<F: FnMut(u32) -> bool>(
        &self,
        state: &State<'a, S>,
        mut word_checker: F,
    ) -> bool {
        if self.at_beginning() && self.at_end(state) {
            return false;
        }
        let that = !self.at_beginning() && word_checker(self.back_peek_char(state));
        let this = !self.at_end(state) && word_checker(self.peek_char(state));
        this == that
    }

    fn can_success(&self, state: &State<'a, S>) -> bool {
        if !self.toplevel {
            return true;
        }
        if state.match_all && !self.at_end(state) {
            return false;
        }
        if state.must_advance && self.string_position == state.start {
            return false;
        }
        true
    }

    fn success(&mut self) {
        self.has_matched = Some(true);
    }

    fn failure(&mut self) {
        self.has_matched = Some(false);
    }
}

fn at<'a, S: StrDrive>(state: &State<'a, S>, ctx: &MatchContext<'a, S>, atcode: SreAtCode) -> bool {
    match atcode {
        SreAtCode::BEGINNING | SreAtCode::BEGINNING_STRING => ctx.at_beginning(),
        SreAtCode::BEGINNING_LINE => ctx.at_beginning() || is_linebreak(ctx.back_peek_char(state)),
        SreAtCode::BOUNDARY => ctx.at_boundary(state, is_word),
        SreAtCode::NON_BOUNDARY => ctx.at_non_boundary(state, is_word),
        SreAtCode::END => {
            (ctx.remaining_chars(state) == 1 && ctx.at_linebreak(state)) || ctx.at_end(state)
        }
        SreAtCode::END_LINE => ctx.at_linebreak(state) || ctx.at_end(state),
        SreAtCode::END_STRING => ctx.at_end(state),
        SreAtCode::LOC_BOUNDARY => ctx.at_boundary(state, is_loc_word),
        SreAtCode::LOC_NON_BOUNDARY => ctx.at_non_boundary(state, is_loc_word),
        SreAtCode::UNI_BOUNDARY => ctx.at_boundary(state, is_uni_word),
        SreAtCode::UNI_NON_BOUNDARY => ctx.at_non_boundary(state, is_uni_word),
    }
}

fn general_op_literal<'a, S: StrDrive, F: FnOnce(u32, u32) -> bool>(
    state: &State<'a, S>,
    ctx: &mut MatchContext<'a, S>,
    f: F,
) {
    if ctx.at_end(state) || !f(ctx.peek_code(state, 1), ctx.peek_char(state)) {
        ctx.failure();
    } else {
        ctx.skip_code(2);
        ctx.skip_char(state, 1);
    }
}

fn general_op_in<'a, S: StrDrive, F: FnOnce(&[u32], u32) -> bool>(
    state: &State<'a, S>,
    ctx: &mut MatchContext<'a, S>,
    f: F,
) {
    if ctx.at_end(state) || !f(&ctx.pattern(state)[2..], ctx.peek_char(state)) {
        ctx.failure();
    } else {
        ctx.skip_code_from(state, 1);
        ctx.skip_char(state, 1);
    }
}

fn general_op_groupref<'a, S: StrDrive, F: FnMut(u32) -> u32>(
    state: &State<'a, S>,
    ctx: &mut MatchContext<'a, S>,
    mut f: F,
) {
    let (group_start, group_end) = state.get_marks(ctx.peek_code(state, 1) as usize);
    let (group_start, group_end) = match (group_start, group_end) {
        (Some(start), Some(end)) if start <= end => (start, end),
        _ => {
            return ctx.failure();
        }
    };

    let mut gctx = MatchContext {
        string_position: group_start,
        string_offset: state.string.offset(0, group_start),
        ..*ctx
    };

    for _ in group_start..group_end {
        if ctx.at_end(state) || f(ctx.peek_char(state)) != f(gctx.peek_char(state)) {
            return ctx.failure();
        }
        ctx.skip_char(state, 1);
        gctx.skip_char(state, 1);
    }

    ctx.skip_code(2);
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

fn _count<'a, S: StrDrive>(
    state: &mut State<'a, S>,
    ctx: &MatchContext<'a, S>,
    max_count: usize,
) -> usize {
    let mut ctx = *ctx;
    let max_count = std::cmp::min(max_count, ctx.remaining_chars(state));
    let end = ctx.string_position + max_count;
    let opcode = SreOpcode::try_from(ctx.peek_code(state, 0)).unwrap();

    match opcode {
        SreOpcode::ANY => {
            while !ctx.string_position < end && !ctx.at_linebreak(state) {
                ctx.skip_char(state, 1);
            }
        }
        SreOpcode::ANY_ALL => {
            ctx.skip_char(state, max_count);
        }
        SreOpcode::IN => {
            while !ctx.string_position < end
                && charset(&ctx.pattern(state)[2..], ctx.peek_char(state))
            {
                ctx.skip_char(state, 1);
            }
        }
        SreOpcode::LITERAL => {
            general_count_literal(state, &mut ctx, end, |code, c| code == c as u32);
        }
        SreOpcode::NOT_LITERAL => {
            general_count_literal(state, &mut ctx, end, |code, c| code != c as u32);
        }
        SreOpcode::LITERAL_IGNORE => {
            general_count_literal(state, &mut ctx, end, |code, c| {
                code == lower_ascii(c) as u32
            });
        }
        SreOpcode::NOT_LITERAL_IGNORE => {
            general_count_literal(state, &mut ctx, end, |code, c| {
                code != lower_ascii(c) as u32
            });
        }
        SreOpcode::LITERAL_LOC_IGNORE => {
            general_count_literal(state, &mut ctx, end, char_loc_ignore);
        }
        SreOpcode::NOT_LITERAL_LOC_IGNORE => {
            general_count_literal(state, &mut ctx, end, |code, c| !char_loc_ignore(code, c));
        }
        SreOpcode::LITERAL_UNI_IGNORE => {
            general_count_literal(state, &mut ctx, end, |code, c| {
                code == lower_unicode(c) as u32
            });
        }
        SreOpcode::NOT_LITERAL_UNI_IGNORE => {
            general_count_literal(state, &mut ctx, end, |code, c| {
                code != lower_unicode(c) as u32
            });
        }
        _ => {
            /* General case */
            let mut count = 0;

            ctx.skip_code(4);
            let reset_position = ctx.code_position;

            while count < max_count {
                ctx.code_position = reset_position;
                let code = ctx.peek_code(state, 0);
                let code = SreOpcode::try_from(code).unwrap();
                dispatch(state, &mut ctx, code);
                if ctx.has_matched == Some(false) {
                    break;
                }
                count += 1;
            }
            return count;
        }
    }

    // TODO: return offset
    ctx.string_position - state.string_position
}

fn general_count_literal<'a, S: StrDrive, F: FnMut(u32, u32) -> bool>(
    state: &State<'a, S>,
    ctx: &mut MatchContext<'a, S>,
    end: usize,
    mut f: F,
) {
    let ch = ctx.peek_code(state, 1);
    while !ctx.string_position < end && f(ch, ctx.peek_char(state)) {
        ctx.skip_char(state, 1);
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
