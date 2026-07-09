//! Differential sweep of the classification predicates over the full scalar
//! range `0..0x110000` against a committed CPython reference dataset.
//!
//! CPython 3.14 ships Unicode 16.0.0 while the Rust standard library / icu4x
//! build used here may be a later release. Code points whose classification
//! changed between those Unicode versions are expected to differ; those are
//! recorded in `data/version_skew_cpython3.14.txt` as an explicit allow-list.
//! Any divergence outside that list fails the test — a real regression, not a
//! version bump.
//!
//! Both data files use the same run-length format: one `predicate` line per
//! str method, followed by comma-separated hex `start:end` inclusive ranges.

// spell-checker:ignore recategorized recategorizations

#[cfg(test)]
mod tests {
    extern crate alloc;

    use alloc::collections::{BTreeMap, BTreeSet};

    use rustpython_unicode::{case, classify};

    const MAX: u32 = 0x110000;
    const REFERENCE: &str = include_str!("data/cpython3.14_predicates.txt");
    const VERSION_SKEW: &str = include_str!("data/version_skew_cpython3.14.txt");
    const MAPPINGS: &str = include_str!("data/cpython3.14_mappings.txt");
    const MAPPING_SKEW: &str = include_str!("data/version_skew_mappings_cpython3.14.txt");

    fn crate_predicate(name: &str, cp: u32) -> bool {
        let Some(c) = char::from_u32(cp) else {
            // Lone surrogates are not scalars; every str predicate is false.
            return false;
        };
        match name {
            "isalpha" => classify::is_alpha(c),
            "isalnum" => classify::is_alnum(c),
            "isdecimal" => classify::is_decimal(c),
            "isdigit" => classify::is_digit(c),
            "isnumeric" => classify::is_numeric(c),
            "isspace" => classify::is_space(c),
            "isprintable" => classify::is_printable(c),
            "isidentifier" => {
                // str.isidentifier is a whole-string predicate; for a single char it
                // is "may start an identifier".
                classify_is_identifier_char(c)
            }
            "is_lowercase" => case::is_lowercase(c),
            "is_uppercase" => case::is_uppercase(c),
            "is_titlecase" => case::is_titlecase(c),
            "is_cased" => case::is_cased(c),
            other => panic!("unknown predicate {other}"),
        }
    }

    /// The crate's simple case mapping for `name` at `cp`, as a code point.
    fn crate_mapping(name: &str, cp: u32) -> u32 {
        let Some(c) = char::from_u32(cp) else {
            return cp;
        };
        match name {
            "tolower" => case::simple_lowercase(c) as u32,
            other => panic!("unknown mapping {other}"),
        }
    }

    fn classify_is_identifier_char(c: char) -> bool {
        rustpython_unicode::identifier::is_start(c)
    }

    /// Parse a `name -> sorted set of code points` map from a run-length file.
    ///
    /// Each non-comment line is `predicate start:end,start:end,...` with inclusive
    /// hex ranges; a predicate with no members is a bare `predicate`.
    fn parse_ranges(text: &str) -> BTreeMap<String, BTreeSet<u32>> {
        let mut map = BTreeMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (name, packed) = match line.split_once(' ') {
                Some((name, packed)) => (name, packed.trim()),
                None => (line, ""),
            };
            let mut set = BTreeSet::new();
            if !packed.is_empty() {
                for run in packed.split(',') {
                    let (s, e) = run.split_once(':').expect("run is start:end");
                    let start = u32::from_str_radix(s, 16).unwrap();
                    let end = u32::from_str_radix(e, 16).unwrap();
                    for cp in start..=end {
                        set.insert(cp);
                    }
                }
            }
            map.insert(name.to_string(), set);
        }
        map
    }

    /// Parse a `name -> {code point -> mapped code point}` table.
    ///
    /// Each non-comment line is `name cp:mapped,cp:mapped,...` listing only the
    /// code points whose mapping differs from identity.
    fn parse_mappings(text: &str) -> BTreeMap<String, BTreeMap<u32, u32>> {
        let mut map = BTreeMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (name, packed) = match line.split_once(' ') {
                Some((name, packed)) => (name, packed.trim()),
                None => (line, ""),
            };
            let mut table = BTreeMap::new();
            if !packed.is_empty() {
                for pair in packed.split(',') {
                    let (cp, mapped) = pair.split_once(':').expect("pair is cp:mapped");
                    table.insert(
                        u32::from_str_radix(cp, 16).unwrap(),
                        u32::from_str_radix(mapped, 16).unwrap(),
                    );
                }
            }
            map.insert(name.to_string(), table);
        }
        map
    }

    /// Collapse a sorted code-point set into inclusive `start:end` runs.
    fn encode_ranges(set: &BTreeSet<u32>) -> String {
        let mut runs = Vec::new();
        let mut iter = set.iter().copied();
        if let Some(first) = iter.next() {
            let (mut start, mut end) = (first, first);
            for cp in iter {
                if cp == end + 1 {
                    end = cp;
                } else {
                    runs.push((start, end));
                    start = cp;
                    end = cp;
                }
            }
            runs.push((start, end));
        }
        runs.iter()
            .map(|(s, e)| format!("{s:X}:{e:X}"))
            .collect::<Vec<_>>()
            .join(",")
    }

    /// Recompute the full divergence set. Every entry is a `(predicate, code
    /// point)` where the crate and the CPython reference disagree.
    fn all_divergences(reference: &BTreeMap<String, BTreeSet<u32>>) -> Vec<(String, u32, bool)> {
        let mut out = Vec::new();
        for (name, truth) in reference {
            for cp in 0..MAX {
                let expected = truth.contains(&cp);
                let actual = crate_predicate(name, cp);
                if expected != actual {
                    out.push((name.clone(), cp, expected));
                }
            }
        }
        out
    }

    /// Documented `cpython=true/crate=false` divergences: a code point that had a
    /// property in Unicode 16.0.0 but lost it in the release icu4x ships, because
    /// the code point was recategorized (not a regression in this crate).
    ///
    /// * U+0295 LATIN LETTER PHARYNGEAL VOICED FRICATIVE was general category `Ll`
    ///   in Unicode 16.0.0 and `Lo` from 17.0.0, so it is no longer `Lowercase`.
    const KNOWN_RECATEGORIZATIONS: &[(&str, u32)] = &[("is_lowercase", 0x0295)];

    /// Regenerate `data/version_skew_cpython3.14.txt` from the current toolchain.
    ///
    /// Run with `RUSTPYTHON_UNICODE_REGEN_SKEW=1 cargo test -p rustpython-unicode
    /// --test differential` after bumping the Rust/icu toolchain. Divergences are
    /// normally one-directional (crate=true, cpython=false) — newly-assigned code
    /// points from a later Unicode release. A `cpython=true, crate=false` entry
    /// means a code point lost a property; that is a real regression unless it is
    /// an explicit entry in `KNOWN_RECATEGORIZATIONS`, so this refuses to record
    /// any other reverse-direction divergence.
    #[test]
    fn regen_version_skew() {
        if std::env::var_os("RUSTPYTHON_UNICODE_REGEN_SKEW").is_none() {
            return;
        }
        let reference = parse_ranges(REFERENCE);
        let divergences = all_divergences(&reference);

        let regressions: Vec<_> = divergences
            .iter()
            .filter(|(name, cp, expected)| {
                *expected && !KNOWN_RECATEGORIZATIONS.contains(&(name.as_str(), *cp))
            })
            .collect();
        assert!(
            regressions.is_empty(),
            "refusing to record {} cpython=true/crate=false divergence(s) — these are \
             regressions, not version skew: {:?}",
            regressions.len(),
            &regressions[..regressions.len().min(20)]
        );

        let mut by_predicate: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
        for (name, cp, _) in &divergences {
            by_predicate.entry(name.clone()).or_default().insert(*cp);
        }

        let mut body = String::from(
            "# Code points whose classification differs between CPython 3.14 (Unicode 16.0.0)\n\
             # and the Rust std / icu4x build used here (a later Unicode release assigns them).\n\
             # Regenerate with RUSTPYTHON_UNICODE_REGEN_SKEW=1.\n\
             # Format: `predicate start:end,...` with inclusive hex ranges.\n",
        );
        for (name, set) in &by_predicate {
            body.push_str(&format!("{name} {}\n", encode_ranges(set)));
        }
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/data/version_skew_cpython3.14.txt"
        );
        std::fs::write(path, body).unwrap();
        eprintln!(
            "wrote {} skew code points across {} predicates to {path}",
            divergences.len(),
            by_predicate.len()
        );
    }

    #[test]
    fn predicates_match_cpython_except_documented_version_skew() {
        let reference = parse_ranges(REFERENCE);
        let skew = parse_ranges(VERSION_SKEW);

        let allowed = |name: &str, cp: u32| skew.get(name).is_some_and(|set| set.contains(&cp));

        let mut unexpected: Vec<(String, u32, bool, bool)> = Vec::new();

        for (name, truth) in &reference {
            for cp in 0..MAX {
                let expected = truth.contains(&cp);
                let actual = crate_predicate(name, cp);
                if expected != actual && !allowed(name, cp) {
                    unexpected.push((name.clone(), cp, expected, actual));
                }
            }
        }

        // Also flag stale allow-list entries: code points that no longer diverge.
        let mut stale: Vec<(String, u32)> = Vec::new();
        for (name, set) in &skew {
            let Some(truth) = reference.get(name) else {
                continue;
            };
            for &cp in set {
                let expected = truth.contains(&cp);
                let actual = crate_predicate(name, cp);
                if expected == actual {
                    stale.push((name.clone(), cp));
                }
            }
        }

        if !unexpected.is_empty() || !stale.is_empty() {
            let mut msg = String::new();
            if !unexpected.is_empty() {
                msg.push_str(&format!(
                    "{} undocumented divergence(s) from CPython:\n",
                    unexpected.len()
                ));
                for (name, cp, expected, actual) in unexpected.iter().take(50) {
                    msg.push_str(&format!(
                        "  {name} U+{cp:04X}: cpython={expected} crate={actual}\n"
                    ));
                }
            }
            if !stale.is_empty() {
                msg.push_str(&format!(
                    "{} stale version_skew_cpython3.14.txt entries that now agree:\n",
                    stale.len()
                ));
                for (name, cp) in stale.iter().take(50) {
                    msg.push_str(&format!("  {name} U+{cp:04X}\n"));
                }
            }
            panic!("{msg}");
        }
    }

    /// All `(mapping, code point)` where the crate and CPython map differently.
    fn all_mapping_divergences(
        reference: &BTreeMap<String, BTreeMap<u32, u32>>,
    ) -> Vec<(String, u32)> {
        let mut out = Vec::new();
        for (name, table) in reference {
            for cp in 0..MAX {
                let expected = table.get(&cp).copied().unwrap_or(cp);
                if crate_mapping(name, cp) != expected {
                    out.push((name.clone(), cp));
                }
            }
        }
        out
    }

    /// Regenerate `data/version_skew_mappings_cpython3.14.txt` from the current
    /// toolchain (`RUSTPYTHON_UNICODE_REGEN_SKEW=1`).
    ///
    /// Divergences are normally the crate gaining a mapping a later Unicode
    /// release assigns. A code point that CPython maps but the crate leaves
    /// unmapped is a regression, not version skew, so this refuses to record it.
    #[test]
    fn regen_mapping_version_skew() {
        if std::env::var_os("RUSTPYTHON_UNICODE_REGEN_SKEW").is_none() {
            return;
        }
        let reference = parse_mappings(MAPPINGS);
        let divergences = all_mapping_divergences(&reference);

        let regressions: Vec<_> = divergences
            .iter()
            .filter(|(name, cp)| {
                let expected = reference.get(name).and_then(|t| t.get(cp)).copied();
                expected.is_some_and(|e| e != *cp) && crate_mapping(name, *cp) == *cp
            })
            .collect();
        assert!(
            regressions.is_empty(),
            "refusing to record {} cpython-maps/crate-unmapped divergence(s) — these are \
             regressions, not version skew: {:?}",
            regressions.len(),
            &regressions[..regressions.len().min(20)]
        );

        let mut by_mapping: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
        for (name, cp) in &divergences {
            by_mapping.entry(name.clone()).or_default().insert(*cp);
        }

        let mut body = String::from(
            "# Code points whose simple case mapping differs between CPython 3.14\n\
             # (Unicode 16.0.0) and the icu4x build used here (a later Unicode release\n\
             # assigns the pair). Regenerate with RUSTPYTHON_UNICODE_REGEN_SKEW=1.\n\
             # Format: `mapping start:end,...` with inclusive hex ranges.\n",
        );
        for (name, set) in &by_mapping {
            body.push_str(&format!("{name} {}\n", encode_ranges(set)));
        }
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/data/version_skew_mappings_cpython3.14.txt"
        );
        std::fs::write(path, body).unwrap();
        eprintln!(
            "wrote {} skew code points across {} mappings to {path}",
            divergences.len(),
            by_mapping.len()
        );
    }

    #[test]
    fn simple_mappings_match_cpython_except_documented_version_skew() {
        let reference = parse_mappings(MAPPINGS);
        let skew = parse_ranges(MAPPING_SKEW);

        let allowed = |name: &str, cp: u32| skew.get(name).is_some_and(|set| set.contains(&cp));

        let mut unexpected: Vec<(String, u32, u32, u32)> = Vec::new();
        for (name, table) in &reference {
            for cp in 0..MAX {
                let expected = table.get(&cp).copied().unwrap_or(cp);
                let actual = crate_mapping(name, cp);
                if expected != actual && !allowed(name, cp) {
                    unexpected.push((name.clone(), cp, expected, actual));
                }
            }
        }

        let mut stale: Vec<(String, u32)> = Vec::new();
        for (name, set) in &skew {
            for &cp in set {
                let expected = reference
                    .get(name)
                    .and_then(|t| t.get(&cp))
                    .copied()
                    .unwrap_or(cp);
                if crate_mapping(name, cp) == expected {
                    stale.push((name.clone(), cp));
                }
            }
        }

        if !unexpected.is_empty() || !stale.is_empty() {
            let mut msg = String::new();
            if !unexpected.is_empty() {
                msg.push_str(&format!(
                    "{} undocumented mapping divergence(s) from CPython:\n",
                    unexpected.len()
                ));
                for (name, cp, expected, actual) in unexpected.iter().take(50) {
                    msg.push_str(&format!(
                        "  {name} U+{cp:04X}: cpython=U+{expected:04X} crate=U+{actual:04X}\n"
                    ));
                }
            }
            if !stale.is_empty() {
                msg.push_str(&format!(
                    "{} stale version_skew_mappings_cpython3.14.txt entries that now agree:\n",
                    stale.len()
                ));
                for (name, cp) in stale.iter().take(50) {
                    msg.push_str(&format!("  {name} U+{cp:04X}\n"));
                }
            }
            panic!("{msg}");
        }
    }
}
