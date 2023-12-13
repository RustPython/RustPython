// good luck to those that follow; here be dragons

use crate::constants::SreInfo;

use super::constants::{SreAtCode, SreCatCode, SreOpcode};
use super::MAXREPEAT;
use optional::Optioned;
use std::convert::TryFrom;

const fn is_py_ascii_whitespace(b: u8) -> bool {
    matches!(b, b'\t' | b'\n' | b'\x0C' | b'\r' | b' ' | b'\x0B')
}

#[derive(Debug, Clone, Copy)]
pub struct Request<'a, S> {
    pub string: S,
    pub start: usize,
    pub end: usize,
    pub pattern_codes: &'a [u32],
    pub match_all: bool,
    pub must_advance: bool,
}

impl<'a, S: StrDrive> Request<'a, S> {
    pub fn new(
        string: S,
        start: usize,
        end: usize,
        pattern_codes: &'a [u32],
        match_all: bool,
    ) -> Self {
        let end = std::cmp::min(end, string.count());
        let start = std::cmp::min(start, end);

        Self {
            string,
            start,
            end,
            pattern_codes,
            match_all,
            must_advance: false,
        }
    }
}

#[derive(Debug)]
pub struct Marks {
    last_index: isize,
    marks: Vec<Optioned<usize>>,
    marks_stack: Vec<(Vec<Optioned<usize>>, isize)>,
}

impl Default for Marks {
    fn default() -> Self {
        Self {
            last_index: -1,
            marks: Vec::new(),
            marks_stack: Vec::new(),
        }
    }
}

impl Marks {
    pub fn get(&self, group_index: usize) -> (Optioned<usize>, Optioned<usize>) {
        let marks_index = 2 * group_index;
        if marks_index + 1 < self.marks.len() {
            (self.marks[marks_index], self.marks[marks_index + 1])
        } else {
            (Optioned::none(), Optioned::none())
        }
    }

    pub fn last_index(&self) -> isize {
        self.last_index
    }

    pub fn raw(&self) -> &[Optioned<usize>] {
        self.marks.as_slice()
    }

    fn set(&mut self, mark_nr: usize, position: usize) {
        if mark_nr & 1 != 0 {
            self.last_index = mark_nr as isize / 2 + 1;
        }
        if mark_nr >= self.marks.len() {
            self.marks.resize(mark_nr + 1, Optioned::none());
        }
        self.marks[mark_nr] = Optioned::some(position);
    }

    fn push(&mut self) {
        self.marks_stack.push((self.marks.clone(), self.last_index));
    }

    fn pop(&mut self) {
        let (marks, last_index) = self.marks_stack.pop().unwrap();
        self.marks = marks;
        self.last_index = last_index;
    }

    fn pop_keep(&mut self) {
        let (marks, last_index) = self.marks_stack.last().unwrap().clone();
        self.marks = marks;
        self.last_index = last_index;
    }

    fn pop_discard(&mut self) {
        self.marks_stack.pop();
    }

    fn clear(&mut self) {
        self.last_index = -1;
        self.marks.clear();
        self.marks_stack.clear();
    }
}

#[derive(Debug, Default)]
pub struct State {
    pub start: usize,
    pub marks: Marks,
    pub string_position: usize,
    repeat_stack: Vec<RepeatContext>,
}

impl State {
    pub fn reset(&mut self, start: usize) {
        self.marks.clear();
        self.repeat_stack.clear();
        self.start = start;
        self.string_position = start;
    }

    pub fn pymatch<S: StrDrive>(&mut self, req: &Request<S>) -> bool {
        self.start = req.start;
        self.string_position = req.start;

        let ctx = MatchContext {
            string_position: req.start,
            string_offset: req.string.offset(0, req.start),
            code_position: 0,
            toplevel: true,
            jump: Jump::OpCode,
            repeat_ctx_id: usize::MAX,
            count: -1,
        };
        _match(&req, self, ctx)
    }

    pub fn search<S: StrDrive>(&mut self, mut req: Request<S>) -> bool {
        self.start = req.start;
        self.string_position = req.start;

        if req.start > req.end {
            return false;
        }

        let mut end = req.end;

        let mut start_offset = req.string.offset(0, req.start);

        let mut ctx = MatchContext {
            string_position: req.start,
            string_offset: start_offset,
            code_position: 0,
            toplevel: true,
            jump: Jump::OpCode,
            repeat_ctx_id: usize::MAX,
            count: -1,
        };

        if ctx.peek_code(&req, 0) == SreOpcode::INFO as u32 {
            /* optimization info block */
            /* <INFO> <1=skip> <2=flags> <3=min> <4=max> <5=prefix info>  */
            let min = ctx.peek_code(&req, 3) as usize;

            if ctx.remaining_chars(&req) < min {
                return false;
            }

            if min > 1 {
                /* adjust end point (but make sure we leave at least one
                character in there, so literal search will work) */
                // no overflow can happen as remaining chars >= min
                end -= min - 1;

                // adjust ctx position
                if end < ctx.string_position {
                    ctx.string_position = end;
                    ctx.string_offset = req.string.offset(0, ctx.string_position);
                }
            }

            let flags = SreInfo::from_bits_truncate(ctx.peek_code(&req, 2));

            if flags.contains(SreInfo::PREFIX) {
                if flags.contains(SreInfo::LITERAL) {
                    return search_info_literal::<true, S>(&mut req, self, ctx);
                } else {
                    return search_info_literal::<false, S>(&mut req, self, ctx);
                }
            } else if flags.contains(SreInfo::CHARSET) {
                return search_info_charset(&mut req, self, ctx);
            }
            // fallback to general search
        }

        if _match(&req, self, ctx) {
            return true;
        }

        req.must_advance = false;
        ctx.toplevel = false;
        while req.start < end {
            req.start += 1;
            start_offset = req.string.offset(start_offset, 1);
            self.reset(req.start);
            ctx.string_position = req.start;
            ctx.string_offset = start_offset;

            if _match(&req, self, ctx) {
                return true;
            }
        }
        false
    }
}

pub struct SearchIter<'a, S: StrDrive> {
    pub req: Request<'a, S>,
    pub state: State,
}

impl<'a, S: StrDrive> Iterator for SearchIter<'a, S> {
    type Item = ();

    fn next(&mut self) -> Option<Self::Item> {
        if self.req.start > self.req.end {
            return None;
        }

        self.state.reset(self.req.start);
        if !self.state.search(self.req) {
            return None;
        }

        self.req.must_advance = self.state.string_position == self.state.start;
        self.req.start = self.state.string_position;

        Some(())
    }
}

#[derive(Debug, Clone, Copy)]
enum Jump {
    OpCode,
    Assert1,
    AssertNot1,
    Branch1,
    Branch2,
    Repeat1,
    UntilBacktrace,
    MaxUntil2,
    MaxUntil3,
    MinUntil1,
    RepeatOne1,
    RepeatOne2,
    MinRepeatOne1,
    MinRepeatOne2,
    AtomicGroup1,
    PossessiveRepeat1,
    PossessiveRepeat2,
    PossessiveRepeat3,
    PossessiveRepeat4,
}

fn _match<S: StrDrive>(req: &Request<S>, state: &mut State, ctx: MatchContext) -> bool {
    let mut context_stack = vec![ctx];
    let mut popped_result = false;

    'coro: loop {
        let Some(mut ctx) = context_stack.pop() else {
            break;
        };

        popped_result = 'result: loop {
            let yield_ = 'context: loop {
                match ctx.jump {
                    Jump::OpCode => {}
                    Jump::Assert1 => {
                        if popped_result {
                            ctx.skip_code_from(req, 1);
                        } else {
                            break 'result false;
                        }
                    }
                    Jump::AssertNot1 => {
                        if popped_result {
                            break 'result false;
                        }
                        ctx.skip_code_from(req, 1);
                    }
                    Jump::Branch1 => {
                        let branch_offset = ctx.count as usize;
                        let next_length = ctx.peek_code(req, branch_offset) as isize;
                        if next_length == 0 {
                            state.marks.pop_discard();
                            break 'result false;
                        }
                        state.string_position = ctx.string_position;
                        let next_ctx = ctx.next_offset(branch_offset + 1, Jump::Branch2);
                        ctx.count += next_length;
                        break 'context next_ctx;
                    }
                    Jump::Branch2 => {
                        if popped_result {
                            break 'result true;
                        }
                        state.marks.pop_keep();
                        ctx.jump = Jump::Branch1;
                        continue 'context;
                    }
                    Jump::Repeat1 => {
                        state.repeat_stack.pop();
                        break 'result popped_result;
                    }
                    Jump::UntilBacktrace => {
                        if !popped_result {
                            state.repeat_stack[ctx.repeat_ctx_id].count -= 1;
                            state.string_position = ctx.string_position;
                        }
                        break 'result popped_result;
                    }
                    Jump::MaxUntil2 => {
                        let save_last_position = ctx.count as usize;
                        let repeat_ctx = &mut state.repeat_stack[ctx.repeat_ctx_id];
                        repeat_ctx.last_position = save_last_position;

                        if popped_result {
                            state.marks.pop_discard();
                            break 'result true;
                        }

                        state.marks.pop();
                        repeat_ctx.count -= 1;
                        state.string_position = ctx.string_position;

                        /* cannot match more repeated items here.  make sure the
                        tail matches */
                        let mut next_ctx = ctx.next_offset(1, Jump::MaxUntil3);
                        next_ctx.repeat_ctx_id = repeat_ctx.prev_id;
                        break 'context next_ctx;
                    }
                    Jump::MaxUntil3 => {
                        if !popped_result {
                            state.string_position = ctx.string_position;
                        }
                        break 'result popped_result;
                    }
                    Jump::MinUntil1 => {
                        if popped_result {
                            break 'result true;
                        }
                        ctx.repeat_ctx_id = ctx.count as usize;
                        let repeat_ctx = &mut state.repeat_stack[ctx.repeat_ctx_id];
                        state.string_position = ctx.string_position;
                        state.marks.pop();

                        // match more until tail matches
                        if repeat_ctx.count as usize >= repeat_ctx.max_count
                            && repeat_ctx.max_count != MAXREPEAT
                            || state.string_position == repeat_ctx.last_position
                        {
                            repeat_ctx.count -= 1;
                            break 'result false;
                        }

                        /* zero-width match protection */
                        repeat_ctx.last_position = state.string_position;

                        break 'context ctx
                            .next_at(repeat_ctx.code_position + 4, Jump::UntilBacktrace);
                    }
                    Jump::RepeatOne1 => {
                        let min_count = ctx.peek_code(req, 2) as isize;
                        let next_code = ctx.peek_code(req, ctx.peek_code(req, 1) as usize + 1);
                        if next_code == SreOpcode::LITERAL as u32 {
                            // Special case: Tail starts with a literal. Skip positions where
                            // the rest of the pattern cannot possibly match.
                            let c = ctx.peek_code(req, ctx.peek_code(req, 1) as usize + 2);
                            while ctx.at_end(req) || ctx.peek_char(req) != c {
                                if ctx.count <= min_count {
                                    state.marks.pop_discard();
                                    break 'result false;
                                }
                                ctx.back_skip_char(req, 1);
                                ctx.count -= 1;
                            }
                        }

                        state.string_position = ctx.string_position;
                        // General case: backtracking
                        break 'context ctx.next_peek_from(1, req, Jump::RepeatOne2);
                    }
                    Jump::RepeatOne2 => {
                        if popped_result {
                            break 'result true;
                        }

                        let min_count = ctx.peek_code(req, 2) as isize;
                        if ctx.count <= min_count {
                            state.marks.pop_discard();
                            break 'result false;
                        }

                        ctx.back_skip_char(req, 1);
                        ctx.count -= 1;

                        state.marks.pop_keep();
                        ctx.jump = Jump::RepeatOne1;
                        continue 'context;
                    }
                    Jump::MinRepeatOne1 => {
                        let max_count = ctx.peek_code(req, 3) as usize;
                        if max_count == MAXREPEAT || ctx.count as usize <= max_count {
                            state.string_position = ctx.string_position;
                            break 'context ctx.next_peek_from(1, req, Jump::MinRepeatOne2);
                        } else {
                            state.marks.pop_discard();
                            break 'result false;
                        }
                    }
                    Jump::MinRepeatOne2 => {
                        if popped_result {
                            break 'result true;
                        }

                        state.string_position = ctx.string_position;

                        let mut count_ctx = ctx;
                        count_ctx.skip_code(4);
                        if _count(req, state, count_ctx, 1) == 0 {
                            state.marks.pop_discard();
                            break 'result false;
                        }

                        ctx.skip_char(req, 1);
                        ctx.count += 1;
                        state.marks.pop_keep();
                        ctx.jump = Jump::MinRepeatOne1;
                        continue 'context;
                    }
                    Jump::AtomicGroup1 => {
                        if popped_result {
                            ctx.skip_code_from(req, 1);
                            ctx.string_position = state.string_position;
                            ctx.string_offset = req.string.offset(0, state.string_position);
                            // dispatch opcode
                        } else {
                            state.string_position = ctx.string_position;
                            break 'result false;
                        }
                    }
                    Jump::PossessiveRepeat1 => {
                        let min_count = ctx.peek_code(req, 2) as isize;
                        if ctx.count < min_count {
                            break 'context ctx.next_offset(4, Jump::PossessiveRepeat2);
                        }
                        // zero match protection
                        ctx.string_position = usize::MAX;
                        ctx.jump = Jump::PossessiveRepeat3;
                        continue 'context;
                    }
                    Jump::PossessiveRepeat2 => {
                        if popped_result {
                            ctx.count += 1;
                            ctx.jump = Jump::PossessiveRepeat1;
                            continue 'context;
                        } else {
                            state.string_position = ctx.string_position;
                            break 'result false;
                        }
                    }
                    Jump::PossessiveRepeat3 => {
                        let max_count = ctx.peek_code(req, 3) as usize;
                        if ((ctx.count as usize) < max_count || max_count == MAXREPEAT)
                            && ctx.string_position != state.string_position
                        {
                            state.marks.push();
                            ctx.string_position = state.string_position;
                            ctx.string_offset = req.string.offset(0, state.string_position);
                            break 'context ctx.next_offset(4, Jump::PossessiveRepeat4);
                        }
                        ctx.string_position = state.string_position;
                        ctx.string_offset = req.string.offset(0, state.string_position);
                        ctx.skip_code_from(req, 1);
                        ctx.skip_code(1);
                    }
                    Jump::PossessiveRepeat4 => {
                        if popped_result {
                            state.marks.pop_discard();
                            ctx.count += 1;
                            ctx.jump = Jump::PossessiveRepeat3;
                            continue 'context;
                        }
                        state.marks.pop();
                        state.string_position = ctx.string_position;
                        ctx.skip_code_from(req, 1);
                        ctx.skip_code(1);
                    }
                }
                ctx.jump = Jump::OpCode;

                loop {
                    macro_rules! general_op_literal {
                        ($f:expr) => {{
                            if ctx.at_end(req) || !$f(ctx.peek_code(req, 1), ctx.peek_char(req)) {
                                break 'result false;
                            }
                            ctx.skip_code(2);
                            ctx.skip_char(req, 1);
                        }};
                    }

                    macro_rules! general_op_in {
                        ($f:expr) => {{
                            if ctx.at_end(req) || !$f(&ctx.pattern(req)[2..], ctx.peek_char(req)) {
                                break 'result false;
                            }
                            ctx.skip_code_from(req, 1);
                            ctx.skip_char(req, 1);
                        }};
                    }

                    macro_rules! general_op_groupref {
                        ($f:expr) => {{
                            let (group_start, group_end) =
                                state.marks.get(ctx.peek_code(req, 1) as usize);
                            let (group_start, group_end) = if group_start.is_some()
                                && group_end.is_some()
                                && group_start.unpack() <= group_end.unpack()
                            {
                                (group_start.unpack(), group_end.unpack())
                            } else {
                                break 'result false;
                            };

                            let mut gctx = MatchContext {
                                string_position: group_start,
                                string_offset: req.string.offset(0, group_start),
                                ..ctx
                            };

                            for _ in group_start..group_end {
                                if ctx.at_end(req)
                                    || $f(ctx.peek_char(req)) != $f(gctx.peek_char(req))
                                {
                                    break 'result false;
                                }
                                ctx.skip_char(req, 1);
                                gctx.skip_char(req, 1);
                            }

                            ctx.skip_code(2);
                        }};
                    }

                    if ctx.remaining_codes(req) == 0 {
                        break 'result false;
                    }
                    let opcode = ctx.peek_code(req, 0);
                    let opcode = SreOpcode::try_from(opcode).unwrap();

                    match opcode {
                        SreOpcode::FAILURE => break 'result false,
                        SreOpcode::SUCCESS => {
                            if ctx.can_success(req) {
                                state.string_position = ctx.string_position;
                                break 'result true;
                            }
                            break 'result false;
                        }
                        SreOpcode::ANY => {
                            if ctx.at_end(req) || ctx.at_linebreak(req) {
                                break 'result false;
                            }
                            ctx.skip_code(1);
                            ctx.skip_char(req, 1);
                        }
                        SreOpcode::ANY_ALL => {
                            if ctx.at_end(req) {
                                break 'result false;
                            }
                            ctx.skip_code(1);
                            ctx.skip_char(req, 1);
                        }
                        /* <ASSERT> <skip> <back> <pattern> */
                        SreOpcode::ASSERT => {
                            let back = ctx.peek_code(req, 2) as usize;
                            if ctx.string_position < back {
                                break 'result false;
                            }

                            let mut next_ctx = ctx.next_offset(3, Jump::Assert1);
                            next_ctx.toplevel = false;
                            next_ctx.back_skip_char(req, back);
                            state.string_position = next_ctx.string_position;
                            break 'context next_ctx;
                        }
                        /* <ASSERT_NOT> <skip> <back> <pattern> */
                        SreOpcode::ASSERT_NOT => {
                            let back = ctx.peek_code(req, 2) as usize;
                            if ctx.string_position < back {
                                ctx.skip_code_from(req, 1);
                                continue;
                            }

                            let mut next_ctx = ctx.next_offset(3, Jump::AssertNot1);
                            next_ctx.toplevel = false;
                            next_ctx.back_skip_char(req, back);
                            state.string_position = next_ctx.string_position;
                            break 'context next_ctx;
                        }
                        SreOpcode::AT => {
                            let atcode = SreAtCode::try_from(ctx.peek_code(req, 1)).unwrap();
                            if at(req, &ctx, atcode) {
                                ctx.skip_code(2);
                            } else {
                                break 'result false;
                            }
                        }
                        // <BRANCH> <0=skip> code <JUMP> ... <NULL>
                        SreOpcode::BRANCH => {
                            state.marks.push();
                            ctx.count = 1;
                            ctx.jump = Jump::Branch1;
                            continue 'context;
                        }
                        SreOpcode::CATEGORY => {
                            let catcode = SreCatCode::try_from(ctx.peek_code(req, 1)).unwrap();
                            if ctx.at_end(req) || !category(catcode, ctx.peek_char(req)) {
                                break 'result false;
                            }
                            ctx.skip_code(2);
                            ctx.skip_char(req, 1);
                        }
                        SreOpcode::IN => general_op_in!(charset),
                        SreOpcode::IN_IGNORE => {
                            general_op_in!(|set, c| charset(set, lower_ascii(c)))
                        }
                        SreOpcode::IN_UNI_IGNORE => {
                            general_op_in!(|set, c| charset(set, lower_unicode(c)))
                        }
                        SreOpcode::IN_LOC_IGNORE => general_op_in!(charset_loc_ignore),
                        SreOpcode::MARK => {
                            state
                                .marks
                                .set(ctx.peek_code(req, 1) as usize, ctx.string_position);
                            ctx.skip_code(2);
                        }
                        SreOpcode::INFO | SreOpcode::JUMP => ctx.skip_code_from(req, 1),
                        /* <REPEAT> <skip> <1=min> <2=max> item <UNTIL> tail */
                        SreOpcode::REPEAT => {
                            let repeat_ctx = RepeatContext {
                                count: -1,
                                min_count: ctx.peek_code(req, 2) as usize,
                                max_count: ctx.peek_code(req, 3) as usize,
                                code_position: ctx.code_position,
                                last_position: std::usize::MAX,
                                prev_id: ctx.repeat_ctx_id,
                            };
                            state.repeat_stack.push(repeat_ctx);
                            let repeat_ctx_id = state.repeat_stack.len() - 1;
                            state.string_position = ctx.string_position;
                            let mut next_ctx = ctx.next_peek_from(1, req, Jump::Repeat1);
                            next_ctx.repeat_ctx_id = repeat_ctx_id;
                            break 'context next_ctx;
                        }
                        SreOpcode::MAX_UNTIL => {
                            let repeat_ctx = &mut state.repeat_stack[ctx.repeat_ctx_id];
                            state.string_position = ctx.string_position;
                            repeat_ctx.count += 1;

                            if (repeat_ctx.count as usize) < repeat_ctx.min_count {
                                // not enough matches
                                break 'context ctx
                                    .next_at(repeat_ctx.code_position + 4, Jump::UntilBacktrace);
                            }

                            if ((repeat_ctx.count as usize) < repeat_ctx.max_count
                                || repeat_ctx.max_count == MAXREPEAT)
                                && state.string_position != repeat_ctx.last_position
                            {
                                /* we may have enough matches, but if we can
                                match another item, do so */
                                state.marks.push();
                                ctx.count = repeat_ctx.last_position as isize;
                                repeat_ctx.last_position = state.string_position;

                                break 'context ctx
                                    .next_at(repeat_ctx.code_position + 4, Jump::MaxUntil2);
                            }

                            /* cannot match more repeated items here.  make sure the
                            tail matches */
                            let mut next_ctx = ctx.next_offset(1, Jump::MaxUntil3);
                            next_ctx.repeat_ctx_id = repeat_ctx.prev_id;
                            break 'context next_ctx;
                        }
                        SreOpcode::MIN_UNTIL => {
                            let repeat_ctx = state.repeat_stack.last_mut().unwrap();
                            state.string_position = ctx.string_position;
                            repeat_ctx.count += 1;

                            if (repeat_ctx.count as usize) < repeat_ctx.min_count {
                                // not enough matches
                                break 'context ctx
                                    .next_at(repeat_ctx.code_position + 4, Jump::UntilBacktrace);
                            }

                            state.marks.push();
                            ctx.count = ctx.repeat_ctx_id as isize;
                            let mut next_ctx = ctx.next_offset(1, Jump::MinUntil1);
                            next_ctx.repeat_ctx_id = repeat_ctx.prev_id;
                            break 'context next_ctx;
                        }
                        /* <REPEAT_ONE> <skip> <1=min> <2=max> item <SUCCESS> tail */
                        SreOpcode::REPEAT_ONE => {
                            let min_count = ctx.peek_code(req, 2) as usize;
                            let max_count = ctx.peek_code(req, 3) as usize;

                            if ctx.remaining_chars(req) < min_count {
                                break 'result false;
                            }

                            state.string_position = ctx.string_position;

                            let mut next_ctx = ctx;
                            next_ctx.skip_code(4);
                            let count = _count(req, state, next_ctx, max_count);
                            ctx.skip_char(req, count);
                            if count < min_count {
                                break 'result false;
                            }

                            let next_code = ctx.peek_code(req, ctx.peek_code(req, 1) as usize + 1);
                            if next_code == SreOpcode::SUCCESS as u32 && ctx.can_success(req) {
                                // tail is empty. we're finished
                                state.string_position = ctx.string_position;
                                break 'result true;
                            }

                            state.marks.push();
                            ctx.count = count as isize;
                            ctx.jump = Jump::RepeatOne1;
                            continue 'context;
                        }
                        /* <MIN_REPEAT_ONE> <skip> <1=min> <2=max> item <SUCCESS> tail */
                        SreOpcode::MIN_REPEAT_ONE => {
                            let min_count = ctx.peek_code(req, 2) as usize;
                            if ctx.remaining_chars(req) < min_count {
                                break 'result false;
                            }

                            state.string_position = ctx.string_position;
                            ctx.count = if min_count == 0 {
                                0
                            } else {
                                let mut count_ctx = ctx;
                                count_ctx.skip_code(4);
                                let count = _count(req, state, count_ctx, min_count);
                                if count < min_count {
                                    break 'result false;
                                }
                                ctx.skip_char(req, count);
                                count as isize
                            };

                            let next_code = ctx.peek_code(req, ctx.peek_code(req, 1) as usize + 1);
                            if next_code == SreOpcode::SUCCESS as u32 && ctx.can_success(req) {
                                // tail is empty. we're finished
                                state.string_position = ctx.string_position;
                                break 'result true;
                            }

                            state.marks.push();
                            ctx.jump = Jump::MinRepeatOne1;
                            continue 'context;
                        }
                        SreOpcode::LITERAL => general_op_literal!(|code, c| code == c),
                        SreOpcode::NOT_LITERAL => general_op_literal!(|code, c| code != c),
                        SreOpcode::LITERAL_IGNORE => {
                            general_op_literal!(|code, c| code == lower_ascii(c))
                        }
                        SreOpcode::NOT_LITERAL_IGNORE => {
                            general_op_literal!(|code, c| code != lower_ascii(c))
                        }
                        SreOpcode::LITERAL_UNI_IGNORE => {
                            general_op_literal!(|code, c| code == lower_unicode(c))
                        }
                        SreOpcode::NOT_LITERAL_UNI_IGNORE => {
                            general_op_literal!(|code, c| code != lower_unicode(c))
                        }
                        SreOpcode::LITERAL_LOC_IGNORE => general_op_literal!(char_loc_ignore),
                        SreOpcode::NOT_LITERAL_LOC_IGNORE => {
                            general_op_literal!(|code, c| !char_loc_ignore(code, c))
                        }
                        SreOpcode::GROUPREF => general_op_groupref!(|x| x),
                        SreOpcode::GROUPREF_IGNORE => general_op_groupref!(lower_ascii),
                        SreOpcode::GROUPREF_LOC_IGNORE => general_op_groupref!(lower_locate),
                        SreOpcode::GROUPREF_UNI_IGNORE => general_op_groupref!(lower_unicode),
                        SreOpcode::GROUPREF_EXISTS => {
                            let (group_start, group_end) =
                                state.marks.get(ctx.peek_code(req, 1) as usize);
                            if group_start.is_some()
                                && group_end.is_some()
                                && group_start.unpack() <= group_end.unpack()
                            {
                                ctx.skip_code(3);
                            } else {
                                ctx.skip_code_from(req, 2)
                            }
                        }
                        /* <ATOMIC_GROUP> <skip> pattern <SUCCESS> tail */
                        SreOpcode::ATOMIC_GROUP => {
                            state.string_position = ctx.string_position;
                            break 'context ctx.next_offset(2, Jump::AtomicGroup1);
                        }
                        /* <POSSESSIVE_REPEAT> <skip> <1=min> <2=max> pattern
                        <SUCCESS> tail */
                        SreOpcode::POSSESSIVE_REPEAT => {
                            state.string_position = ctx.string_position;
                            ctx.count = 0;
                            ctx.jump = Jump::PossessiveRepeat1;
                            continue 'context;
                        }
                        /* <POSSESSIVE_REPEAT_ONE> <skip> <1=min> <2=max> item <SUCCESS>
                        tail */
                        SreOpcode::POSSESSIVE_REPEAT_ONE => {
                            let min_count = ctx.peek_code(req, 2) as usize;
                            let max_count = ctx.peek_code(req, 3) as usize;
                            if ctx.remaining_chars(req) < min_count {
                                break 'result false;
                            }
                            state.string_position = ctx.string_position;
                            let mut count_ctx = ctx;
                            count_ctx.skip_code(4);
                            let count = _count(req, state, count_ctx, max_count);
                            if count < min_count {
                                break 'result false;
                            }
                            ctx.skip_char(req, count);
                            ctx.skip_code_from(req, 1);
                        }
                        SreOpcode::CHARSET
                        | SreOpcode::BIGCHARSET
                        | SreOpcode::NEGATE
                        | SreOpcode::RANGE
                        | SreOpcode::RANGE_UNI_IGNORE
                        | SreOpcode::SUBPATTERN => {
                            unreachable!("unexpected opcode on main dispatch")
                        }
                    }
                }
            };
            context_stack.push(ctx);
            context_stack.push(yield_);
            continue 'coro;
        };
    }
    popped_result
}

fn search_info_literal<const LITERAL: bool, S: StrDrive>(
    req: &mut Request<S>,
    state: &mut State,
    mut ctx: MatchContext,
) -> bool {
    /* pattern starts with a known prefix */
    /* <length> <skip> <prefix data> <overlap data> */
    let len = ctx.peek_code(req, 5) as usize;
    let skip = ctx.peek_code(req, 6) as usize;
    let prefix = &ctx.pattern(req)[7..7 + len];
    let overlap = &ctx.pattern(req)[7 + len - 1..7 + len * 2];

    // code_position ready for tail match
    ctx.skip_code_from(req, 1);
    ctx.skip_code(2 * skip);

    req.must_advance = false;

    if len == 1 {
        // pattern starts with a literal character
        let c = prefix[0];

        while !ctx.at_end(req) {
            // find the next matched literal
            while ctx.peek_char(req) != c {
                ctx.skip_char(req, 1);
                if ctx.at_end(req) {
                    return false;
                }
            }

            req.start = ctx.string_position;
            state.start = ctx.string_position;
            state.string_position = ctx.string_position + skip;

            // literal only
            if LITERAL {
                return true;
            }

            let mut next_ctx = ctx;
            next_ctx.skip_char(req, skip);

            if _match(req, state, next_ctx) {
                return true;
            }

            ctx.skip_char(req, 1);
            state.marks.clear();
        }
    } else {
        while !ctx.at_end(req) {
            let c = prefix[0];
            while ctx.peek_char(req) != c {
                ctx.skip_char(req, 1);
                if ctx.at_end(req) {
                    return false;
                }
            }
            ctx.skip_char(req, 1);
            if ctx.at_end(req) {
                return false;
            }

            let mut i = 1;
            loop {
                if ctx.peek_char(req) == prefix[i] {
                    i += 1;
                    if i != len {
                        ctx.skip_char(req, 1);
                        if ctx.at_end(req) {
                            return false;
                        }
                        continue;
                    }

                    req.start = ctx.string_position - (len - 1);
                    state.start = req.start;
                    state.string_position = state.start + skip;

                    // literal only
                    if LITERAL {
                        return true;
                    }

                    let mut next_ctx = ctx;
                    if skip != 0 {
                        next_ctx.skip_char(req, 1);
                    } else {
                        next_ctx.string_position = state.string_position;
                        next_ctx.string_offset = req.string.offset(0, state.string_position);
                    }

                    if _match(req, state, next_ctx) {
                        return true;
                    }

                    ctx.skip_char(req, 1);
                    if ctx.at_end(req) {
                        return false;
                    }
                    state.marks.clear();
                }

                i = overlap[i] as usize;
                if i == 0 {
                    break;
                }
            }
        }
    }
    false
}

fn search_info_charset<S: StrDrive>(
    req: &mut Request<S>,
    state: &mut State,
    mut ctx: MatchContext,
) -> bool {
    let set = &ctx.pattern(req)[5..];

    ctx.skip_code_from(req, 1);

    req.must_advance = false;

    loop {
        while !ctx.at_end(req) && !charset(set, ctx.peek_char(req)) {
            ctx.skip_char(req, 1);
        }
        if ctx.at_end(req) {
            return false;
        }

        req.start = ctx.string_position;
        state.start = ctx.string_position;
        state.string_position = ctx.string_position;

        if _match(req, state, ctx) {
            return true;
        }

        ctx.skip_char(req, 1);
        state.marks.clear();
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

pub trait StrDrive: Copy {
    fn offset(&self, offset: usize, skip: usize) -> usize;
    fn count(&self) -> usize;
    fn peek(&self, offset: usize) -> u32;
    fn back_peek(&self, offset: usize) -> u32;
    fn back_offset(&self, offset: usize, skip: usize) -> usize;
}

impl StrDrive for &str {
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

#[derive(Clone, Copy)]
struct MatchContext {
    string_position: usize,
    string_offset: usize,
    code_position: usize,
    toplevel: bool,
    jump: Jump,
    repeat_ctx_id: usize,
    count: isize,
}

impl MatchContext {
    fn pattern<'a, S>(&self, req: &Request<'a, S>) -> &'a [u32] {
        &req.pattern_codes[self.code_position..]
    }

    fn remaining_codes<S>(&self, req: &Request<S>) -> usize {
        req.pattern_codes.len() - self.code_position
    }

    fn remaining_chars<S>(&self, req: &Request<S>) -> usize {
        req.end - self.string_position
    }

    fn peek_char<S: StrDrive>(&self, req: &Request<S>) -> u32 {
        req.string.peek(self.string_offset)
    }

    fn skip_char<S: StrDrive>(&mut self, req: &Request<S>, skip: usize) {
        self.string_position += skip;
        self.string_offset = req.string.offset(self.string_offset, skip);
    }

    fn back_peek_char<S: StrDrive>(&self, req: &Request<S>) -> u32 {
        req.string.back_peek(self.string_offset)
    }

    fn back_skip_char<S: StrDrive>(&mut self, req: &Request<S>, skip: usize) {
        self.string_position -= skip;
        self.string_offset = req.string.back_offset(self.string_offset, skip);
    }

    fn peek_code<S>(&self, req: &Request<S>, peek: usize) -> u32 {
        req.pattern_codes[self.code_position + peek]
    }

    fn skip_code(&mut self, skip: usize) {
        self.code_position += skip;
    }

    fn skip_code_from<S>(&mut self, req: &Request<S>, peek: usize) {
        self.skip_code(self.peek_code(req, peek) as usize + 1);
    }

    fn at_beginning(&self) -> bool {
        // self.ctx().string_position == self.state().start
        self.string_position == 0
    }

    fn at_end<S>(&self, req: &Request<S>) -> bool {
        self.string_position == req.end
    }

    fn at_linebreak<S: StrDrive>(&self, req: &Request<S>) -> bool {
        !self.at_end(req) && is_linebreak(self.peek_char(req))
    }

    fn at_boundary<S: StrDrive, F: FnMut(u32) -> bool>(
        &self,
        req: &Request<S>,
        mut word_checker: F,
    ) -> bool {
        if self.at_beginning() && self.at_end(req) {
            return false;
        }
        let that = !self.at_beginning() && word_checker(self.back_peek_char(req));
        let this = !self.at_end(req) && word_checker(self.peek_char(req));
        this != that
    }

    fn at_non_boundary<S: StrDrive, F: FnMut(u32) -> bool>(
        &self,
        req: &Request<S>,
        mut word_checker: F,
    ) -> bool {
        if self.at_beginning() && self.at_end(req) {
            return false;
        }
        let that = !self.at_beginning() && word_checker(self.back_peek_char(req));
        let this = !self.at_end(req) && word_checker(self.peek_char(req));
        this == that
    }

    fn can_success<S>(&self, req: &Request<S>) -> bool {
        if !self.toplevel {
            return true;
        }
        if req.match_all && !self.at_end(req) {
            return false;
        }
        if req.must_advance && self.string_position == req.start {
            return false;
        }
        true
    }

    #[must_use]
    fn next_peek_from<S>(&mut self, peek: usize, req: &Request<S>, jump: Jump) -> Self {
        self.next_offset(self.peek_code(req, peek) as usize + 1, jump)
    }

    #[must_use]
    fn next_offset(&mut self, offset: usize, jump: Jump) -> Self {
        self.next_at(self.code_position + offset, jump)
    }

    #[must_use]
    fn next_at(&mut self, code_position: usize, jump: Jump) -> Self {
        self.jump = jump;
        MatchContext {
            code_position,
            jump: Jump::OpCode,
            count: -1,
            ..*self
        }
    }
}

fn at<S: StrDrive>(req: &Request<S>, ctx: &MatchContext, atcode: SreAtCode) -> bool {
    match atcode {
        SreAtCode::BEGINNING | SreAtCode::BEGINNING_STRING => ctx.at_beginning(),
        SreAtCode::BEGINNING_LINE => ctx.at_beginning() || is_linebreak(ctx.back_peek_char(req)),
        SreAtCode::BOUNDARY => ctx.at_boundary(req, is_word),
        SreAtCode::NON_BOUNDARY => ctx.at_non_boundary(req, is_word),
        SreAtCode::END => {
            (ctx.remaining_chars(req) == 1 && ctx.at_linebreak(req)) || ctx.at_end(req)
        }
        SreAtCode::END_LINE => ctx.at_linebreak(req) || ctx.at_end(req),
        SreAtCode::END_STRING => ctx.at_end(req),
        SreAtCode::LOC_BOUNDARY => ctx.at_boundary(req, is_loc_word),
        SreAtCode::LOC_NON_BOUNDARY => ctx.at_non_boundary(req, is_loc_word),
        SreAtCode::UNI_BOUNDARY => ctx.at_boundary(req, is_uni_word),
        SreAtCode::UNI_NON_BOUNDARY => ctx.at_non_boundary(req, is_uni_word),
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

fn _count<S: StrDrive>(
    req: &Request<S>,
    state: &mut State,
    mut ctx: MatchContext,
    max_count: usize,
) -> usize {
    let max_count = std::cmp::min(max_count, ctx.remaining_chars(req));
    let end = ctx.string_position + max_count;
    let opcode = SreOpcode::try_from(ctx.peek_code(req, 0)).unwrap();

    match opcode {
        SreOpcode::ANY => {
            while ctx.string_position < end && !ctx.at_linebreak(req) {
                ctx.skip_char(req, 1);
            }
        }
        SreOpcode::ANY_ALL => {
            ctx.skip_char(req, max_count);
        }
        SreOpcode::IN => {
            while ctx.string_position < end && charset(&ctx.pattern(req)[2..], ctx.peek_char(req)) {
                ctx.skip_char(req, 1);
            }
        }
        SreOpcode::LITERAL => {
            general_count_literal(req, &mut ctx, end, |code, c| code == c as u32);
        }
        SreOpcode::NOT_LITERAL => {
            general_count_literal(req, &mut ctx, end, |code, c| code != c as u32);
        }
        SreOpcode::LITERAL_IGNORE => {
            general_count_literal(req, &mut ctx, end, |code, c| code == lower_ascii(c) as u32);
        }
        SreOpcode::NOT_LITERAL_IGNORE => {
            general_count_literal(req, &mut ctx, end, |code, c| code != lower_ascii(c) as u32);
        }
        SreOpcode::LITERAL_LOC_IGNORE => {
            general_count_literal(req, &mut ctx, end, char_loc_ignore);
        }
        SreOpcode::NOT_LITERAL_LOC_IGNORE => {
            general_count_literal(req, &mut ctx, end, |code, c| !char_loc_ignore(code, c));
        }
        SreOpcode::LITERAL_UNI_IGNORE => {
            general_count_literal(req, &mut ctx, end, |code, c| {
                code == lower_unicode(c) as u32
            });
        }
        SreOpcode::NOT_LITERAL_UNI_IGNORE => {
            general_count_literal(req, &mut ctx, end, |code, c| {
                code != lower_unicode(c) as u32
            });
        }
        _ => {
            /* General case */
            let mut count = 0;

            while count < max_count {
                let sub_ctx = MatchContext {
                    toplevel: false,
                    jump: Jump::OpCode,
                    repeat_ctx_id: usize::MAX,
                    count: -1,
                    ..ctx
                };
                if !_match(req, state, sub_ctx) {
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

fn general_count_literal<S: StrDrive, F: FnMut(u32, u32) -> bool>(
    req: &Request<S>,
    ctx: &mut MatchContext,
    end: usize,
    mut f: F,
) {
    let ch = ctx.peek_code(req, 1);
    while ctx.string_position < end && f(ch, ctx.peek_char(req)) {
        ctx.skip_char(req, 1);
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
