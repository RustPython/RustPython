//! Extended grapheme clusters over Python's full code point domain.
//!
//! State transitions and rule ordering follow CPython's Modules/unicodedata.c
//! at d220409a13b77de7a6c30abe16ea90db01528edc (Unicode 17.0.0).

// spell-checker:ignore incb

use icu_properties::{
    CodePointMapData, CodePointSetData,
    props::{ExtendedPictographic, GraphemeClusterBreak as Gcb, IndicConjunctBreak as Incb},
};
use rustpython_wtf8::CodePoint;

fn gcb(c: CodePoint) -> Gcb {
    CodePointMapData::<Gcb>::new().get32(c.to_u32())
}

fn incb(c: CodePoint) -> Incb {
    CodePointMapData::<Incb>::new().get32(c.to_u32())
}

/// The long Grapheme_Cluster_Break property name.
#[must_use]
pub fn grapheme_cluster_break(c: CodePoint) -> &'static str {
    match gcb(c) {
        Gcb::CR => "CR",
        Gcb::LF => "LF",
        Gcb::Control => "Control",
        Gcb::Extend => "Extend",
        Gcb::L => "L",
        Gcb::V => "V",
        Gcb::T => "T",
        Gcb::LV => "LV",
        Gcb::LVT => "LVT",
        Gcb::SpacingMark => "SpacingMark",
        Gcb::Prepend => "Prepend",
        Gcb::RegionalIndicator => "Regional_Indicator",
        Gcb::ZWJ => "ZWJ",
        _ => "Other",
    }
}

/// The long Indic_Conjunct_Break property name.
#[must_use]
pub fn indic_conjunct_break(c: CodePoint) -> &'static str {
    match incb(c) {
        Incb::Consonant => "Consonant",
        Incb::Extend => "Extend",
        Incb::Linker => "Linker",
        _ => "None",
    }
}

/// Whether the code point has the Extended_Pictographic property.
#[must_use]
pub fn extended_pictographic(c: CodePoint) -> bool {
    CodePointSetData::new::<ExtendedPictographic>().contains32(c.to_u32())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExtPictState {
    Init,
    Started,
    Zwj,
    Matched,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IncbState {
    Init,
    Started,
    Linker,
    Matched,
}

/// Streaming break state, initialized afresh at the requested slice start.
///
/// A code point is consumed before its preceding boundary is returned. Keep
/// this state across cluster yields, rather than processing that point twice.
#[derive(Debug)]
pub struct GraphemeBreakState {
    previous: Gcb,
    pictographic: ExtPictState,
    conjunct: IncbState,
    regional_odd: bool,
    started: bool,
}

impl Default for GraphemeBreakState {
    fn default() -> Self {
        Self {
            previous: Gcb::Other,
            pictographic: ExtPictState::Init,
            conjunct: IncbState::Init,
            regional_odd: false,
            started: false,
        }
    }
}

impl GraphemeBreakState {
    /// Consume one code point and report whether a new cluster starts here.
    pub fn advance(&mut self, c: CodePoint) -> bool {
        let current = gcb(c);
        self.pictographic = if extended_pictographic(c) {
            if self.pictographic == ExtPictState::Zwj {
                ExtPictState::Matched
            } else {
                ExtPictState::Started
            }
        } else if matches!(
            self.pictographic,
            ExtPictState::Started | ExtPictState::Matched
        ) {
            match current {
                Gcb::Extend => ExtPictState::Started,
                Gcb::ZWJ => ExtPictState::Zwj,
                _ => ExtPictState::Init,
            }
        } else {
            ExtPictState::Init
        };
        self.conjunct = match incb(c) {
            Incb::Consonant => {
                if self.conjunct == IncbState::Linker {
                    IncbState::Matched
                } else {
                    IncbState::Started
                }
            }
            Incb::Extend if self.conjunct != IncbState::Init => {
                if self.conjunct == IncbState::Linker {
                    IncbState::Linker
                } else {
                    IncbState::Started
                }
            }
            Incb::Linker if self.conjunct != IncbState::Init => IncbState::Linker,
            _ => IncbState::Init,
        };
        self.regional_odd = current == Gcb::RegionalIndicator && !self.regional_odd;
        let boundary = self.started && self.is_break(current);
        self.previous = current;
        self.started = true;
        boundary
    }

    fn is_break(&self, current: Gcb) -> bool {
        let previous = self.previous;
        // GB3, GB4, GB5: controls take precedence over attachment rules.
        if previous == Gcb::CR && current == Gcb::LF {
            return false;
        }
        if matches!(previous, Gcb::CR | Gcb::LF | Gcb::Control)
            || matches!(current, Gcb::CR | Gcb::LF | Gcb::Control)
        {
            return true;
        }
        // GB6, GB7, GB8: Hangul syllables.
        if previous == Gcb::L && matches!(current, Gcb::L | Gcb::V | Gcb::LV | Gcb::LVT)
            || matches!(previous, Gcb::LV | Gcb::V) && matches!(current, Gcb::V | Gcb::T)
            || matches!(previous, Gcb::LVT | Gcb::T) && current == Gcb::T
        {
            return false;
        }
        // GB9, GB9a, GB9b, GB9c, GB11.
        if matches!(current, Gcb::Extend | Gcb::ZWJ | Gcb::SpacingMark)
            || previous == Gcb::Prepend
            || self.conjunct == IncbState::Matched
            || self.pictographic == ExtPictState::Matched
        {
            return false;
        }
        // GB12, GB13: parity includes the current regional indicator.
        if previous == Gcb::RegionalIndicator && current == previous {
            return self.regional_odd;
        }
        true // GB999
    }
}
