use rustpython_parser::parser;

#[derive(Clone, Copy)]
pub enum Mode {
    Exec,
    Eval,
    Single,
}

impl std::str::FromStr for Mode {
    type Err = ModeParseError;
    fn from_str(s: &str) -> Result<Self, ModeParseError> {
        match s {
            "exec" => Ok(Mode::Exec),
            "eval" => Ok(Mode::Eval),
            "single" => Ok(Mode::Single),
            _ => Err(ModeParseError { _priv: () }),
        }
    }
}

impl Mode {
    pub fn to_parser_mode(self) -> parser::Mode {
        match self {
            Mode::Exec | Mode::Single => parser::Mode::Program,
            Mode::Eval => parser::Mode::Statement,
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
