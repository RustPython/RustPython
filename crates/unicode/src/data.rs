//! Access to the Unicode character database (`unicodedata`).
//!
//! Owns the generated Unicode 3.2.0 / latest tables and the
//! `icu4x`/`unicode_names2` lookups behind them.

// spell-checker:ignore codep decomp DECOMP unidata

use core::{cmp::Ordering, fmt::Write, hint::cold_path};

use icu_normalizer::properties::{CanonicalDecomposition, Decomposed};
use icu_properties::props::{
    BidiClass, BidiMirrored, BinaryProperty, CanonicalCombiningClass, EastAsianWidth,
    EnumeratedProperty, GeneralCategory, NamedEnumeratedProperty, NumericType,
};
use rustpython_wtf8::CodePoint;

include!(concat!(env!("OUT_DIR"), "/generated/unicode_3_2.rs"));
include!(concat!(env!("OUT_DIR"), "/generated/unicode_latest.rs"));
include!(concat!(env!("OUT_DIR"), "/generated/unicode_num_type.rs"));
include!(concat!(
    env!("OUT_DIR"),
    "/generated/unicode_numeric_value.rs"
));

#[derive(Clone, Copy)]
#[repr(u8)]
enum DecompositionType {
    #[allow(unused)]
    Canonical,
    Compat,
    Circle,
    Final,
    Font,
    Fraction,
    Initial,
    Isolated,
    Medial,
    Narrow,
    Nobreak,
    Small,
    Square,
    Sub,
    Super,
    Vertical,
    Wide,
}

impl DecompositionType {
    const fn type_tag(self) -> &'static str {
        match self {
            Self::Canonical => "canonical",
            Self::Compat => "compat",
            Self::Circle => "circle",
            Self::Final => "final",
            Self::Font => "font",
            Self::Fraction => "fraction",
            Self::Initial => "initial",
            Self::Isolated => "isolated",
            Self::Medial => "medial",
            Self::Narrow => "narrow",
            Self::Nobreak => "noBreak",
            Self::Small => "small",
            Self::Square => "square",
            Self::Sub => "sub",
            Self::Super => "super",
            Self::Vertical => "vertical",
            Self::Wide => "wide",
        }
    }
}

fn lookup_property<T: Copy>(table: &[(u32, u32, T)], ch: char) -> Option<T> {
    let ch = ch as u32;
    table
        .binary_search_by(|&(start, end, _)| {
            if ch > end {
                Ordering::Less
            } else if ch < start {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        })
        .ok()
        .map(|i| table[i].2)
}

fn lookup_numeric_val(ch: char, modern: bool) -> Option<f64> {
    if modern {
        lookup_property(NUMERIC_VALUES, ch)
    } else {
        cold_path();
        lookup_property(NUMERIC_VALUES_DIFF, ch).or_else(|| {
            NUMERIC_VAL_EXISTS_32
                .binary_search_by(|&(start, end)| {
                    let ch = ch as u32;
                    if ch > end {
                        Ordering::Less
                    } else if ch < start {
                        Ordering::Greater
                    } else {
                        Ordering::Equal
                    }
                })
                .ok()
                .and_then(|_| lookup_property(NUMERIC_VALUES, ch))
        })
    }
}

/// The version string of the latest Unicode database bundled with the standard
/// library (`unicodedata.unidata_version`).
#[must_use]
pub fn unicode_version() -> String {
    format!(
        "{}.{}.{}",
        char::UNICODE_VERSION.0,
        char::UNICODE_VERSION.1,
        char::UNICODE_VERSION.2
    )
}

/// Look up a character by its Unicode name (`unicodedata.lookup`).
#[must_use]
pub fn lookup_character(name: &str) -> Option<char> {
    unicode_names2::character(name)
}

/// The Unicode name of `ch` (`unicodedata.name`), if any.
#[must_use]
pub fn character_name(ch: char) -> Option<String> {
    unicode_names2::name(ch).map(|name| name.to_string())
}

/// A view over the Unicode character database at a fixed version.
///
/// `modern` selects the latest bundled UCD; otherwise the Unicode 3.2.0 tables
/// used by `unicodedata.ucd_3_2_0` are consulted.
#[derive(Debug, Clone, Copy)]
pub struct Ucd {
    modern: bool,
}

impl Ucd {
    #[must_use]
    pub const fn new(modern: bool) -> Self {
        Self { modern }
    }

    #[must_use]
    pub fn category(&self, c: CodePoint) -> &'static str {
        let Some(c) = c.to_char() else {
            return GeneralCategory::Surrogate.short_name();
        };
        if self.modern {
            Some(GeneralCategory::for_char(c))
        } else {
            cold_path();
            lookup_property(GENERAL_CATEGORY, c)
        }
        .unwrap_or(GeneralCategory::Unassigned)
        .short_name()
    }

    #[must_use]
    pub fn bidirectional(&self, c: CodePoint) -> &'static str {
        c.to_char()
            .and_then(|c| {
                if self.modern {
                    Some(BidiClass::for_char(c))
                } else {
                    cold_path();
                    lookup_property(BIDI_CLASS, c)
                }
            })
            .unwrap_or(BidiClass::LeftToRight)
            .short_name()
    }

    #[must_use]
    pub fn east_asian_width(&self, c: CodePoint) -> &'static str {
        c.to_char()
            .and_then(|c| {
                if self.modern {
                    Some(EastAsianWidth::for_char(c))
                } else {
                    cold_path();
                    // CPython overrides characters in the PUA for 3.2.0.
                    // Basic Multilingual Plane:
                    // https://en.wikipedia.org/wiki/Plane_(Unicode)#Basic_Multilingual_Plane
                    // https://en.wikipedia.org/wiki/Private_Use_Areas
                    // https://www.unicode.org/reports/tr11/tr11-10.html
                    // https://www.unicode.org/reports/tr11/
                    //
                    // Currently, this implementation is incomplete because I can't figure
                    // out what CPython is doing.
                    lookup_property(EAST_ASIAN_WIDTH, c)
                }
            })
            .unwrap_or(EastAsianWidth::Neutral)
            .short_name()
    }

    #[must_use]
    pub fn mirrored(&self, c: CodePoint) -> i32 {
        c.to_char().map_or(0, |c| {
            (if self.modern {
                BidiMirrored::for_char(c)
            } else {
                cold_path();
                let c = c as u32;
                BIDI_MIRRORED
                    .binary_search_by(|&(start, end)| {
                        if c > end {
                            Ordering::Less
                        } else if c < start {
                            Ordering::Greater
                        } else {
                            Ordering::Equal
                        }
                    })
                    .is_ok()
            }) as i32
        })
    }

    #[must_use]
    pub fn combining(&self, c: CodePoint) -> u8 {
        c.to_char()
            .and_then(|c| {
                if self.modern {
                    Some(CanonicalCombiningClass::for_char(c))
                } else {
                    cold_path();
                    lookup_property(COMBINING_CLASS, c)
                }
            })
            .unwrap_or(CanonicalCombiningClass::NotReordered)
            .to_icu4c_value()
    }

    #[must_use]
    pub fn decomposition(&self, c: CodePoint) -> String {
        let Some(ch) = c.to_char() else {
            return String::new();
        };

        // Decomposition is remarkable stable according to the normalization file,
        // so the updates slice is very small - only about four char pairs. Linearly searching
        // it is very fast. The file lists the original, incorrect decomp and the fixed char.
        // For 3.2.0, we use the original decomp for compatibility while ignoring the update.
        //
        // Finally, we don't have to do anything for the latest UCD as it's already updated.
        if self.modern
            && let Some((_, original)) = DECOMP_UPDATES
                .iter()
                .find(|&&(codep, _original)| codep == ch as u32)
        {
            format!("{original:04X}")
        } else if let Ok(i) =
            DECOMP_COMPAT.binary_search_by_key(&(ch as u32), |&(codep, _, _)| codep)
        {
            // Compatibility decomposition
            // `icu4x` doesn't expose a non-recursive, compatibility decomposer so we
            // have to do it manually for now.
            let tag = DECOMP_COMPAT[i].1.type_tag();
            let end = DECOMP_COMPAT[i].2;
            let start = i
                .checked_sub(1)
                .map(|i| DECOMP_COMPAT[i].2)
                .unwrap_or_default();

            let decomp = &DECOMP_RANGE[start..end];
            let cap = decomp.len() * 10 + decomp.len() + tag.len() + 1;
            let mut out = String::with_capacity(cap);

            write!(out, "<{tag}>").unwrap();
            for ch in decomp {
                write!(out, " {ch:04X}").unwrap();
            }

            out
        } else {
            // Canonical decomposition
            let decomposed = CanonicalDecomposition::new().decompose(ch);
            match decomposed {
                Decomposed::Default => String::new(),
                Decomposed::Singleton(ch) => format!("{:04X}", ch as u32),
                Decomposed::Expansion(l, r) => format!("{:04X} {:04X}", l as u32, r as u32),
            }
        }
    }

    fn numeric_type_matches(self, ch: CodePoint, expected: &[NumericType]) -> Option<char> {
        let ch = ch.to_char()?;

        let actual = if self.modern {
            NumericType::for_char(ch)
        } else {
            cold_path();
            lookup_property(NUMERIC_TYPE_DIFF, ch).unwrap_or_else(|| NumericType::for_char(ch))
        };

        expected.contains(&actual).then_some(ch)
    }

    /// The integer digit value of `c` (`unicodedata.digit`), if it has one.
    #[must_use]
    pub fn digit(&self, c: CodePoint) -> Option<u64> {
        let expected = [NumericType::Decimal, NumericType::Digit];
        self.numeric_type_matches(c, &expected).and_then(|ch| {
            let value = lookup_numeric_val(ch, true)?;
            (value.trunc() == value).then_some(value as u64)
        })
    }

    /// The integer decimal value of `c` (`unicodedata.decimal`), if it has one.
    #[must_use]
    pub fn decimal(&self, c: CodePoint) -> Option<u64> {
        let expected = [NumericType::Decimal];
        self.numeric_type_matches(c, &expected).and_then(|ch| {
            let value = lookup_numeric_val(ch, self.modern)?;
            (value.trunc() == value).then_some(value as u64)
        })
    }

    /// The numeric value of `c` (`unicodedata.numeric`), if it has one.
    #[must_use]
    pub fn numeric(&self, c: CodePoint) -> Option<f64> {
        let expected = &NumericType::ALL_VALUES[1..];
        self.numeric_type_matches(c, expected)
            .and_then(|ch| lookup_numeric_val(ch, self.modern))
    }

    #[must_use]
    pub fn unidata_version(&self) -> String {
        if self.modern {
            unicode_version()
        } else {
            "3.2.0".into()
        }
    }
}
