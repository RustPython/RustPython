use engine::{State, StrDrive};

use super::*;

#[test]
fn test_2427() {
    let str_drive = StrDrive::Str("x");
    // r'(?<!\.)x\b'
    let code: Vec<u32> = vec![15, 4, 0, 1, 1, 5, 5, 1, 17, 46, 1, 17, 120, 6, 10, 1];
    let mut state = State::new(
        str_drive,
        0,
        std::usize::MAX,
        constants::SreFlag::UNICODE,
        &code,
    );
    state = state.pymatch();
    assert!(state.has_matched == Some(true));
}
