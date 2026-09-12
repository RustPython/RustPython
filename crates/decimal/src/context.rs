//! Arithmetic context: the precision, exponent range and rounding that shape
//! every result.
//!
//! Flags and traps live in the binding layer; an engine operation only reports
//! the conditions it raised through a status word, exactly as libmpdec does.

use crate::{MAX_EMAX, MAX_PREC, MIN_EMIN};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RoundMode {
    Down,
    Up,
    HalfUp,
    HalfDown,
    HalfEven,
    Ceiling,
    Floor,
    ZeroFiveUp,
}

impl RoundMode {
    pub const ALL: [Self; 8] = [
        Self::Down,
        Self::HalfUp,
        Self::HalfEven,
        Self::Ceiling,
        Self::Floor,
        Self::Up,
        Self::HalfDown,
        Self::ZeroFiveUp,
    ];

    /// The Python-level name, e.g. `ROUND_HALF_EVEN`.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Down => "ROUND_DOWN",
            Self::Up => "ROUND_UP",
            Self::HalfUp => "ROUND_HALF_UP",
            Self::HalfDown => "ROUND_HALF_DOWN",
            Self::HalfEven => "ROUND_HALF_EVEN",
            Self::Ceiling => "ROUND_CEILING",
            Self::Floor => "ROUND_FLOOR",
            Self::ZeroFiveUp => "ROUND_05UP",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.name() == name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Context {
    pub prec: i64,
    pub emin: i64,
    pub emax: i64,
    pub round: RoundMode,
    pub capitals: bool,
    pub clamp: bool,
}

impl Default for Context {
    fn default() -> Self {
        Self {
            prec: 28,
            emin: -999_999,
            emax: 999_999,
            round: RoundMode::HalfEven,
            capitals: true,
            clamp: false,
        }
    }
}

impl Context {
    /// The widest context the module can represent, used for the exact
    /// conversions that `Decimal(...)` performs without a context.
    pub const fn max() -> Self {
        Self {
            prec: MAX_PREC,
            emin: MIN_EMIN,
            emax: MAX_EMAX,
            round: RoundMode::HalfEven,
            capitals: true,
            clamp: false,
        }
    }

    /// Smallest exponent a subnormal result may have.
    #[inline]
    pub const fn etiny(&self) -> i64 {
        self.emin - (self.prec - 1)
    }

    /// Largest exponent a result with a full-precision coefficient may have.
    #[inline]
    pub const fn etop(&self) -> i64 {
        self.emax - (self.prec - 1)
    }
}
