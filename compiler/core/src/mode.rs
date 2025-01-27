pub use ruff_python_parser::ModeParseError;

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

impl From<Mode> for ruff_python_parser::Mode {
    fn from(mode: Mode) -> Self {
        match mode {
            Mode::Exec => Self::Module,
            Mode::Eval => Self::Expression,
            // TODO: Improve ruff API
            // ruff does not have an interactive mode
            Mode::Single | Mode::BlockExpr => Self::Ipython,
        }
    }
}
