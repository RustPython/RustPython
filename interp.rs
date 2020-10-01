// good luck to those that follow; here be dragons

use crate::builtins::PyStrRef;

use super::constants::{SreFlag, SreOpcode};

use std::convert::TryFrom;
use std::{iter, slice};

pub struct State {
    start: usize,
    s_pos: usize,
    end: usize,
    pos: usize,
    flags: SreFlag,
    marks: Vec<usize>,
    lastindex: isize,
    marks_stack: Vec<usize>,
    context_stack: Vec<MatchContext>,
    repeat: Option<usize>,
    s: PyStrRef,
}

// struct State1<'a> {
//     state: &'a mut State,
// }

struct MatchContext {
    s_pos: usize,
    code_pos: usize,
}

// struct Context<'a> {
//     context_stack: &mut Vec<MatchContext>,
// }

impl State {
    pub fn new(s: PyStrRef, start: usize, end: usize, flags: SreFlag) -> Self {
        let end = std::cmp::min(end, s.char_len());
        Self {
            start,
            s_pos: start,
            end,
            pos: start,
            flags,
            marks: Vec::new(),
            lastindex: -1,
            marks_stack: Vec::new(),
            context_stack: Vec::new(),
            repeat: None,
            s,
        }
    }
}

// struct OpcodeDispatcher {
//     executing_contexts: HashMap<usize, Rc<State>>,
// }

pub struct BadSreCode;

pub fn parse_ops(code: &[u32]) -> impl Iterator<Item = Result<Op, BadSreCode>> + '_ {
    let mut it = code.iter().copied();
    std::iter::from_fn(move || -> Option<Option<Op>> {
        let op = it.next()?;
        let op = SreOpcode::try_from(op)
            .ok()
            .and_then(|op| extract_code(op, &mut it));
        Some(op)
    })
    .map(|x| x.ok_or(BadSreCode))
}

type It<'a> = iter::Copied<slice::Iter<'a, u32>>;
fn extract_code(op: SreOpcode, it: &mut It) -> Option<Op> {
    let skip = |it: &mut It| {
        let skip = it.next()? as usize;
        if skip > it.len() {
            None
        } else {
            Some(skip)
        }
    };
    match op {
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
        SreOpcode::GROUPREF_IGNORE => {}
        SreOpcode::IN => {}
        SreOpcode::IN_IGNORE => {}
        SreOpcode::INFO => {
            // let skip = it.next()?;
        }
        SreOpcode::JUMP => {}
        SreOpcode::LITERAL => {}
        SreOpcode::LITERAL_IGNORE => {}
        SreOpcode::MARK => {}
        SreOpcode::MAX_UNTIL => {}
        SreOpcode::MIN_UNTIL => {}
        SreOpcode::NOT_LITERAL => {}
        SreOpcode::NOT_LITERAL_IGNORE => {}
        SreOpcode::NEGATE => {}
        SreOpcode::RANGE => {}
        SreOpcode::REPEAT => {}
        SreOpcode::REPEAT_ONE => {}
        SreOpcode::SUBPATTERN => {}
        SreOpcode::MIN_REPEAT_ONE => {}
        SreOpcode::RANGE_IGNORE => {}
    }
    todo!()
}

pub enum Op {
    Info {},
}
