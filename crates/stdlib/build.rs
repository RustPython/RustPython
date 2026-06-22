#![allow(
    clippy::disallowed_methods,
    reason = "build scripts cannot use rustpython-host_env"
)]

// spell-checker:ignore decomp DECOMP ossl osslconf

extern crate alloc;

use core::num::NonZeroUsize;

use alloc::collections::{BTreeMap, BTreeSet};

use std::{
    env,
    fs::{self, File},
    io::{self, BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    thread,
};

use icu_properties::props::{EnumeratedProperty, GeneralCategory, NumericType};

fn generate_unicode_3_2() {
    let path = PathBuf::from(env::var("OUT_DIR").unwrap())
        .join("generated")
        .join("unicode_3_2.rs");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut writer = BufWriter::new(File::create(&path).unwrap());

    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("unicode")
        .join("ucd32");

    write_derived(
        &base,
        "DerivedGeneralCategory-3.2.0.txt",
        "GENERAL_CATEGORY",
        "(u32, u32, GeneralCategory)",
        NonZeroUsize::new(1).unwrap(),
        &mut writer,
        |start, end, id, _| {
            let id = parse_general(id);
            if id != GeneralCategory::Unassigned {
                Some((start, end, id))
            } else {
                None
            }
        },
        |writer, mut values| {
            values.sort_unstable_by_key(|(start, _, _)| *start);
            write!(writer, "[").unwrap();
            for (start, end, id) in values {
                write!(writer, "({start}, {end}, GeneralCategory::{id:?}),").unwrap();
            }
            write!(writer, "];").unwrap();
        },
    );

    write_derived(
        &base,
        "DerivedEastAsianWidth-3.2.0.txt",
        "EAST_ASIAN_WIDTH",
        "(u32, u32, EastAsianWidth)",
        NonZeroUsize::new(1).unwrap(),
        &mut writer,
        |start, end, id, _| {
            let id = parse_eaw(id);
            if id != "EastAsianWidth::Neutral" {
                Some((start, end, id))
            } else {
                None
            }
        },
        |writer, mut values| {
            values.sort_unstable_by_key(|(start, _, _)| *start);
            write!(writer, "[").unwrap();
            for (start, end, id) in values {
                write!(writer, "({start}, {end}, {id}),").unwrap();
            }
            write!(writer, "];").unwrap();
        },
    );

    write_derived(
        &base,
        "DerivedBidiClass-3.2.0.txt",
        "BIDI_CLASS",
        "(u32, u32, BidiClass)",
        NonZeroUsize::new(1).unwrap(),
        &mut writer,
        |start, end, id, _| {
            let id = parse_bidi(id);
            if id != "BidiClass::LeftToRight" {
                Some((start, end, id))
            } else {
                None
            }
        },
        |writer, mut values| {
            values.sort_unstable_by_key(|(start, _, _)| *start);
            write!(writer, "[").unwrap();
            for (start, end, id) in values {
                write!(writer, "({start}, {end}, {id}),").unwrap();
            }
            write!(writer, "];").unwrap();
        },
    );

    write_derived(
        &base,
        "DerivedBinaryProperties-3.2.0.txt",
        "BIDI_MIRRORED",
        "(u32, u32)",
        NonZeroUsize::new(1).unwrap(),
        &mut writer,
        |start, end, id, _| {
            assert_eq!(
                "Bidi_Mirrored",
                id.trim(),
                "DerivedBinaryProperties-3.2.0 only has Bidi_Mirrored"
            );
            Some((start, end))
        },
        |writer, mut values| {
            values.sort_unstable_by_key(|(start, _)| *start);
            writeln!(writer, "{values:?};").unwrap();
        },
    );

    write_derived(
        &base,
        "DerivedCombiningClass-3.2.0.txt",
        "COMBINING_CLASS",
        "(u32, u32, CanonicalCombiningClass)",
        NonZeroUsize::new(1).unwrap(),
        &mut writer,
        |start, end, id, _| {
            let id: u8 = id.parse().unwrap();
            if id == 0 {
                return None;
            }
            Some((start, end, id))
        },
        |writer, mut values| {
            values.sort_unstable_by_key(|(start, _, _)| *start);
            write!(writer, "[").unwrap();
            for (start, end, id) in values {
                write!(
                    writer,
                    "({start}, {end}, CanonicalCombiningClass::from_icu4c_value({id})),"
                )
                .unwrap();
            }
            writeln!(writer, "];").unwrap();
        },
    );
}

fn generate_numeric_type() {
    let path = PathBuf::from(env::var("OUT_DIR").unwrap())
        .join("generated")
        .join("unicode_num_type.rs");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut writer = BufWriter::new(File::create(&path).unwrap());

    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("unicode")
        .join("ucd32");

    write_derived(
        &base,
        "DerivedNumericType-3.2.0.txt",
        "NUMERIC_TYPE_DIFF",
        "(u32, u32, NumericType)",
        NonZeroUsize::new(1).unwrap(),
        &mut writer,
        |start, end, id, _| {
            let id = parse_numeric_type_str(id);
            let differs = (start..=end).any(|c| match char::from_u32(c) {
                Some(c) => {
                    let modern = parse_numeric_type_val(NumericType::for_char(c));
                    modern != id
                }
                None => true,
            });

            if differs {
                Some((start, end, id))
            } else {
                None
            }
        },
        |writer, mut values| {
            values.sort_unstable_by_key(|(start, _, _)| *start);
            write!(writer, "[").unwrap();
            for (start, end, id) in values {
                write!(writer, "({start}, {end}, {id}),").unwrap();
            }
            writeln!(writer, "];").unwrap();
        },
    );
}

fn generate_numeric_value() {
    let path = PathBuf::from(env::var("OUT_DIR").unwrap())
        .join("generated")
        .join("unicode_numeric_value.rs");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut writer = BufWriter::new(File::create(&path).unwrap());

    // Ideally, this would store the diffs between the two tables. However, we need 3.2.0
    // membership as well as different chars. The final tables are both smaller than storing the
    // full 3.2.0 value table.
    let ucd32 = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("unicode")
        .join("ucd32");
    let mut ucd32_diffs = BTreeMap::new();
    let mut ucd32_member = BTreeSet::new();
    let numeric_32 =
        BufReader::new(File::open(ucd32.join("DerivedNumericValues-3.2.0.txt")).unwrap());
    parse_unicode_3_2(
        numeric_32,
        NonZeroUsize::new(1).unwrap(),
        &mut io::empty(),
        |start, end, value, _| {
            let value: f64 = value
                .parse()
                .expect("Unicode data contains valid properties");
            ucd32_diffs.insert((start, end), value);
            ucd32_member.insert((start, end));
            Option::<()>::None
        },
        |_writer, _values| {},
    );

    let ucd_latest = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("unicode")
        .join("latest");

    write_derived(
        &ucd_latest,
        "DerivedNumericValues.txt",
        "NUMERIC_VALUES",
        "(u32, u32, f64)",
        NonZeroUsize::new(1).unwrap(),
        &mut writer,
        |start, end, value, _| {
            let value: f64 = value
                .parse()
                .expect("Unicode data contains valid properties");

            if ucd32_diffs
                .get(&(start, end))
                .is_some_and(|old_v| *old_v == value)
            {
                ucd32_diffs.remove(&(start, end));
            }

            Some((start, end, value))
        },
        |writer, mut values| {
            values.sort_unstable_by_key(|(ch, _, _)| *ch);
            writeln!(writer, "{values:?};").unwrap();
        },
    );

    // TODO: More flexible parser
    writeln!(
        writer,
        "static NUMERIC_VALUES_DIFF: &[(u32, u32, f64)] = &["
    )
    .unwrap();
    for ((start, end), value) in ucd32_diffs {
        write!(writer, "({start}, {end}, {value:?}),").unwrap();
    }
    writeln!(writer, "];").unwrap();

    // Compress membership table
    let mut iter = ucd32_member.iter();
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
    membership.sort_unstable_by_key(|&(start, _)| start);

    writeln!(writer, "static NUMERIC_VAL_EXISTS_32: &[(u32, u32)] = &").unwrap();
    write!(writer, "{membership:?};").unwrap();
}

fn generate_unicode_latest() {
    let path = PathBuf::from(env::var("OUT_DIR").unwrap())
        .join("generated")
        .join("unicode_latest.rs");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut writer = BufWriter::new(File::create(&path).unwrap());

    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("unicode")
        .join("latest");

    // NOTE:
    // This ONLY parses compatibility decomposition because Python exposes the tags. The tags are
    // the "<square>", "<circle>", et cetera bits before the decomposition. Thus, we can save space
    // by using icu4x's CanonicalDecomposer for non-compatibility decomposition.
    let mut decomp_ranges = Vec::new();
    write_derived(
        &base,
        "UnicodeData.txt",
        "DECOMP_COMPAT",
        "(u32, DecompositionType, usize)",
        NonZeroUsize::new(5).unwrap(),
        &mut writer,
        |start, _end, value, _| {
            // We're building a sparse array. Most characters don't decompose, so we don't
            // need to literally store a row for each char.
            if value.is_empty() {
                return None;
            }

            let (dtype, decomp) = value.split_once('>').map(|(dtype, decomp)| {
                let dtype = dtype.strip_prefix('<').unwrap_or_else(|| {
                    panic!("Compatibility decomp; expected <tag>\n\tgot: {value}")
                });
                (
                    parse_decomp_type(dtype),
                    decomp
                        .split_whitespace()
                        .map(|s| u32::from_str_radix(s, 16).unwrap()),
                )
            })?;

            decomp_ranges.extend(decomp);
            let end = decomp_ranges.len();

            Some((start, dtype, end))
        },
        |writer, values| {
            // UnicodeData.txt should already be sorted
            write!(writer, "[").unwrap();
            for (start, dtype, end) in values {
                write!(writer, "({start}, DecompositionType::{dtype:?}, {end}),").unwrap();
            }
            writeln!(writer, "];").unwrap();
        },
    );

    writeln!(writer, "static DECOMP_RANGE: &[u32] = &{decomp_ranges:?};").unwrap();

    // Normalization corrections is super small - only a handful chars at the time of writing.
    write_derived(
        &base,
        "NormalizationCorrections.txt",
        "DECOMP_UPDATES",
        "(u32, u32)",
        NonZeroUsize::new(1).unwrap(),
        &mut writer,
        |start, _end, value, line| {
            let original = u32::from_str_radix(value.trim(), 16).unwrap_or_else(|e| {
                panic!("field 2 of decomp corrections should be a char in hex: {value} {e}")
            });
            let version = line
                .rsplit(';')
                .next()
                .unwrap_or_else(|| {
                    panic!("field 4 of decomp corrections should be a UCD version: {line}")
                })
                .split_once('#')
                .unwrap()
                .0
                .trim();

            // `version` = when the char was updated. Therefore, we use the incorrect chars past
            // 3.2.0 but skip the chars fixed in 3.2.0 because they'll already be right.
            if version != "3.2.0" {
                Some((start, original))
            } else {
                None
            }
        },
        |writer, mut values| {
            values.sort_unstable_by_key(|(c, _)| *c);
            write!(writer, "{values:?};").unwrap();
        },
    );
}

#[expect(clippy::too_many_arguments)]
fn write_derived<W, P, FW, T>(
    base: &Path,
    file_name: &str,
    static_name: &str,
    array_type: &str,
    field: NonZeroUsize,
    writer: &mut W,
    parse: P,
    write_vec: FW,
) where
    W: Write,
    P: FnMut(u32, u32, &str, &str) -> Option<T>,
    FW: FnMut(&mut W, Vec<T>),
{
    let path = base.join(file_name);
    let reader = BufReader::new(File::open(path).unwrap());
    writeln!(writer, "static {static_name}: &[{array_type}] = &").unwrap();
    parse_unicode_3_2(reader, field, writer, parse, write_vec);
}

/// Parse Unicode 3.2.0 property files.
fn parse_unicode_3_2<W, P, FW, T>(
    reader: impl BufRead,
    field: NonZeroUsize,
    writer: &mut W,
    mut parse: P,
    mut write_vec: FW,
) where
    W: Write,
    P: FnMut(u32, u32, &str, &str) -> Option<T>,
    FW: FnMut(&mut W, Vec<T>),
{
    let mut parsed = Vec::new();

    for line in reader.lines().map(Result::unwrap) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let mut fields = line.split(';');
        let range = fields.next().expect("Unicode data is missing a char range");
        let id = fields
            .nth(field.get().saturating_sub(1))
            .expect("Unicode data is missing a property");
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

        let id = id.split_once('#').map_or(id, |(left, _)| left).trim();
        if let Some(val) = parse(start, end, id, line) {
            parsed.push(val);
        }
    }
    write_vec(writer, parsed);
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

fn main() {
    println!(r#"cargo::rustc-check-cfg=cfg(osslconf, values("OPENSSL_NO_COMP"))"#);
    println!(r#"cargo::rustc-check-cfg=cfg(openssl_vendored)"#);

    #[allow(
        clippy::unusual_byte_groupings,
        reason = "hex groups follow OpenSSL version field boundaries"
    )]
    let ossl_vers = [
        (0x1_00_01_00_0, "ossl101"),
        (0x1_00_02_00_0, "ossl102"),
        (0x1_01_00_00_0, "ossl110"),
        (0x1_01_00_07_0, "ossl110g"),
        (0x1_01_00_08_0, "ossl110h"),
        (0x1_01_01_00_0, "ossl111"),
        (0x1_01_01_04_0, "ossl111d"),
        (0x3_00_00_00_0, "ossl300"),
        (0x3_01_00_00_0, "ossl310"),
        (0x3_02_00_00_0, "ossl320"),
        (0x3_03_00_00_0, "ossl330"),
    ];

    for (_, cfg) in ossl_vers {
        println!("cargo::rustc-check-cfg=cfg({cfg})");
    }

    #[cfg(feature = "ssl-openssl")]
    {
        #[allow(
            clippy::unusual_byte_groupings,
            reason = "OpenSSL version number is parsed with grouped hex fields"
        )]
        if let Ok(v) = std::env::var("DEP_OPENSSL_VERSION_NUMBER") {
            println!("cargo:rustc-env=OPENSSL_API_VERSION={v}");
            // cfg setup from openssl crate's build script
            let version = u64::from_str_radix(&v, 16).unwrap();
            for (ver, cfg) in ossl_vers {
                if version >= ver {
                    println!("cargo:rustc-cfg={cfg}");
                }
            }
        }
        if let Ok(v) = std::env::var("DEP_OPENSSL_CONF") {
            for conf in v.split(',') {
                println!("cargo:rustc-cfg=osslconf=\"{conf}\"");
            }
        }
        // it's possible for openssl-sys to link against the system openssl under certain conditions,
        // so let the ssl module know to only perform a probe if we're actually vendored
        if std::env::var("DEP_OPENSSL_VENDORED").is_ok_and(|s| s == "1") {
            println!("cargo::rustc-cfg=openssl_vendored")
        }
    }

    println!("cargo:rerun-if-changed=unicode/ucd32");
    println!("cargo:rerun-if-changed=unicode/latest");

    let t_32 = thread::spawn(generate_unicode_3_2);
    let t_numeric_type = thread::spawn(generate_numeric_type);
    let t_numeric_value = thread::spawn(generate_numeric_value);
    let t_latest = thread::spawn(generate_unicode_latest);
    t_32.join().unwrap();
    t_numeric_type.join().unwrap();
    t_numeric_value.join().unwrap();
    t_latest.join().unwrap();
}
