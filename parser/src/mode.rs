#[derive(Clone, Copy)]
pub enum Mode {
    Program,
    Statement,
}

impl std::str::FromStr for Mode {
    type Err = ModeParseError;
    fn from_str(s: &str) -> Result<Self, ModeParseError> {
        match s {
            "exec" | "single" => Ok(Mode::Program),
            "eval" => Ok(Mode::Statement),
            _ => Err(ModeParseError { _priv: () }),
        }
    }
}

#[derive(Debug)]
pub struct ModeParseError {
    _priv: (),
}

impl std::fmt::Display for ModeParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, r#"mode should be "exec", "eval", or "single""#)
    }
}
