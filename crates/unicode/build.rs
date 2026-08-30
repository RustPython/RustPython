// spell-checker:ignore decomp DECOMP

extern crate alloc;

use core::{
    fmt::{Debug, Display},
    iter::Iterator,
    num::NonZeroUsize,
};

use alloc::collections::{BTreeMap, BTreeSet};

use std::{
    env,
    fs::{self, File},
    io::{BufRead, BufReader, BufWriter, Lines, Write},
    path::{Path, PathBuf},
    thread,
};

use icu_properties::props::{
    BidiClass, EnumeratedProperty, GeneralCategory, NamedEnumeratedProperty, NumericType,
};

/// Iterator over Unicode data file lines.
struct UnicodeLineReader {
    reader: Lines<BufReader<File>>,
}

impl UnicodeLineReader {
    fn new(reader: BufReader<File>) -> Self {
        let reader = reader.lines();
        Self { reader }
    }

    fn from_file_name(file_name: &str, modern: bool) -> Self {
        Self::new(open_reader(file_name, modern))
    }

    fn next_line_raw(&mut self) -> Option<Box<str>> {
        self.reader.find_map(|line| {
            let line = line.unwrap();
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                None
            } else {
                Some(line.to_owned().into_boxed_str())
            }
        })
    }
}

impl Iterator for UnicodeLineReader {
    type Item = UnicodeLine;

    fn next(&mut self) -> Option<Self::Item> {
        let line = self.next_line_raw()?;

        let mut fields = line.split(';');
        let range = fields.next().expect("Unicode data is missing a char range");
        let (start, end) = match range.split_once("..") {
            Some((left, right)) => {
                let start = u32::from_str_radix(left.trim(), 16).unwrap();
                let end = u32::from_str_radix(right.trim(), 16).unwrap();
                (start, end)
            }
            None => {
                let start = u32::from_str_radix(range.trim(), 16).unwrap();
                (start, start)
            }
        };

        Some(UnicodeLine { start, end, line })
    }
}

struct UnicodeLine {
    start: u32,
    end: u32,
    line: Box<str>,
}

impl UnicodeLine {
    /// Retrieve a field string from a line of Unicode data.
    fn field(&self, n: NonZeroUsize, msg: Option<&str>) -> &str {
        let field =
            self.line.split(';').nth(n.get()).unwrap_or_else(|| {
                panic!("{}", msg.unwrap_or("Unicode data is missing a property"))
            });
        // The field may have a comment so strip that out
        field.split_once('#').map_or(field, |(left, _)| left).trim()
    }
}

/// Helper to write a vector of condensed start, end char pairs and the associated ICU prop.
///
/// The values are impl Display because Debug would break the representation. For example, strs
/// would be wrapped in quotation marks which is wrong.
fn write_slice_display(
    writer: &mut impl Write,
    static_name: &str,
    array_type: &str,
    values: &mut [(u32, u32, impl Display)],
) {
    write_slice_pre(writer, static_name, array_type);
    values.sort_unstable_by_key(|(start, _, _)| *start);
    for (start, end, id) in values {
        write!(writer, "({start}, {end}, {id}),").unwrap();
    }
    write_slice_post(writer);
}

fn write_slice_debug(
    writer: &mut impl Write,
    static_name: &str,
    array_type: &str,
    values: &mut [(u32, u32, impl Debug)],
) {
    write_slice_pre(writer, static_name, array_type);
    values.sort_unstable_by_key(|(start, _, _)| *start);
    for (start, end, id) in values {
        write!(writer, "({start}, {end}, {id:?}),").unwrap();
    }
    write_slice_post(writer);
}

/// Helper to write a vector of start, end char pairs.
///
/// See: [`write_slice_display`].
fn write_slice_pairs(
    writer: &mut impl Write,
    static_name: &str,
    array_type: &str,
    values: &mut [(u32, u32)],
) {
    write_slice_pre(writer, static_name, array_type);
    values.sort_unstable_by_key(|(start, _)| *start);
    for (start, end) in values {
        write!(writer, "({start}, {end}),").unwrap();
    }
    write_slice_post(writer);
}

/// Helper to write a pre-sorted slice of strs.
///
/// See: [`write_slice_display`]
fn write_slice_unordered(
    writer: &mut impl Write,
    static_name: &str,
    array_type: &str,
    values: &mut [impl Display],
) {
    write_slice_pre(writer, static_name, array_type);
    for v in values {
        write!(writer, "{v},").unwrap();
    }
    write_slice_post(writer);
}

fn write_slice_pre(writer: &mut impl Write, static_name: &str, array_type: &str) {
    write!(writer, "static {static_name}: &[{array_type}] = &[").unwrap();
}

fn write_slice_post(writer: &mut impl Write) {
    writeln!(writer, "];").unwrap();
}

fn open_writer(file_name: &str) -> BufWriter<File> {
    let path = PathBuf::from(env::var("OUT_DIR").unwrap())
        .join("generated")
        .join(file_name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    BufWriter::new(File::create(&path).unwrap())
}

fn open_reader(file_name: &str, modern: bool) -> BufReader<File> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("unicode")
        .join(if modern { "latest" } else { "ucd32" })
        .join(file_name);

    BufReader::new(File::open(&path).unwrap_or_else(|e| {
        panic!(
            "{e}: vendored Unicode data file should exist: {}",
            path.display()
        )
    }))
}

/// Drive parsers that require the latest Unicode data.
///
/// The full data is huge so it's ideal to parse it only once.
fn full_data_parsers_latest() {
    let reader = UnicodeLineReader::from_file_name("UnicodeData.txt", true);

    let mut decomp_lines = Vec::new();
    for line in reader {
        let decomp_field = line.field(
            NonZeroUsize::new(5).unwrap(),
            Some(&format!(
                "field 5 missing from UnicodeData.txt: {}",
                line.line
            )),
        );
        if !decomp_field.is_empty() {
            decomp_lines.push((line.start, decomp_field.to_owned()));
        }
    }

    generate_decomp(decomp_lines);
}

fn generate_decomp(decomp_lines: Vec<(u32, String)>) {
    let mut writer = open_writer("decomp.rs");

    // NOTE:
    // This ONLY parses compatibility decomposition because Python exposes the tags. The tags are
    // the "<square>", "<circle>", et cetera bits before the decomposition. Thus, we can save space
    // by using icu4x's CanonicalDecomposer for non-compatibility decomposition.
    let mut decomp_ranges = Vec::new();
    let mut values = Vec::new();
    for (start, value) in decomp_lines {
        // We're building a sparse array. Most characters don't decompose, so we don't
        // need to literally store a row for each char.
        assert!(
            !value.is_empty(),
            "Decomp field shouldn't be empty at this point"
        );

        let Some((dtype, decomp)) = value.split_once('>').map(|(dtype, decomp)| {
            let dtype = dtype
                .strip_prefix('<')
                .unwrap_or_else(|| panic!("Compatibility decomp; expected <tag>\n\tgot: {value}"));
            (
                parse_decomp_type(dtype),
                decomp
                    .split_whitespace()
                    .map(|s| u32::from_str_radix(s, 16).unwrap()),
            )
        }) else {
            continue;
        };

        decomp_ranges.extend(decomp);
        let end = decomp_ranges.len();

        let value = format!("({start}, DecompositionType::{dtype:?}, {end})");
        values.push(value);
    }

    write_slice_unordered(
        &mut writer,
        "DECOMP_COMPAT",
        "(u32, DecompositionType, usize)",
        &mut values,
    );
    write_slice_unordered(&mut writer, "DECOMP_RANGE", "u32", &mut decomp_ranges);

    // Normalization corrections is super small - only a handful chars at the time of writing.
    let reader = UnicodeLineReader::from_file_name("NormalizationCorrections.txt", true);
    let mut values = Vec::new();
    for line in reader {
        let field = line.field(
            NonZeroUsize::new(1).unwrap(),
            Some(&format!(
                "field 2 missing from NormalizationCorrections: {}",
                line.line
            )),
        );
        let original = u32::from_str_radix(field.trim(), 16).unwrap_or_else(|e| {
            panic!("field 2 of decomp corrections should be a char in hex: {field} {e}")
        });
        let version = line.field(
            NonZeroUsize::new(3).unwrap(),
            Some(&format!(
                "field 4 of decomp corrections should be a UCD version: {}",
                line.line
            )),
        );

        // `version` = when the char was updated. Therefore, we use the incorrect chars past
        // 3.2.0 but skip the chars fixed in 3.2.0 because they'll already be right.
        if version != "3.2.0" {
            values.push((line.start, original))
        }
    }
    write_slice_pairs(&mut writer, "DECOMP_UPDATES", "(u32, u32)", &mut values);
}

/// Drive parsers that require the full 3.2.0 data.
///
/// As the full data set is HUGE, it's more efficient to parse it once then delegate to subparsers.
fn full_data_parsers_3_2() {
    let reader = UnicodeLineReader::from_file_name("UnicodeData-3.2.0.txt", false);

    // `DerivedNumericValues` writes the value rounded to a few digits, so the
    // fraction `UnicodeData` field 8 spells out is what the value comes from.
    // A character `UnicodeData` leaves out takes its value from Unihan, where
    // the rounded field is a whole number and loses nothing.
    let mut ucd32_fractions = BTreeMap::new();

    // Parse membership from the full data. Unfortunately, this is largely uncompressed.
    let mut membership_set = BTreeSet::new();
    let mut range_membership = Vec::new();

    for line in reader {
        // 3.2.0 membership
        let name = line.field(
            NonZeroUsize::new(1).unwrap(),
            Some(&format!(
                "field 1 (char name) missing from UnicodeData-3.2.0.txt: {}",
                line.line
            )),
        );
        if name.ends_with("First>") | name.ends_with("Last>") {
            // Some lines (literally 20) are compressed ranges, so we have to handle those separately
            range_membership.push(line.start);
        } else {
            membership_set.insert((line.start, line.end));
        }

        // Fractions
        let numeric = line.field(
            NonZeroUsize::new(8).unwrap(),
            Some(&format!(
                "field 8 (fraction) missing from UnicodeData-3.2.0.txt: {}",
                line.line
            )),
        );
        if !numeric.is_empty() {
            // start == end because the full data doesn't list code ranges.
            ucd32_fractions.insert(line.start, parse_numeric_value(numeric));
        }
    }

    // Now delegate to parsers that need the full data.
    generate_membership_3_2(membership_set, range_membership);
    generate_numeric_value(ucd32_fractions);
}

/// Generate a compressed array of Unicode 3.2 membership.
///
/// Membership + diff checks is more efficient than storing the full table for 3.2. The logic is to
/// default to the latest Unicode if a character exists in 3.2 but isn't different. Membership
/// is needed because diffs aren't enough - a character may be absent in 3.2 which is different
/// than returning a default.
fn generate_membership_3_2(membership_set: BTreeSet<(u32, u32)>, range_membership: Vec<u32>) {
    // Second pass. Compress the ranges.
    let mut iter = membership_set.iter();
    let &(mut start_prev, mut end_prev) = iter.next().unwrap();
    let mut membership = Vec::new();

    for &(start, end) in iter {
        if start <= end_prev + 1 {
            end_prev = end_prev.max(end);
        } else {
            membership.push((start_prev, end_prev));
            start_prev = start;
            end_prev = end;
        }
    }
    membership.push((start_prev, end_prev));

    let (chunks, &[]) = range_membership.as_chunks::<2>() else {
        panic!("Range membership is always in pairs ('First>' and 'Last>'");
    };
    for &chunk in chunks {
        membership.push(chunk.into());
    }

    let mut writer = open_writer("membership_3_2.rs");
    write_slice_pairs(&mut writer, "MEMBERSHIP_3_2", "(u32, u32)", &mut membership);
}

fn generate_numeric_value(ucd32_fractions: BTreeMap<u32, f64>) {
    let mut ucd32_diffs = BTreeMap::new();
    let numeric_32 = UnicodeLineReader::from_file_name("DerivedNumericValues-3.2.0.txt", false);
    for line in numeric_32 {
        let field = line.field(
            NonZeroUsize::new(1).unwrap(),
            Some(&format!(
                "field 1 (numeric representation) missing from DerivedNumericValues-3.2.0.txt: {}",
                line.line
            )),
        );
        let value = ucd32_fractions
            .get(&line.start)
            .copied()
            .unwrap_or_else(|| parse_numeric_value(field));

        ucd32_diffs.insert((line.start, line.end), value);
    }

    let ucd_latest = UnicodeLineReader::from_file_name("DerivedNumericValues.txt", true);
    let mut values_latest = Vec::new();
    for line in ucd_latest {
        // Field 3 holds the fraction; field 1 rounds it to a few digits.
        let field = line.field(
            NonZeroUsize::new(3).unwrap(),
            Some(&format!(
                "field 3 (fraction) missing from DerivedNumericValues.txt: {}",
                line.line
            )),
        );
        let value = parse_numeric_value(field);

        if ucd32_diffs
            .get(&(line.start, line.end))
            .is_some_and(|old_v| *old_v == value)
        {
            ucd32_diffs.remove(&(line.start, line.end));
        }

        values_latest.push((line.start, line.end, value));
    }

    let mut writer = open_writer("numeric_value_3_2.rs");
    write_slice_debug(
        &mut writer,
        "NUMERIC_VALUES",
        "(u32, u32, f64)",
        &mut values_latest,
    );
    let mut ucd32_diffs: Vec<_> = ucd32_diffs
        .into_iter()
        .map(|((start, end), value)| (start, end, value))
        .collect();
    write_slice_debug(
        &mut writer,
        "NUMERIC_VALUES_DIFF",
        "(u32, u32, f64)",
        &mut ucd32_diffs,
    );
}

fn general_category_3_2() {
    let mut writer = open_writer("gen_cat_3_2.rs");
    let reader = UnicodeLineReader::from_file_name("DerivedGeneralCategory-3.2.0.txt", false);

    let mut values = Vec::new();
    for line in reader {
        let field = line.field(
            NonZeroUsize::new(1).unwrap(),
            Some(&format!(
                "field 1 (category) missing from DerivedGeneralCategory-3.2.0.txt: {}",
                line.line
            )),
        );
        let id = parse_general(field);
        if id != GeneralCategory::Unassigned {
            let value = format!("GeneralCategory::{id:?}");
            values.push((line.start, line.end, value));
        }
    }

    write_slice_display(
        &mut writer,
        "GENERAL_CATEGORY",
        "(u32, u32, GeneralCategory)",
        &mut values,
    );
}

fn east_asian_width_3_2() {
    let mut writer = open_writer("eaw_3_2.rs");
    let reader = UnicodeLineReader::from_file_name("DerivedEastAsianWidth-3.2.0.txt", false);

    let mut values = Vec::new();
    for line in reader {
        let field = line.field(
            NonZeroUsize::new(1).unwrap(),
            Some(&format!(
                "field 1 (east Asian width class) missing from DerivedEastAsianWidth-3.2.0.txt: {}",
                line.line
            )),
        );
        let id = parse_eaw(field);
        if id != "EastAsianWidth::Neutral" {
            values.push((line.start, line.end, id));
        }
    }

    write_slice_display(
        &mut writer,
        "EAST_ASIAN_WIDTH",
        "(u32, u32, EastAsianWidth)",
        &mut values,
    );
}

fn bidi_class_3_2() {
    let mut writer = open_writer("bidi_class_3_2.rs");
    let reader = UnicodeLineReader::from_file_name("DerivedBidiClass-3.2.0.txt", false);

    let mut values = Vec::new();
    for line in reader {
        let field = line.field(
            NonZeroUsize::new(1).unwrap(),
            Some(&format!(
                "field 1 (BIDI class) missing from DerivedBidiClass-3.2.0.txt: {}",
                line.line
            )),
        );
        let id = parse_bidi(field);
        for i in line.start..=line.end {
            let legacy = BidiClass::try_from_str(id.rsplit_once("::").unwrap().1)
                .expect("Unicode data contains valid variants");
            let modern = char::from_u32(i).map(BidiClass::for_char);

            if Some(legacy) != modern {
                values.push((line.start, line.end, id));
                break;
            }
        }
    }

    write_slice_display(
        &mut writer,
        "BIDI_CLASS_DIFF",
        "(u32, u32, BidiClass)",
        &mut values,
    );
}

fn binary_props_3_2() {
    let mut writer = open_writer("binary_props_3_2.rs");
    let reader = UnicodeLineReader::from_file_name("DerivedBinaryProperties-3.2.0.txt", false);

    let mut values = Vec::new();
    for line in reader {
        let id = line.field(
            NonZeroUsize::new(1).unwrap(),
            Some(&format!(
                "field 1 (binary property) missing from DerivedBinaryProperties-3.2.0.txt: {}",
                line.line
            )),
        );
        assert_eq!(
            "Bidi_Mirrored",
            id.trim(),
            "DerivedBinaryProperties-3.2.0 only has Bidi_Mirrored"
        );
        values.push((line.start, line.end));
    }

    write_slice_pairs(&mut writer, "BIDI_MIRRORED", "(u32, u32)", &mut values);
}

fn combining_class_3_2() {
    let mut writer = open_writer("combining_class_3_2.rs");
    let reader = UnicodeLineReader::from_file_name("DerivedCombiningClass-3.2.0.txt", false);

    let mut values = Vec::new();
    for line in reader {
        let field = line.field(
            NonZeroUsize::new(1).unwrap(),
            Some(&format!(
                "field 1 (combining class) missing from DerivedCombiningClass-3.2.0.txt: {}",
                line.line
            )),
        );
        let id: u8 = field.parse().unwrap();
        if id != 0 {
            let value = format!("CanonicalCombiningClass::from_icu4c_value({id})");
            values.push((line.start, line.end, value));
        }
    }

    write_slice_display(
        &mut writer,
        "COMBINING_CLASS",
        "(u32, u32, CanonicalCombiningClass)",
        &mut values,
    );
}

fn generate_numeric_type_3_2() {
    let mut writer = open_writer("num_type_3_2.rs");
    let reader = UnicodeLineReader::from_file_name("DerivedNumericType-3.2.0.txt", false);

    let mut values = Vec::new();
    for line in reader {
        let field = line.field(
            NonZeroUsize::new(1).unwrap(),
            Some(&format!(
                "field 1 (numeric type) missing from DerivedNumericType-3.2.0.txt: {}",
                line.line
            )),
        );
        let id = parse_numeric_type_str(field);
        let differs = (line.start..=line.end).any(|c| match char::from_u32(c) {
            Some(c) => {
                let modern = parse_numeric_type_val(NumericType::for_char(c));
                modern != id
            }
            None => true,
        });

        if differs {
            values.push((line.start, line.end, id));
        }
    }

    write_slice_display(
        &mut writer,
        "NUMERIC_TYPE_DIFF",
        "(u32, u32, NumericType)",
        &mut values,
    );
}

/// Run each UCD parser in parallel.
fn drive_parsers() {
    let parsers = [
        full_data_parsers_latest,
        full_data_parsers_3_2,
        general_category_3_2,
        east_asian_width_3_2,
        bidi_class_3_2,
        binary_props_3_2,
        combining_class_3_2,
        generate_numeric_type_3_2,
    ];

    let mut handles = Vec::with_capacity(parsers.len());
    for parser in parsers {
        handles.push(thread::spawn(parser));
    }

    for handle in handles {
        handle.join().unwrap();
    }
}

fn parse_general(id: &str) -> GeneralCategory {
    match id.trim() {
        "Cn" => GeneralCategory::Unassigned,
        "Lu" => GeneralCategory::UppercaseLetter,
        "Ll" => GeneralCategory::LowercaseLetter,
        "Lt" => GeneralCategory::TitlecaseLetter,
        "Lm" => GeneralCategory::ModifierLetter,
        "Lo" => GeneralCategory::OtherLetter,
        "Mn" => GeneralCategory::NonspacingMark,
        "Mc" => GeneralCategory::SpacingMark,
        "Me" => GeneralCategory::EnclosingMark,
        "Nd" => GeneralCategory::DecimalNumber,
        "Nl" => GeneralCategory::LetterNumber,
        "No" => GeneralCategory::OtherNumber,
        "Zs" => GeneralCategory::SpaceSeparator,
        "Zl" => GeneralCategory::LineSeparator,
        "Zp" => GeneralCategory::ParagraphSeparator,
        "Cc" => GeneralCategory::Control,
        "Cf" => GeneralCategory::Format,
        "Co" => GeneralCategory::PrivateUse,
        "Cs" => GeneralCategory::Surrogate,
        "Pd" => GeneralCategory::DashPunctuation,
        "Ps" => GeneralCategory::OpenPunctuation,
        "Pe" => GeneralCategory::ClosePunctuation,
        "Pc" => GeneralCategory::ConnectorPunctuation,
        "Pi" => GeneralCategory::InitialPunctuation,
        "Pf" => GeneralCategory::FinalPunctuation,
        "Po" => GeneralCategory::OtherPunctuation,
        "Sm" => GeneralCategory::MathSymbol,
        "Sc" => GeneralCategory::CurrencySymbol,
        "Sk" => GeneralCategory::ModifierSymbol,
        "So" => GeneralCategory::OtherSymbol,
        invalid => unreachable!("Unicode data contains valid properties: {invalid}"),
    }
}

fn parse_eaw(id: &str) -> &'static str {
    match id.trim() {
        "N" => "EastAsianWidth::Neutral",
        "A" => "EastAsianWidth::Ambiguous",
        "H" => "EastAsianWidth::Halfwidth",
        "F" => "EastAsianWidth::Fullwidth",
        "Na" => "EastAsianWidth::Narrow",
        "W" => "EastAsianWidth::Wide",
        invalid => unreachable!("Unicode data contains valid properties: {invalid}"),
    }
}

fn parse_bidi(id: &str) -> &'static str {
    match id.trim() {
        "L" => "BidiClass::LeftToRight",
        "R" => "BidiClass::RightToLeft",
        "EN" => "BidiClass::EuropeanNumber",
        "ES" => "BidiClass::EuropeanSeparator",
        "ET" => "BidiClass::EuropeanTerminator",
        "AN" => "BidiClass::ArabicNumber",
        "CS" => "BidiClass::CommonSeparator",
        "B" => "BidiClass::ParagraphSeparator",
        "S" => "BidiClass::SegmentSeparator",
        "WS" => "BidiClass::WhiteSpace",
        "ON" => "BidiClass::OtherNeutral",
        "LRE" => "BidiClass::LeftToRightEmbedding",
        "LRO" => "BidiClass::LeftToRightOverride",
        "AL" => "BidiClass::ArabicLetter",
        "RLE" => "BidiClass::RightToLeftEmbedding",
        "RLO" => "BidiClass::RightToLeftOverride",
        "PDF" => "BidiClass::PopDirectionalFormat",
        "NSM" => "BidiClass::NonspacingMark",
        "BN" => "BidiClass::BoundaryNeutral",
        "FSI" => "BidiClass::FirstStrongIsolate",
        "LRI" => "BidiClass::LeftToRightIsolate",
        "RLI" => "BidiClass::RightToLeftIsolate",
        "PDI" => "BidiClass::PopDirectionalIsolate",
        invalid => unreachable!("Unicode data contains valid properties: {invalid}"),
    }
}

fn parse_numeric_type_val(val: NumericType) -> &'static str {
    match val {
        NumericType::None => "none",
        NumericType::Decimal => "decimal",
        NumericType::Digit => "digit",
        NumericType::Numeric => "numeric",
        _ => unreachable!("Unicode data contains valid properties"),
    }
}

fn parse_numeric_type_str(id: &str) -> &'static str {
    match id {
        "none" => "NumericType::None",
        "decimal" => "NumericType::Decimal",
        "digit" => "NumericType::Digit",
        "numeric" => "NumericType::Numeric",
        invalid => unreachable!("Unicode data contains valid properties: {invalid}"),
    }
}

#[derive(Debug, Default)]
enum DecompositionType {
    #[default]
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

fn parse_decomp_type(id: &str) -> DecompositionType {
    match id {
        "canonical" => DecompositionType::Canonical,
        "compat" => DecompositionType::Compat,
        "circle" => DecompositionType::Circle,
        "final" => DecompositionType::Final,
        "font" => DecompositionType::Font,
        "fraction" => DecompositionType::Fraction,
        "initial" => DecompositionType::Initial,
        "isolated" => DecompositionType::Isolated,
        "medial" => DecompositionType::Medial,
        "narrow" => DecompositionType::Narrow,
        "noBreak" => DecompositionType::Nobreak,
        "small" => DecompositionType::Small,
        "square" => DecompositionType::Square,
        "sub" => DecompositionType::Sub,
        "super" => DecompositionType::Super,
        "vertical" => DecompositionType::Vertical,
        "wide" => DecompositionType::Wide,
        invalid => unreachable!("Unicode data contains valid properties: {invalid}"),
    }
}

/// Read a numeric value written either on its own or as `numerator/denominator`.
fn parse_numeric_value(text: &str) -> f64 {
    let text = text.trim();
    let (numerator, denominator) = text.split_once('/').unwrap_or((text, "1"));
    let numerator: f64 = numerator
        .trim()
        .parse()
        .expect("Unicode data contains valid properties");
    let denominator: f64 = denominator
        .trim()
        .parse()
        .expect("Unicode data contains valid properties");
    numerator / denominator
}

fn main() {
    println!("cargo:rerun-if-changed=unicode/ucd32");
    println!("cargo:rerun-if-changed=unicode/latest");

    drive_parsers();
}
