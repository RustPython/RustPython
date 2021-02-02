use sre_engine::constants::SreFlag;
use sre_engine::engine;

struct Pattern {
    code: &'static [u32],
    flags: SreFlag,
}

impl Pattern {
    fn state<'a>(
        &self,
        string: impl Into<engine::StrDrive<'a>>,
        range: std::ops::Range<usize>,
    ) -> engine::State<'a> {
        engine::State::new(string.into(), range.start, range.end, self.flags, self.code)
    }
}

#[test]
fn test_2427() {
    // r'(?<!\.)x\b'
    let pattern = include!("lookbehind.re");
    let mut state = pattern.state("x", 0..usize::MAX);
    state = state.pymatch();
    assert!(state.has_matched == Some(true));
}
