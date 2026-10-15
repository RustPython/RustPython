//! Cumulative entity accounting, independent of the fixed depth and queue limits.

/// Immutable limits for cumulative entity expansion. The factor uses the same
/// single-precision ratio as Expat: `(direct + indirect) / direct`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AmplificationLimits {
    maximum_factor_bits: u32,
    activation_threshold: u64,
}

impl AmplificationLimits {
    /// Rejects NaN and factors below one. Positive infinity is permitted.
    #[must_use]
    pub fn new(maximum_factor: f32, activation_threshold: u64) -> Option<Self> {
        if maximum_factor.is_nan() || maximum_factor < 1.0 {
            return None;
        }
        Some(Self {
            maximum_factor_bits: maximum_factor.to_bits(),
            activation_threshold,
        })
    }
}

#[derive(Clone)]
pub(crate) struct Accounting {
    limits: Option<AmplificationLimits>,
    pub(crate) direct: u64,
    indirect: u64,
    overflow: bool,
}

impl Accounting {
    pub(crate) fn new(limits: Option<AmplificationLimits>) -> Self {
        Self {
            limits,
            direct: 0,
            indirect: 0,
            overflow: false,
        }
    }

    pub(crate) fn add_direct(&mut self, bytes: usize) {
        self.direct = self.direct.checked_add(bytes as u64).unwrap_or_else(|| {
            self.overflow = true;
            u64::MAX
        });
    }

    pub(crate) fn add_indirect(&mut self, bytes: usize) {
        self.indirect = self.indirect.checked_add(bytes as u64).unwrap_or_else(|| {
            self.overflow = true;
            u64::MAX
        });
    }

    pub(crate) fn tolerated(&self) -> bool {
        self.tolerated_at_direct(self.direct)
    }

    pub(crate) fn tolerated_at_direct(&self, direct: u64) -> bool {
        let Some(limits) = self.limits else {
            return true;
        };
        let Some(total) = direct.checked_add(self.indirect) else {
            return false;
        };
        if self.overflow {
            return false;
        }
        if total < limits.activation_threshold {
            return true;
        }
        let factor = if direct != 0 {
            total as f32 / direct as f32
        } else {
            // Expat uses the shortest possible external entity declaration.
            (22.0 + self.indirect as f32) / 22.0
        };
        factor <= f32::from_bits(limits.maximum_factor_bits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_factor_and_overflow() {
        assert!(AmplificationLimits::new(f32::NAN, 0).is_none());
        assert!(AmplificationLimits::new(0.99, 0).is_none());
        let mut a = Accounting::new(AmplificationLimits::new(1.0, 10));
        a.add_direct(5);
        a.add_indirect(4);
        assert!(a.tolerated());
        a.add_indirect(1);
        assert!(!a.tolerated());
        a.limits = AmplificationLimits::new(f32::INFINITY, 0);
        assert!(a.tolerated());
        a.direct = u64::MAX;
        assert!(!a.tolerated());
        a.indirect = 0;
        a.add_direct(1);
        assert!(!a.tolerated());
    }
}
