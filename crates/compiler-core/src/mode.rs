#[derive(Clone, Copy)]
pub enum Mode {
    Exec,
    Eval,
    Single,
    /// Returns the value of the last statement in the statement list.
    BlockExpr,
}

impl core::str::FromStr for Mode {
    type Err = ModeParseError;

    // To support `builtins.compile()` `mode` argument
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "exec" => Ok(Self::Exec),
            "eval" => Ok(Self::Eval),
            "single" => Ok(Self::Single),
            _ => Err(ModeParseError),
        }
    }
}

/// Returned when a given mode is not valid.
#[derive(Debug)]
pub struct ModeParseError;

impl core::fmt::Display for ModeParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, r#"mode must be "exec", "eval", or "single""#)
    }
}
