pub use rustpython_parser_core::mode::ModeParseError;

#[derive(Clone, Copy)]
pub enum Mode {
    Exec,
    Eval,
    Single,
    BlockExpr,
}

impl std::str::FromStr for Mode {
    type Err = ModeParseError;

    // To support `builtins.compile()` `mode` argument
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "exec" => Ok(Mode::Exec),
            "eval" => Ok(Mode::Eval),
            "single" => Ok(Mode::Single),
            _ => Err(ModeParseError),
        }
    }
}

impl From<Mode> for rustpython_parser_core::Mode {
    fn from(mode: Mode) -> Self {
        match mode {
            Mode::Exec => Self::Module,
            Mode::Eval => Self::Expression,
            Mode::Single | Mode::BlockExpr => Self::Interactive,
        }
    }
}
