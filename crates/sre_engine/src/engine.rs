// good luck to those that follow; here be dragons

use crate::string::{
    is_digit, is_linebreak, is_loc_word, is_space, is_uni_digit, is_uni_linebreak, is_uni_space,
    is_uni_word, is_word, lower_ascii, lower_locate, lower_unicode, upper_locate, upper_unicode,
};

use super::{MAXREPEAT, SreAtCode, SreCatCode, SreInfo, SreOpcode, StrDrive, StringCursor};
use optional::Optioned;
use std::{convert::TryFrom, ptr::null};

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

    pub const fn last_index(&self) -> isize {
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
    pub cursor: StringCursor,
    repeat_stack: Vec<RepeatContext>,
}

impl State {
    pub fn reset<S: StrDrive>(&mut self, req: &Request<'_, S>, start: usize) {
        self.marks.clear();
        self.repeat_stack.clear();
        self.start = start;
        req.string.adjust_cursor(&mut self.cursor, start);
    }

    pub fn py_match<S: StrDrive>(&mut self, req: &Request<'_, S>) -> bool {
        self.start = req.start;
        req.string.adjust_cursor(&mut self.cursor, self.start);

        let ctx = MatchContext {
            cursor: self.cursor,
            code_position: 0,
            toplevel: true,
            jump: Jump::OpCode,
            repeat_ctx_id: usize::MAX,
            count: -1,
        };
        _match(req, self, ctx)
    }

    pub fn search<S: StrDrive>(&mut self, mut req: Request<'_, S>) -> bool {
        self.start = req.start;
        req.string.adjust_cursor(&mut self.cursor, self.start);

        if req.start > req.end {
            return false;
        }

        let mut end = req.end;

        let mut ctx = MatchContext {
            cursor: self.cursor,
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
                if end < ctx.cursor.position {
                    let skip = end - self.cursor.position;
                    S::skip(&mut self.cursor, skip);
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
            // skip OP INFO
            ctx.skip_code_from(&req, 1);
        }

        if _match(&req, self, ctx) {
            return true;
        }

        if ctx.try_peek_code_as::<SreOpcode, _>(&req, 0).unwrap() == SreOpcode::AT
            && (ctx.try_peek_code_as::<SreAtCode, _>(&req, 1).unwrap() == SreAtCode::BEGINNING
                || ctx.try_peek_code_as::<SreAtCode, _>(&req, 1).unwrap()
                    == SreAtCode::BEGINNING_STRING)
        {
            self.cursor.position = req.end;
            self.cursor.ptr = null();
            // self.reset(&req, req.end);
            return false;
        }

        req.must_advance = false;
        ctx.toplevel = false;
        while req.start < end {
            req.start += 1;
            self.reset(&req, req.start);
            ctx.cursor = self.cursor;

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

impl<S: StrDrive> Iterator for SearchIter<'_, S> {
    type Item = ();

    fn next(&mut self) -> Option<Self::Item> {
        if self.req.start > self.req.end {
            return None;
        }

        self.state.reset(&self.req, self.req.start);
        if !self.state.search(self.req) {
            return None;
        }

        self.req.must_advance = self.state.cursor.position == self.state.start;
        self.req.start = self.state.cursor.position;

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

fn _match<S: StrDrive>(req: &Request<'_, S>, state: &mut State, mut ctx: MatchContext) -> bool {
    let mut context_stack = vec![];
    let mut popped_result = false;

    // NOTE: 'result loop is not an actual loop but break label
    #[allow(clippy::never_loop)]
    'coro: loop {
        popped_result = 'result: loop {
            let yielded = 'context: loop {
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
                        state.marks.pop();
                        ctx.skip_code_from(req, 1);
                    }
                    Jump::Branch1 => {
                        let branch_offset = ctx.count as usize;
                        let next_length = ctx.peek_code(req, branch_offset) as isize;
                        if next_length == 0 {
                            state.marks.pop_discard();
                            break 'result false;
                        }
                        state.cursor = ctx.cursor;
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
                            state.cursor = ctx.cursor;
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
                        state.cursor = ctx.cursor;

                        /* cannot match more repeated items here.  make sure the
                        tail matches */
                        let mut next_ctx = ctx.next_offset(1, Jump::MaxUntil3);
                        next_ctx.repeat_ctx_id = repeat_ctx.prev_id;
                        break 'context next_ctx;
                    }
                    Jump::MaxUntil3 => {
                        if !popped_result {
                            state.cursor = ctx.cursor;
                        }
                        break 'result popped_result;
                    }
                    Jump::MinUntil1 => {
                        if popped_result {
                            break 'result true;
                        }
                        ctx.repeat_ctx_id = ctx.count as usize;
                        let repeat_ctx = &mut state.repeat_stack[ctx.repeat_ctx_id];
                        state.cursor = ctx.cursor;
                        state.marks.pop();

                        // match more until tail matches
                        if repeat_ctx.count as usize >= repeat_ctx.max_count
                            && repeat_ctx.max_count != MAXREPEAT
                            || state.cursor.position == repeat_ctx.last_position
                        {
                            repeat_ctx.count -= 1;
                            break 'result false;
                        }

                        /* zero-width match protection */
                        repeat_ctx.last_position = state.cursor.position;

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
                            while ctx.at_end(req) || ctx.peek_char::<S>() != c {
                                if ctx.count <= min_count {
                                    state.marks.pop_discard();
                                    break 'result false;
                                }
                                ctx.back_advance_char::<S>();
                                ctx.count -= 1;
                            }
                        }

                        state.cursor = ctx.cursor;
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

                        ctx.back_advance_char::<S>();
                        ctx.count -= 1;

                        state.marks.pop_keep();
                        ctx.jump = Jump::RepeatOne1;
                        continue 'context;
                    }
                    Jump::MinRepeatOne1 => {
                        let max_count = ctx.peek_code(req, 3) as usize;
                        if max_count == MAXREPEAT || ctx.count as usize <= max_count {
                            state.cursor = ctx.cursor;
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

                        state.cursor = ctx.cursor;

                        let mut count_ctx = ctx;
                        count_ctx.skip_code(4);
                        if _count(req, state, &mut count_ctx, 1) == 0 {
                            state.marks.pop_discard();
                            break 'result false;
                        }

                        ctx.advance_char::<S>();
                        ctx.count += 1;
                        state.marks.pop_keep();
                        ctx.jump = Jump::MinRepeatOne1;
                        continue 'context;
                    }
                    Jump::AtomicGroup1 => {
                        if popped_result {
                            ctx.skip_code_from(req, 1);
                            ctx.cursor = state.cursor;
                            // dispatch opcode
                        } else {
                            state.cursor = ctx.cursor;
                            break 'result false;
                        }
                    }
                    Jump::PossessiveRepeat1 => {
                        let min_count = ctx.peek_code(req, 2) as isize;
                        if ctx.count < min_count {
                            break 'context ctx.next_offset(4, Jump::PossessiveRepeat2);
                        }
                        // zero match protection
                        ctx.cursor.position = usize::MAX;
                        ctx.jump = Jump::PossessiveRepeat3;
                        continue 'context;
                    }
                    Jump::PossessiveRepeat2 => {
                        if popped_result {
                            ctx.count += 1;
                            ctx.jump = Jump::PossessiveRepeat1;
                            continue 'context;
                        } else {
                            state.cursor = ctx.cursor;
                            break 'result false;
                        }
                    }
                    Jump::PossessiveRepeat3 => {
                        let max_count = ctx.peek_code(req, 3) as usize;
                        if ((ctx.count as usize) < max_count || max_count == MAXREPEAT)
                            && ctx.cursor.position != state.cursor.position
                        {
                            state.marks.push();
                            ctx.cursor = state.cursor;
                            break 'context ctx.next_offset(4, Jump::PossessiveRepeat4);
                        }
                        ctx.cursor = state.cursor;
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
                        state.cursor = ctx.cursor;
                        ctx.skip_code_from(req, 1);
                        ctx.skip_code(1);
                    }
                }
                ctx.jump = Jump::OpCode;

                loop {
                    macro_rules! general_op_literal {
                        ($f:expr) => {{
                            #[allow(clippy::redundant_closure_call)]
                            if ctx.at_end(req) || !$f(ctx.peek_code(req, 1), ctx.peek_char::<S>()) {
                                break 'result false;
                            }
                            ctx.skip_code(2);
                            ctx.advance_char::<S>();
                        }};
                    }

                    macro_rules! general_op_in {
                        ($f:expr) => {{
                            #[allow(clippy::redundant_closure_call)]
                            if ctx.at_end(req) || !$f(&ctx.pattern(req)[2..], ctx.peek_char::<S>())
                            {
                                break 'result false;
                            }
                            ctx.skip_code_from(req, 1);
                            ctx.advance_char::<S>();
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

                            let mut g_ctx = MatchContext {
                                cursor: req.string.create_cursor(group_start),
                                ..ctx
                            };

                            for _ in group_start..group_end {
                                #[allow(clippy::redundant_closure_call)]
                                if ctx.at_end(req)
                                    || $f(ctx.peek_char::<S>()) != $f(g_ctx.peek_char::<S>())
                                {
                                    break 'result false;
                                }
                                ctx.advance_char::<S>();
                                g_ctx.advance_char::<S>();
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
                                state.cursor = ctx.cursor;
                                break 'result true;
                            }
                            break 'result false;
                        }
                        SreOpcode::ANY => {
                            if ctx.at_end(req) || ctx.at_linebreak(req) {
                                break 'result false;
                            }
                            ctx.skip_code(1);
                            ctx.advance_char::<S>();
                        }
                        SreOpcode::ANY_ALL => {
                            if ctx.at_end(req) {
                                break 'result false;
                            }
                            ctx.skip_code(1);
                            ctx.advance_char::<S>();
                        }
                        /* <ASSERT> <skip> <back> <pattern> */
                        SreOpcode::ASSERT => {
                            let back = ctx.peek_code(req, 2) as usize;
                            if ctx.cursor.position < back {
                                break 'result false;
                            }

                            let mut next_ctx = ctx.next_offset(3, Jump::Assert1);
                            next_ctx.toplevel = false;
                            next_ctx.back_skip_char::<S>(back);
                            state.cursor = next_ctx.cursor;
                            break 'context next_ctx;
                        }
                        /* <ASSERT_NOT> <skip> <back> <pattern> */
                        SreOpcode::ASSERT_NOT => {
                            let back = ctx.peek_code(req, 2) as usize;
                            if ctx.cursor.position < back {
                                ctx.skip_code_from(req, 1);
                                continue;
                            }
                            state.marks.push();

                            let mut next_ctx = ctx.next_offset(3, Jump::AssertNot1);
                            next_ctx.toplevel = false;
                            next_ctx.back_skip_char::<S>(back);
                            state.cursor = next_ctx.cursor;
                            break 'context next_ctx;
                        }
                        SreOpcode::AT => {
                            let at_code = SreAtCode::try_from(ctx.peek_code(req, 1)).unwrap();
                            if at(req, &ctx, at_code) {
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
                            let cat_code = SreCatCode::try_from(ctx.peek_code(req, 1)).unwrap();
                            if ctx.at_end(req) || !category(cat_code, ctx.peek_char::<S>()) {
                                break 'result false;
                            }
                            ctx.skip_code(2);
                            ctx.advance_char::<S>();
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
                                .set(ctx.peek_code(req, 1) as usize, ctx.cursor.position);
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
                                last_position: usize::MAX,
                                prev_id: ctx.repeat_ctx_id,
                            };
                            state.repeat_stack.push(repeat_ctx);
                            let repeat_ctx_id = state.repeat_stack.len() - 1;
                            state.cursor = ctx.cursor;
                            let mut next_ctx = ctx.next_peek_from(1, req, Jump::Repeat1);
                            next_ctx.repeat_ctx_id = repeat_ctx_id;
                            break 'context next_ctx;
                        }
                        SreOpcode::MAX_UNTIL => {
                            let repeat_ctx = &mut state.repeat_stack[ctx.repeat_ctx_id];
                            state.cursor = ctx.cursor;
                            repeat_ctx.count += 1;

                            if (repeat_ctx.count as usize) < repeat_ctx.min_count {
                                // not enough matches
                                break 'context ctx
                                    .next_at(repeat_ctx.code_position + 4, Jump::UntilBacktrace);
                            }

                            if ((repeat_ctx.count as usize) < repeat_ctx.max_count
                                || repeat_ctx.max_count == MAXREPEAT)
                                && state.cursor.position != repeat_ctx.last_position
                            {
                                /* we may have enough matches, but if we can
                                match another item, do so */
                                state.marks.push();
                                ctx.count = repeat_ctx.last_position as isize;
                                repeat_ctx.last_position = state.cursor.position;

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
                            state.cursor = ctx.cursor;
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

                            state.cursor = ctx.cursor;

                            let mut count_ctx = ctx;
                            count_ctx.skip_code(4);
                            let count = _count(req, state, &mut count_ctx, max_count);
                            if count < min_count {
                                break 'result false;
                            }
                            ctx.cursor = count_ctx.cursor;

                            let next_code = ctx.peek_code(req, ctx.peek_code(req, 1) as usize + 1);
                            if next_code == SreOpcode::SUCCESS as u32 && ctx.can_success(req) {
                                // tail is empty. we're finished
                                state.cursor = ctx.cursor;
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

                            state.cursor = ctx.cursor;
                            ctx.count = if min_count == 0 {
                                0
                            } else {
                                let mut count_ctx = ctx;
                                count_ctx.skip_code(4);
                                let count = _count(req, state, &mut count_ctx, min_count);
                                if count < min_count {
                                    break 'result false;
                                }
                                ctx.cursor = count_ctx.cursor;
                                count as isize
                            };

                            let next_code = ctx.peek_code(req, ctx.peek_code(req, 1) as usize + 1);
                            if next_code == SreOpcode::SUCCESS as u32 && ctx.can_success(req) {
                                // tail is empty. we're finished
                                state.cursor = ctx.cursor;
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
                            state.cursor = ctx.cursor;
                            break 'context ctx.next_offset(2, Jump::AtomicGroup1);
                        }
                        /* <POSSESSIVE_REPEAT> <skip> <1=min> <2=max> pattern
                        <SUCCESS> tail */
                        SreOpcode::POSSESSIVE_REPEAT => {
                            state.cursor = ctx.cursor;
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
                            state.cursor = ctx.cursor;
                            let mut count_ctx = ctx;
                            count_ctx.skip_code(4);
                            let count = _count(req, state, &mut count_ctx, max_count);
                            if count < min_count {
                                break 'result false;
                            }
                            ctx.cursor = count_ctx.cursor;
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
            ctx = yielded;
            continue 'coro;
        };
        if let Some(popped_ctx) = context_stack.pop() {
            ctx = popped_ctx;
        } else {
            break;
        }
    }
    popped_result
}

fn search_info_literal<const LITERAL: bool, S: StrDrive>(
    req: &mut Request<'_, S>,
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
            while ctx.peek_char::<S>() != c {
                ctx.advance_char::<S>();
                if ctx.at_end(req) {
                    return false;
                }
            }

            req.start = ctx.cursor.position;
            state.start = req.start;
            state.cursor = ctx.cursor;
            S::skip(&mut state.cursor, skip);

            // literal only
            if LITERAL {
                return true;
            }

            let mut next_ctx = ctx;
            next_ctx.skip_char::<S>(skip);

            if _match(req, state, next_ctx) {
                return true;
            }

            ctx.advance_char::<S>();
            state.marks.clear();
        }
    } else {
        while !ctx.at_end(req) {
            let c = prefix[0];
            while ctx.peek_char::<S>() != c {
                ctx.advance_char::<S>();
                if ctx.at_end(req) {
                    return false;
                }
            }
            ctx.advance_char::<S>();
            if ctx.at_end(req) {
                return false;
            }

            let mut i = 1;
            loop {
                if ctx.peek_char::<S>() == prefix[i] {
                    i += 1;
                    if i != len {
                        ctx.advance_char::<S>();
                        if ctx.at_end(req) {
                            return false;
                        }
                        continue;
                    }

                    req.start = ctx.cursor.position - (len - 1);
                    state.reset(req, req.start);
                    S::skip(&mut state.cursor, skip);
                    // state.start = req.start;
                    // state.cursor = req.string.create_cursor(req.start + skip);

                    // literal only
                    if LITERAL {
                        return true;
                    }

                    let mut next_ctx = ctx;
                    if skip != 0 {
                        next_ctx.advance_char::<S>();
                    } else {
                        next_ctx.cursor = state.cursor;
                    }

                    if _match(req, state, next_ctx) {
                        return true;
                    }

                    ctx.advance_char::<S>();
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
    req: &mut Request<'_, S>,
    state: &mut State,
    mut ctx: MatchContext,
) -> bool {
    let set = &ctx.pattern(req)[5..];

    ctx.skip_code_from(req, 1);

    req.must_advance = false;

    loop {
        while !ctx.at_end(req) && !charset(set, ctx.peek_char::<S>()) {
            ctx.advance_char::<S>();
        }
        if ctx.at_end(req) {
            return false;
        }

        req.start = ctx.cursor.position;
        state.start = ctx.cursor.position;
        state.cursor = ctx.cursor;

        if _match(req, state, ctx) {
            return true;
        }

        ctx.advance_char::<S>();
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

#[derive(Clone, Copy)]
struct MatchContext {
    cursor: StringCursor,
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

    const fn remaining_codes<S>(&self, req: &Request<'_, S>) -> usize {
        req.pattern_codes.len() - self.code_position
    }

    const fn remaining_chars<S>(&self, req: &Request<'_, S>) -> usize {
        req.end - self.cursor.position
    }

    fn peek_char<S: StrDrive>(&self) -> u32 {
        S::peek(&self.cursor)
    }

    fn skip_char<S: StrDrive>(&mut self, skip: usize) {
        S::skip(&mut self.cursor, skip);
    }

    fn advance_char<S: StrDrive>(&mut self) -> u32 {
        S::advance(&mut self.cursor)
    }

    fn back_peek_char<S: StrDrive>(&self) -> u32 {
        S::back_peek(&self.cursor)
    }

    fn back_skip_char<S: StrDrive>(&mut self, skip: usize) {
        S::back_skip(&mut self.cursor, skip);
    }

    fn back_advance_char<S: StrDrive>(&mut self) -> u32 {
        S::back_advance(&mut self.cursor)
    }

    fn peek_code<S>(&self, req: &Request<'_, S>, peek: usize) -> u32 {
        req.pattern_codes[self.code_position + peek]
    }

    fn try_peek_code_as<T, S>(&self, req: &Request<'_, S>, peek: usize) -> Result<T, T::Error>
    where
        T: TryFrom<u32>,
    {
        self.peek_code(req, peek).try_into()
    }

    const fn skip_code(&mut self, skip: usize) {
        self.code_position += skip;
    }

    fn skip_code_from<S>(&mut self, req: &Request<'_, S>, peek: usize) {
        self.skip_code(self.peek_code(req, peek) as usize + 1);
    }

    const fn at_beginning(&self) -> bool {
        // self.ctx().string_position == self.state().start
        self.cursor.position == 0
    }

    const fn at_end<S>(&self, req: &Request<'_, S>) -> bool {
        self.cursor.position == req.end
    }

    fn at_linebreak<S: StrDrive>(&self, req: &Request<'_, S>) -> bool {
        !self.at_end(req) && is_linebreak(self.peek_char::<S>())
    }

    fn at_boundary<S: StrDrive, F: FnMut(u32) -> bool>(
        &self,
        req: &Request<'_, S>,
        mut word_checker: F,
    ) -> bool {
        if self.at_beginning() && self.at_end(req) {
            return false;
        }
        let that = !self.at_beginning() && word_checker(self.back_peek_char::<S>());
        let this = !self.at_end(req) && word_checker(self.peek_char::<S>());
        this != that
    }

    fn at_non_boundary<S: StrDrive, F: FnMut(u32) -> bool>(
        &self,
        req: &Request<'_, S>,
        mut word_checker: F,
    ) -> bool {
        if self.at_beginning() && self.at_end(req) {
            return false;
        }
        let that = !self.at_beginning() && word_checker(self.back_peek_char::<S>());
        let this = !self.at_end(req) && word_checker(self.peek_char::<S>());
        this == that
    }

    const fn can_success<S>(&self, req: &Request<'_, S>) -> bool {
        if !self.toplevel {
            return true;
        }
        if req.match_all && !self.at_end(req) {
            return false;
        }
        if req.must_advance && self.cursor.position == req.start {
            return false;
        }
        true
    }

    #[must_use]
    fn next_peek_from<S>(&mut self, peek: usize, req: &Request<'_, S>, jump: Jump) -> Self {
        self.next_offset(self.peek_code(req, peek) as usize + 1, jump)
    }

    #[must_use]
    const fn next_offset(&mut self, offset: usize, jump: Jump) -> Self {
        self.next_at(self.code_position + offset, jump)
    }

    #[must_use]
    const fn next_at(&mut self, code_position: usize, jump: Jump) -> Self {
        self.jump = jump;
        Self {
            code_position,
            jump: Jump::OpCode,
            count: -1,
            ..*self
        }
    }
}

fn at<S: StrDrive>(req: &Request<'_, S>, ctx: &MatchContext, at_code: SreAtCode) -> bool {
    match at_code {
        SreAtCode::BEGINNING | SreAtCode::BEGINNING_STRING => ctx.at_beginning(),
        SreAtCode::BEGINNING_LINE => ctx.at_beginning() || is_linebreak(ctx.back_peek_char::<S>()),
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

fn category(cat_code: SreCatCode, c: u32) -> bool {
    match cat_code {
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
                let cat_code = match SreCatCode::try_from(set[i + 1]) {
                    Ok(code) => code,
                    Err(_) => {
                        break;
                    }
                };
                if category(cat_code, ch) {
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
                /* <BIGCHARSET> <block_count> <256 block_indices> <blocks> */
                let count = set[i + 1] as usize;
                if ch < 0x10000 {
                    let set = &set[i + 2..];
                    let block_index = ch >> 8;
                    let (_, block_indices, _) = unsafe { set.align_to::<u8>() };
                    let blocks = &set[64..];
                    let block = block_indices[block_index as usize];
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
    req: &Request<'_, S>,
    state: &mut State,
    ctx: &mut MatchContext,
    max_count: usize,
) -> usize {
    let max_count = std::cmp::min(max_count, ctx.remaining_chars(req));
    let end = ctx.cursor.position + max_count;
    let opcode = SreOpcode::try_from(ctx.peek_code(req, 0)).unwrap();

    match opcode {
        SreOpcode::ANY => {
            while ctx.cursor.position < end && !ctx.at_linebreak(req) {
                ctx.advance_char::<S>();
            }
        }
        SreOpcode::ANY_ALL => {
            ctx.skip_char::<S>(max_count);
        }
        SreOpcode::IN => {
            while ctx.cursor.position < end && charset(&ctx.pattern(req)[2..], ctx.peek_char::<S>())
            {
                ctx.advance_char::<S>();
            }
        }
        SreOpcode::LITERAL => {
            general_count_literal(req, ctx, end, |code, c| code == c);
        }
        SreOpcode::NOT_LITERAL => {
            general_count_literal(req, ctx, end, |code, c| code != c);
        }
        SreOpcode::LITERAL_IGNORE => {
            general_count_literal(req, ctx, end, |code, c| code == lower_ascii(c));
        }
        SreOpcode::NOT_LITERAL_IGNORE => {
            general_count_literal(req, ctx, end, |code, c| code != lower_ascii(c));
        }
        SreOpcode::LITERAL_LOC_IGNORE => {
            general_count_literal(req, ctx, end, char_loc_ignore);
        }
        SreOpcode::NOT_LITERAL_LOC_IGNORE => {
            general_count_literal(req, ctx, end, |code, c| !char_loc_ignore(code, c));
        }
        SreOpcode::LITERAL_UNI_IGNORE => {
            general_count_literal(req, ctx, end, |code, c| code == lower_unicode(c));
        }
        SreOpcode::NOT_LITERAL_UNI_IGNORE => {
            general_count_literal(req, ctx, end, |code, c| code != lower_unicode(c));
        }
        _ => {
            /* General case */
            ctx.toplevel = false;
            ctx.jump = Jump::OpCode;
            ctx.repeat_ctx_id = usize::MAX;
            ctx.count = -1;

            let mut sub_state = State {
                marks: Marks::default(),
                repeat_stack: vec![],
                ..*state
            };

            while ctx.cursor.position < end && _match(req, &mut sub_state, *ctx) {
                ctx.advance_char::<S>();
            }
        }
    }

    // TODO: return offset
    ctx.cursor.position - state.cursor.position
}

fn general_count_literal<S: StrDrive, F: FnMut(u32, u32) -> bool>(
    req: &Request<'_, S>,
    ctx: &mut MatchContext,
    end: usize,
    mut f: F,
) {
    let ch = ctx.peek_code(req, 1);
    while ctx.cursor.position < end && f(ch, ctx.peek_char::<S>()) {
        ctx.advance_char::<S>();
    }
}
