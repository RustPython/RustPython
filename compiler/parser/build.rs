use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use tiny_keccak::{Hasher, Sha3};

fn main() -> anyhow::Result<()> {
    const SOURCE: &str = "python.lalrpop";
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());

    println!("cargo:rerun-if-changed={SOURCE}");

    try_lalrpop(SOURCE, &out_dir.join("python.rs"))?;
    gen_phf(&out_dir);
    gen_unicode_aliases(&out_dir);

    Ok(())
}

fn requires_lalrpop(source: &str, target: &Path) -> Option<String> {
    let Ok(target) = File::open(target) else {
        return Some("python.rs doesn't exist. regenerate.".to_owned());
    };

    let sha_prefix = "// sha3: ";
    let sha3_line = if let Some(sha3_line) =
        BufReader::with_capacity(128, target)
            .lines()
            .find_map(|line| {
                let line = line.unwrap();
                line.starts_with(sha_prefix).then_some(line)
            }) {
        sha3_line
    } else {
        // no sha3 line - maybe old version of lalrpop installed
        return Some("python.rs doesn't include sha3 hash. regenerate.".to_owned());
    };
    let expected_sha3_str = sha3_line.strip_prefix(sha_prefix).unwrap();

    let actual_sha3 = {
        let mut hasher = Sha3::v256();
        let mut f = BufReader::new(File::open(source).unwrap());
        let mut line = String::new();
        while f.read_line(&mut line).unwrap() != 0 {
            if line.ends_with('\n') {
                line.pop();
                if line.ends_with('\r') {
                    line.pop();
                }
            }
            hasher.update(line.as_bytes());
            hasher.update(b"\n");
            line.clear();
        }
        let mut hash = [0u8; 32];
        hasher.finalize(&mut hash);
        hash
    };
    let eq = sha_equal(expected_sha3_str, &actual_sha3);
    if !eq {
        let mut actual_sha3_str = String::new();
        for byte in actual_sha3 {
            write!(actual_sha3_str, "{byte:02x}").unwrap();
        }
        return Some(format!(
            "python.rs hash expected: {expected_sha3_str} but actual: {actual_sha3_str}"
        ));
    }
    None
}

fn try_lalrpop(source: &str, target: &Path) -> anyhow::Result<()> {
    let Some(_message) = requires_lalrpop(source, target) else {
        return Ok(());
    };

    #[cfg(feature = "lalrpop")]
    // We are not using lalrpop::process_root() or Configuration::process_current_dir()
    // because of https://github.com/lalrpop/lalrpop/issues/699.
    lalrpop::Configuration::new()
        .use_cargo_dir_conventions()
        .set_in_dir(Path::new("."))
        .process()
        .unwrap_or_else(|e| {
            println!("cargo:warning={_message}");
            panic!("running lalrpop failed. {e:?}");
        });

    #[cfg(not(feature = "lalrpop"))]
    {
        println!("cargo:warning=try: cargo build --manifest-path=compiler/parser/Cargo.toml --features=lalrpop");
    }
    Ok(())
}

fn sha_equal(expected_sha3_str: &str, actual_sha3: &[u8; 32]) -> bool {
    if expected_sha3_str.len() != 64 {
        panic!("lalrpop version? hash bug is fixed in 0.19.8");
    }

    let mut expected_sha3 = [0u8; 32];
    for (i, b) in expected_sha3.iter_mut().enumerate() {
        *b = u8::from_str_radix(&expected_sha3_str[i * 2..][..2], 16).unwrap();
    }
    *actual_sha3 == expected_sha3
}

fn gen_phf(out_dir: &Path) {
    let mut kwds = phf_codegen::Map::new();
    let kwds = kwds
        // Alphabetical keywords:
        .entry("...", "Tok::Ellipsis")
        .entry("False", "Tok::False")
        .entry("None", "Tok::None")
        .entry("True", "Tok::True")
        // moreso "standard" keywords
        .entry("and", "Tok::And")
        .entry("as", "Tok::As")
        .entry("assert", "Tok::Assert")
        .entry("async", "Tok::Async")
        .entry("await", "Tok::Await")
        .entry("break", "Tok::Break")
        .entry("case", "Tok::Case")
        .entry("class", "Tok::Class")
        .entry("continue", "Tok::Continue")
        .entry("def", "Tok::Def")
        .entry("del", "Tok::Del")
        .entry("elif", "Tok::Elif")
        .entry("else", "Tok::Else")
        .entry("except", "Tok::Except")
        .entry("finally", "Tok::Finally")
        .entry("for", "Tok::For")
        .entry("from", "Tok::From")
        .entry("global", "Tok::Global")
        .entry("if", "Tok::If")
        .entry("import", "Tok::Import")
        .entry("in", "Tok::In")
        .entry("is", "Tok::Is")
        .entry("lambda", "Tok::Lambda")
        .entry("match", "Tok::Match")
        .entry("nonlocal", "Tok::Nonlocal")
        .entry("not", "Tok::Not")
        .entry("or", "Tok::Or")
        .entry("pass", "Tok::Pass")
        .entry("raise", "Tok::Raise")
        .entry("return", "Tok::Return")
        .entry("try", "Tok::Try")
        .entry("while", "Tok::While")
        .entry("with", "Tok::With")
        .entry("yield", "Tok::Yield")
        .build();
    writeln!(
        BufWriter::new(File::create(out_dir.join("keywords.rs")).unwrap()),
        "{kwds}",
    )
    .unwrap();
}

// Generate unicode aliases names,
// generated from https://www.unicode.org/Public/14.0.0/ucd/NameAliases.txt
fn gen_unicode_aliases(out_dir: &Path) {
    let mut aliases = phf_codegen::Map::new();
    // Make into separate statements so as to not (possibly) overflow the
    // compilers stack.
    // Require manual escape.
    aliases.entry("NEW LINE", "'\\u{000A}'");
    aliases.entry("END OF LINE", "'\\u{000A}'");
    aliases.entry("LINE FEED", "'\\u{000A}'");
    aliases.entry("LF", "'\\u{000A}'");
    aliases.entry("TAB", "'\\u{0009}'");
    aliases.entry("CARRIAGE RETURN", "'\\u{000D}'");
    aliases.entry("CR", "'\\u{000D}'");
    aliases.entry("NL", "'\\u{000A}'");
    aliases.entry("EOL", "'\\u{000A}'");
    aliases.entry("CHARACTER TABULATION", "'\\u{0009}'");
    aliases.entry("HORIZONTAL TABULATION", "'\\u{0009}'");
    aliases.entry("HT", "'\\u{0009}'");
    // Invisible characters:
    aliases.entry("WJ", "'\\u{2060}'");
    aliases.entry("SHY", "'\\u{00AD}'");
    aliases.entry("ZWSP", "'\\u{200B}'");
    // Fine as-is.
    aliases.entry("LRI", "'\\u{2066}'");
    aliases.entry("RLI", "'\\u{2067}'");
    aliases.entry("FSI", "'\\u{2068}'");
    aliases.entry("PDI", "'\\u{2069}'");
    aliases.entry("PDF", "'\\u{202C}'");
    aliases.entry("LRO", "'\\u{202D}'");
    aliases.entry("LRE", "'\\u{202A}'");
    aliases.entry("RLE", "'\\u{202B}'");
    aliases.entry("RLO", "'\\u{202E}'");
    aliases.entry("NULL", "'\u{0000}'");
    aliases.entry("NUL", "'\u{0000}'");
    aliases.entry("START OF HEADING", "'\u{0001}'");
    aliases.entry("SOH", "'\u{0001}'");
    aliases.entry("START OF TEXT", "'\u{0002}'");
    aliases.entry("STX", "'\u{0002}'");
    aliases.entry("END OF TEXT", "'\u{0003}'");
    aliases.entry("ETX", "'\u{0003}'");
    aliases.entry("END OF TRANSMISSION", "'\u{0004}'");
    aliases.entry("EOT", "'\u{0004}'");
    aliases.entry("ENQUIRY", "'\u{0005}'");
    aliases.entry("ENQ", "'\u{0005}'");
    aliases.entry("ACKNOWLEDGE", "'\u{0006}'");
    aliases.entry("ACK", "'\u{0006}'");
    aliases.entry("ALERT", "'\u{0007}'");
    aliases.entry("BEL", "'\u{0007}'");
    aliases.entry("BACKSPACE", "'\u{0008}'");
    aliases.entry("BS", "'\u{0008}'");
    aliases.entry("LINE TABULATION", "'\u{000B}'");
    aliases.entry("VERTICAL TABULATION", "'\u{000B}'");
    aliases.entry("VT", "'\u{000B}'");
    aliases.entry("FORM FEED", "'\u{000C}'");
    aliases.entry("FF", "'\u{000C}'");
    aliases.entry("SHIFT OUT", "'\u{000E}'");
    aliases.entry("LOCKING-SHIFT ONE", "'\u{000E}'");
    aliases.entry("SO", "'\u{000E}'");
    aliases.entry("SHIFT IN", "'\u{000F}'");
    aliases.entry("LOCKING-SHIFT ZERO", "'\u{000F}'");
    aliases.entry("SI", "'\u{000F}'");
    aliases.entry("DATA LINK ESCAPE", "'\u{0010}'");
    aliases.entry("DLE", "'\u{0010}'");
    aliases.entry("DEVICE CONTROL ONE", "'\u{0011}'");
    aliases.entry("DC1", "'\u{0011}'");
    aliases.entry("DEVICE CONTROL TWO", "'\u{0012}'");
    aliases.entry("DC2", "'\u{0012}'");
    aliases.entry("DEVICE CONTROL THREE", "'\u{0013}'");
    aliases.entry("DC3", "'\u{0013}'");
    aliases.entry("DEVICE CONTROL FOUR", "'\u{0014}'");
    aliases.entry("DC4", "'\u{0014}'");
    aliases.entry("NEGATIVE ACKNOWLEDGE", "'\u{0015}'");
    aliases.entry("NAK", "'\u{0015}'");
    aliases.entry("SYNCHRONOUS IDLE", "'\u{0016}'");
    aliases.entry("SYN", "'\u{0016}'");
    aliases.entry("END OF TRANSMISSION BLOCK", "'\u{0017}'");
    aliases.entry("ETB", "'\u{0017}'");
    aliases.entry("CANCEL", "'\u{0018}'");
    aliases.entry("CAN", "'\u{0018}'");
    aliases.entry("END OF MEDIUM", "'\u{0019}'");
    aliases.entry("EOM", "'\u{0019}'");
    aliases.entry("SUBSTITUTE", "'\u{001A}'");
    aliases.entry("SUB", "'\u{001A}'");
    aliases.entry("ESCAPE", "'\u{001B}'");
    aliases.entry("ESC", "'\u{001B}'");
    aliases.entry("INFORMATION SEPARATOR FOUR", "'\u{001C}'");
    aliases.entry("FILE SEPARATOR", "'\u{001C}'");
    aliases.entry("FS", "'\u{001C}'");
    aliases.entry("INFORMATION SEPARATOR THREE", "'\u{001D}'");
    aliases.entry("GROUP SEPARATOR", "'\u{001D}'");
    aliases.entry("GS", "'\u{001D}'");
    aliases.entry("INFORMATION SEPARATOR TWO", "'\u{001E}'");
    aliases.entry("RECORD SEPARATOR", "'\u{001E}'");
    aliases.entry("RS", "'\u{001E}'");
    aliases.entry("INFORMATION SEPARATOR ONE", "'\u{001F}'");
    aliases.entry("UNIT SEPARATOR", "'\u{001F}'");
    aliases.entry("US", "'\u{001F}'");
    aliases.entry("SP", "'\u{0020}'");
    aliases.entry("DELETE", "'\u{007F}'");
    aliases.entry("DEL", "'\u{007F}'");
    aliases.entry("PADDING CHARACTER", "'\u{0080}'");
    aliases.entry("PAD", "'\u{0080}'");
    aliases.entry("HIGH OCTET PRESET", "'\u{0081}'");
    aliases.entry("HOP", "'\u{0081}'");
    aliases.entry("BREAK PERMITTED HERE", "'\u{0082}'");
    aliases.entry("BPH", "'\u{0082}'");
    aliases.entry("NO BREAK HERE", "'\u{0083}'");
    aliases.entry("NBH", "'\u{0083}'");
    aliases.entry("INDEX", "'\u{0084}'");
    aliases.entry("IND", "'\u{0084}'");
    aliases.entry("NEXT LINE", "'\u{0085}'");
    aliases.entry("NEL", "'\u{0085}'");
    aliases.entry("START OF SELECTED AREA", "'\u{0086}'");
    aliases.entry("SSA", "'\u{0086}'");
    aliases.entry("END OF SELECTED AREA", "'\u{0087}'");
    aliases.entry("ESA", "'\u{0087}'");
    aliases.entry("CHARACTER TABULATION SET", "'\u{0088}'");
    aliases.entry("HORIZONTAL TABULATION SET", "'\u{0088}'");
    aliases.entry("HTS", "'\u{0088}'");
    aliases.entry("CHARACTER TABULATION WITH JUSTIFICATION", "'\u{0089}'");
    aliases.entry("HORIZONTAL TABULATION WITH JUSTIFICATION", "'\u{0089}'");
    aliases.entry("HTJ", "'\u{0089}'");
    aliases.entry("LINE TABULATION SET", "'\u{008A}'");
    aliases.entry("VERTICAL TABULATION SET", "'\u{008A}'");
    aliases.entry("VTS", "'\u{008A}'");
    aliases.entry("PARTIAL LINE FORWARD", "'\u{008B}'");
    aliases.entry("PARTIAL LINE DOWN", "'\u{008B}'");
    aliases.entry("PLD", "'\u{008B}'");
    aliases.entry("PARTIAL LINE BACKWARD", "'\u{008C}'");
    aliases.entry("PARTIAL LINE UP", "'\u{008C}'");
    aliases.entry("PLU", "'\u{008C}'");
    aliases.entry("REVERSE LINE FEED", "'\u{008D}'");
    aliases.entry("REVERSE INDEX", "'\u{008D}'");
    aliases.entry("RI", "'\u{008D}'");
    aliases.entry("SINGLE SHIFT TWO", "'\u{008E}'");
    aliases.entry("SINGLE-SHIFT-2", "'\u{008E}'");
    aliases.entry("SS2", "'\u{008E}'");
    aliases.entry("SINGLE SHIFT THREE", "'\u{008F}'");
    aliases.entry("SINGLE-SHIFT-3", "'\u{008F}'");
    aliases.entry("SS3", "'\u{008F}'");
    aliases.entry("DEVICE CONTROL STRING", "'\u{0090}'");
    aliases.entry("DCS", "'\u{0090}'");
    aliases.entry("PRIVATE USE ONE", "'\u{0091}'");
    aliases.entry("PRIVATE USE-1", "'\u{0091}'");
    aliases.entry("PU1", "'\u{0091}'");
    aliases.entry("PRIVATE USE TWO", "'\u{0092}'");
    aliases.entry("PRIVATE USE-2", "'\u{0092}'");
    aliases.entry("PU2", "'\u{0092}'");
    aliases.entry("SET TRANSMIT STATE", "'\u{0093}'");
    aliases.entry("STS", "'\u{0093}'");
    aliases.entry("CANCEL CHARACTER", "'\u{0094}'");
    aliases.entry("CCH", "'\u{0094}'");
    aliases.entry("MESSAGE WAITING", "'\u{0095}'");
    aliases.entry("MW", "'\u{0095}'");
    aliases.entry("START OF GUARDED AREA", "'\u{0096}'");
    aliases.entry("START OF PROTECTED AREA", "'\u{0096}'");
    aliases.entry("SPA", "'\u{0096}'");
    aliases.entry("END OF GUARDED AREA", "'\u{0097}'");
    aliases.entry("END OF PROTECTED AREA", "'\u{0097}'");
    aliases.entry("EPA", "'\u{0097}'");
    aliases.entry("START OF STRING", "'\u{0098}'");
    aliases.entry("SOS", "'\u{0098}'");
    aliases.entry("SINGLE GRAPHIC CHARACTER INTRODUCER", "'\u{0099}'");
    aliases.entry("SGC", "'\u{0099}'");
    aliases.entry("SINGLE CHARACTER INTRODUCER", "'\u{009A}'");
    aliases.entry("SCI", "'\u{009A}'");
    aliases.entry("CONTROL SEQUENCE INTRODUCER", "'\u{009B}'");
    aliases.entry("CSI", "'\u{009B}'");
    aliases.entry("STRING TERMINATOR", "'\u{009C}'");
    aliases.entry("ST", "'\u{009C}'");
    aliases.entry("OPERATING SYSTEM COMMAND", "'\u{009D}'");
    aliases.entry("OSC", "'\u{009D}'");
    aliases.entry("PRIVACY MESSAGE", "'\u{009E}'");
    aliases.entry("PM", "'\u{009E}'");
    aliases.entry("APPLICATION PROGRAM COMMAND", "'\u{009F}'");
    aliases.entry("APC", "'\u{009F}'");
    aliases.entry("NBSP", "'\u{00A0}'");
    aliases.entry("LATIN CAPITAL LETTER GHA", "'\u{01A2}'");
    aliases.entry("LATIN SMALL LETTER GHA", "'\u{01A3}'");
    aliases.entry("CGJ", "'\u{034F}'");
    aliases.entry("ALM", "'\u{061C}'");
    aliases.entry("SYRIAC SUBLINEAR COLON SKEWED LEFT", "'\u{0709}'");
    aliases.entry("KANNADA LETTER LLLA", "'\u{0CDE}'");
    aliases.entry("LAO LETTER FO FON", "'\u{0E9D}'");
    aliases.entry("LAO LETTER FO FAY", "'\u{0E9F}'");
    aliases.entry("LAO LETTER RO", "'\u{0EA3}'");
    aliases.entry("LAO LETTER LO", "'\u{0EA5}'");
    aliases.entry("TIBETAN MARK BKA- SHOG GI MGO RGYAN", "'\u{0FD0}'");
    aliases.entry("HANGUL JONGSEONG YESIEUNG-KIYEOK", "'\u{11EC}'");
    aliases.entry("HANGUL JONGSEONG YESIEUNG-SSANGKIYEOK", "'\u{11ED}'");
    aliases.entry("HANGUL JONGSEONG SSANGYESIEUNG", "'\u{11EE}'");
    aliases.entry("HANGUL JONGSEONG YESIEUNG-KHIEUKH", "'\u{11EF}'");
    aliases.entry("FVS1", "'\u{180B}'");
    aliases.entry("FVS2", "'\u{180C}'");
    aliases.entry("FVS3", "'\u{180D}'");
    aliases.entry("MVS", "'\u{180E}'");
    aliases.entry("ZWNJ", "'\u{200C}'");
    aliases.entry("ZWJ", "'\u{200D}'");
    aliases.entry("LRM", "'\u{200E}'");
    aliases.entry("RLM", "'\u{200F}'");
    aliases.entry("NNBSP", "'\u{202F}'");
    aliases.entry("MMSP", "'\u{205F}'");
    aliases.entry("WEIERSTRASS ELLIPTIC FUNCTION", "'\u{2118}'");
    aliases.entry("MICR ON US SYMBOL", "'\u{2448}'");
    aliases.entry("MICR DASH SYMBOL", "'\u{2449}'");
    aliases.entry(
        "LEFTWARDS TRIANGLE-HEADED ARROW WITH DOUBLE VERTICAL STROKE",
        "'\u{2B7A}'",
    );
    aliases.entry(
        "RIGHTWARDS TRIANGLE-HEADED ARROW WITH DOUBLE VERTICAL STROKE",
        "'\u{2B7C}'",
    );
    aliases.entry("YI SYLLABLE ITERATION MARK", "'\u{A015}'");
    aliases.entry("VS1", "'\u{FE00}'");
    aliases.entry("VS2", "'\u{FE01}'");
    aliases.entry("VS3", "'\u{FE02}'");
    aliases.entry("VS4", "'\u{FE03}'");
    aliases.entry("VS5", "'\u{FE04}'");
    aliases.entry("VS6", "'\u{FE05}'");
    aliases.entry("VS7", "'\u{FE06}'");
    aliases.entry("VS8", "'\u{FE07}'");
    aliases.entry("VS9", "'\u{FE08}'");
    aliases.entry("VS10", "'\u{FE09}'");
    aliases.entry("VS11", "'\u{FE0A}'");
    aliases.entry("VS12", "'\u{FE0B}'");
    aliases.entry("VS13", "'\u{FE0C}'");
    aliases.entry("VS14", "'\u{FE0D}'");
    aliases.entry("VS15", "'\u{FE0E}'");
    aliases.entry("VS16", "'\u{FE0F}'");
    aliases.entry(
        "PRESENTATION FORM FOR VERTICAL RIGHT WHITE LENTICULAR BRACKET",
        "'\u{FE18}'",
    );
    aliases.entry("BYTE ORDER MARK", "'\u{FEFF}'");
    aliases.entry("BOM", "'\u{FEFF}'");
    aliases.entry("ZWNBSP", "'\u{FEFF}'");
    aliases.entry("CUNEIFORM SIGN NU11 TENU", "'\u{122D4}'");
    aliases.entry("CUNEIFORM SIGN NU11 OVER NU11 BUR OVER BUR", "'\u{122D5}'");
    aliases.entry("MEDEFAIDRIN CAPITAL LETTER H", "'\u{16E56}'");
    aliases.entry("MEDEFAIDRIN CAPITAL LETTER NG", "'\u{16E57}'");
    aliases.entry("MEDEFAIDRIN SMALL LETTER H", "'\u{16E76}'");
    aliases.entry("MEDEFAIDRIN SMALL LETTER NG", "'\u{16E77}'");
    aliases.entry("HENTAIGANA LETTER E-1", "'\u{1B001}'");
    aliases.entry(
        "BYZANTINE MUSICAL SYMBOL FTHORA SKLIRON CHROMA VASIS",
        "'\u{1D0C5}'",
    );
    aliases.entry("VS17", "'\u{E0100}'");
    aliases.entry("VS18", "'\u{E0101}'");
    aliases.entry("VS19", "'\u{E0102}'");
    aliases.entry("VS20", "'\u{E0103}'");
    aliases.entry("VS21", "'\u{E0104}'");
    aliases.entry("VS22", "'\u{E0105}'");
    aliases.entry("VS23", "'\u{E0106}'");
    aliases.entry("VS24", "'\u{E0107}'");
    aliases.entry("VS25", "'\u{E0108}'");
    aliases.entry("VS26", "'\u{E0109}'");
    aliases.entry("VS27", "'\u{E010A}'");
    aliases.entry("VS28", "'\u{E010B}'");
    aliases.entry("VS29", "'\u{E010C}'");
    aliases.entry("VS30", "'\u{E010D}'");
    aliases.entry("VS31", "'\u{E010E}'");
    aliases.entry("VS32", "'\u{E010F}'");
    aliases.entry("VS33", "'\u{E0110}'");
    aliases.entry("VS34", "'\u{E0111}'");
    aliases.entry("VS35", "'\u{E0112}'");
    aliases.entry("VS36", "'\u{E0113}'");
    aliases.entry("VS37", "'\u{E0114}'");
    aliases.entry("VS38", "'\u{E0115}'");
    aliases.entry("VS39", "'\u{E0116}'");
    aliases.entry("VS40", "'\u{E0117}'");
    aliases.entry("VS41", "'\u{E0118}'");
    aliases.entry("VS42", "'\u{E0119}'");
    aliases.entry("VS43", "'\u{E011A}'");
    aliases.entry("VS44", "'\u{E011B}'");
    aliases.entry("VS45", "'\u{E011C}'");
    aliases.entry("VS46", "'\u{E011D}'");
    aliases.entry("VS47", "'\u{E011E}'");
    aliases.entry("VS48", "'\u{E011F}'");
    aliases.entry("VS49", "'\u{E0120}'");
    aliases.entry("VS50", "'\u{E0121}'");
    aliases.entry("VS51", "'\u{E0122}'");
    aliases.entry("VS52", "'\u{E0123}'");
    aliases.entry("VS53", "'\u{E0124}'");
    aliases.entry("VS54", "'\u{E0125}'");
    aliases.entry("VS55", "'\u{E0126}'");
    aliases.entry("VS56", "'\u{E0127}'");
    aliases.entry("VS57", "'\u{E0128}'");
    aliases.entry("VS58", "'\u{E0129}'");
    aliases.entry("VS59", "'\u{E012A}'");
    aliases.entry("VS60", "'\u{E012B}'");
    aliases.entry("VS61", "'\u{E012C}'");
    aliases.entry("VS62", "'\u{E012D}'");
    aliases.entry("VS63", "'\u{E012E}'");
    aliases.entry("VS64", "'\u{E012F}'");
    aliases.entry("VS65", "'\u{E0130}'");
    aliases.entry("VS66", "'\u{E0131}'");
    aliases.entry("VS67", "'\u{E0132}'");
    aliases.entry("VS68", "'\u{E0133}'");
    aliases.entry("VS69", "'\u{E0134}'");
    aliases.entry("VS70", "'\u{E0135}'");
    aliases.entry("VS71", "'\u{E0136}'");
    aliases.entry("VS72", "'\u{E0137}'");
    aliases.entry("VS73", "'\u{E0138}'");
    aliases.entry("VS74", "'\u{E0139}'");
    aliases.entry("VS75", "'\u{E013A}'");
    aliases.entry("VS76", "'\u{E013B}'");
    aliases.entry("VS77", "'\u{E013C}'");
    aliases.entry("VS78", "'\u{E013D}'");
    aliases.entry("VS79", "'\u{E013E}'");
    aliases.entry("VS80", "'\u{E013F}'");
    aliases.entry("VS81", "'\u{E0140}'");
    aliases.entry("VS82", "'\u{E0141}'");
    aliases.entry("VS83", "'\u{E0142}'");
    aliases.entry("VS84", "'\u{E0143}'");
    aliases.entry("VS85", "'\u{E0144}'");
    aliases.entry("VS86", "'\u{E0145}'");
    aliases.entry("VS87", "'\u{E0146}'");
    aliases.entry("VS88", "'\u{E0147}'");
    aliases.entry("VS89", "'\u{E0148}'");
    aliases.entry("VS90", "'\u{E0149}'");
    aliases.entry("VS91", "'\u{E014A}'");
    aliases.entry("VS92", "'\u{E014B}'");
    aliases.entry("VS93", "'\u{E014C}'");
    aliases.entry("VS94", "'\u{E014D}'");
    aliases.entry("VS95", "'\u{E014E}'");
    aliases.entry("VS96", "'\u{E014F}'");
    aliases.entry("VS97", "'\u{E0150}'");
    aliases.entry("VS98", "'\u{E0151}'");
    aliases.entry("VS99", "'\u{E0152}'");
    aliases.entry("VS100", "'\u{E0153}'");
    aliases.entry("VS101", "'\u{E0154}'");
    aliases.entry("VS102", "'\u{E0155}'");
    aliases.entry("VS103", "'\u{E0156}'");
    aliases.entry("VS104", "'\u{E0157}'");
    aliases.entry("VS105", "'\u{E0158}'");
    aliases.entry("VS106", "'\u{E0159}'");
    aliases.entry("VS107", "'\u{E015A}'");
    aliases.entry("VS108", "'\u{E015B}'");
    aliases.entry("VS109", "'\u{E015C}'");
    aliases.entry("VS110", "'\u{E015D}'");
    aliases.entry("VS111", "'\u{E015E}'");
    aliases.entry("VS112", "'\u{E015F}'");
    aliases.entry("VS113", "'\u{E0160}'");
    aliases.entry("VS114", "'\u{E0161}'");
    aliases.entry("VS115", "'\u{E0162}'");
    aliases.entry("VS116", "'\u{E0163}'");
    aliases.entry("VS117", "'\u{E0164}'");
    aliases.entry("VS118", "'\u{E0165}'");
    aliases.entry("VS119", "'\u{E0166}'");
    aliases.entry("VS120", "'\u{E0167}'");
    aliases.entry("VS121", "'\u{E0168}'");
    aliases.entry("VS122", "'\u{E0169}'");
    aliases.entry("VS123", "'\u{E016A}'");
    aliases.entry("VS124", "'\u{E016B}'");
    aliases.entry("VS125", "'\u{E016C}'");
    aliases.entry("VS126", "'\u{E016D}'");
    aliases.entry("VS127", "'\u{E016E}'");
    aliases.entry("VS128", "'\u{E016F}'");
    aliases.entry("VS129", "'\u{E0170}'");
    aliases.entry("VS130", "'\u{E0171}'");
    aliases.entry("VS131", "'\u{E0172}'");
    aliases.entry("VS132", "'\u{E0173}'");
    aliases.entry("VS133", "'\u{E0174}'");
    aliases.entry("VS134", "'\u{E0175}'");
    aliases.entry("VS135", "'\u{E0176}'");
    aliases.entry("VS136", "'\u{E0177}'");
    aliases.entry("VS137", "'\u{E0178}'");
    aliases.entry("VS138", "'\u{E0179}'");
    aliases.entry("VS139", "'\u{E017A}'");
    aliases.entry("VS140", "'\u{E017B}'");
    aliases.entry("VS141", "'\u{E017C}'");
    aliases.entry("VS142", "'\u{E017D}'");
    aliases.entry("VS143", "'\u{E017E}'");
    aliases.entry("VS144", "'\u{E017F}'");
    aliases.entry("VS145", "'\u{E0180}'");
    aliases.entry("VS146", "'\u{E0181}'");
    aliases.entry("VS147", "'\u{E0182}'");
    aliases.entry("VS148", "'\u{E0183}'");
    aliases.entry("VS149", "'\u{E0184}'");
    aliases.entry("VS150", "'\u{E0185}'");
    aliases.entry("VS151", "'\u{E0186}'");
    aliases.entry("VS152", "'\u{E0187}'");
    aliases.entry("VS153", "'\u{E0188}'");
    aliases.entry("VS154", "'\u{E0189}'");
    aliases.entry("VS155", "'\u{E018A}'");
    aliases.entry("VS156", "'\u{E018B}'");
    aliases.entry("VS157", "'\u{E018C}'");
    aliases.entry("VS158", "'\u{E018D}'");
    aliases.entry("VS159", "'\u{E018E}'");
    aliases.entry("VS160", "'\u{E018F}'");
    aliases.entry("VS161", "'\u{E0190}'");
    aliases.entry("VS162", "'\u{E0191}'");
    aliases.entry("VS163", "'\u{E0192}'");
    aliases.entry("VS164", "'\u{E0193}'");
    aliases.entry("VS165", "'\u{E0194}'");
    aliases.entry("VS166", "'\u{E0195}'");
    aliases.entry("VS167", "'\u{E0196}'");
    aliases.entry("VS168", "'\u{E0197}'");
    aliases.entry("VS169", "'\u{E0198}'");
    aliases.entry("VS170", "'\u{E0199}'");
    aliases.entry("VS171", "'\u{E019A}'");
    aliases.entry("VS172", "'\u{E019B}'");
    aliases.entry("VS173", "'\u{E019C}'");
    aliases.entry("VS174", "'\u{E019D}'");
    aliases.entry("VS175", "'\u{E019E}'");
    aliases.entry("VS176", "'\u{E019F}'");
    aliases.entry("VS177", "'\u{E01A0}'");
    aliases.entry("VS178", "'\u{E01A1}'");
    aliases.entry("VS179", "'\u{E01A2}'");
    aliases.entry("VS180", "'\u{E01A3}'");
    aliases.entry("VS181", "'\u{E01A4}'");
    aliases.entry("VS182", "'\u{E01A5}'");
    aliases.entry("VS183", "'\u{E01A6}'");
    aliases.entry("VS184", "'\u{E01A7}'");
    aliases.entry("VS185", "'\u{E01A8}'");
    aliases.entry("VS186", "'\u{E01A9}'");
    aliases.entry("VS187", "'\u{E01AA}'");
    aliases.entry("VS188", "'\u{E01AB}'");
    aliases.entry("VS189", "'\u{E01AC}'");
    aliases.entry("VS190", "'\u{E01AD}'");
    aliases.entry("VS191", "'\u{E01AE}'");
    aliases.entry("VS192", "'\u{E01AF}'");
    aliases.entry("VS193", "'\u{E01B0}'");
    aliases.entry("VS194", "'\u{E01B1}'");
    aliases.entry("VS195", "'\u{E01B2}'");
    aliases.entry("VS196", "'\u{E01B3}'");
    aliases.entry("VS197", "'\u{E01B4}'");
    aliases.entry("VS198", "'\u{E01B5}'");
    aliases.entry("VS199", "'\u{E01B6}'");
    aliases.entry("VS200", "'\u{E01B7}'");
    aliases.entry("VS201", "'\u{E01B8}'");
    aliases.entry("VS202", "'\u{E01B9}'");
    aliases.entry("VS203", "'\u{E01BA}'");
    aliases.entry("VS204", "'\u{E01BB}'");
    aliases.entry("VS205", "'\u{E01BC}'");
    aliases.entry("VS206", "'\u{E01BD}'");
    aliases.entry("VS207", "'\u{E01BE}'");
    aliases.entry("VS208", "'\u{E01BF}'");
    aliases.entry("VS209", "'\u{E01C0}'");
    aliases.entry("VS210", "'\u{E01C1}'");
    aliases.entry("VS211", "'\u{E01C2}'");
    aliases.entry("VS212", "'\u{E01C3}'");
    aliases.entry("VS213", "'\u{E01C4}'");
    aliases.entry("VS214", "'\u{E01C5}'");
    aliases.entry("VS215", "'\u{E01C6}'");
    aliases.entry("VS216", "'\u{E01C7}'");
    aliases.entry("VS217", "'\u{E01C8}'");
    aliases.entry("VS218", "'\u{E01C9}'");
    aliases.entry("VS219", "'\u{E01CA}'");
    aliases.entry("VS220", "'\u{E01CB}'");
    aliases.entry("VS221", "'\u{E01CC}'");
    aliases.entry("VS222", "'\u{E01CD}'");
    aliases.entry("VS223", "'\u{E01CE}'");
    aliases.entry("VS224", "'\u{E01CF}'");
    aliases.entry("VS225", "'\u{E01D0}'");
    aliases.entry("VS226", "'\u{E01D1}'");
    aliases.entry("VS227", "'\u{E01D2}'");
    aliases.entry("VS228", "'\u{E01D3}'");
    aliases.entry("VS229", "'\u{E01D4}'");
    aliases.entry("VS230", "'\u{E01D5}'");
    aliases.entry("VS231", "'\u{E01D6}'");
    aliases.entry("VS232", "'\u{E01D7}'");
    aliases.entry("VS233", "'\u{E01D8}'");
    aliases.entry("VS234", "'\u{E01D9}'");
    aliases.entry("VS235", "'\u{E01DA}'");
    aliases.entry("VS236", "'\u{E01DB}'");
    aliases.entry("VS237", "'\u{E01DC}'");
    aliases.entry("VS238", "'\u{E01DD}'");
    aliases.entry("VS239", "'\u{E01DE}'");
    aliases.entry("VS240", "'\u{E01DF}'");
    aliases.entry("VS241", "'\u{E01E0}'");
    aliases.entry("VS242", "'\u{E01E1}'");
    aliases.entry("VS243", "'\u{E01E2}'");
    aliases.entry("VS244", "'\u{E01E3}'");
    aliases.entry("VS245", "'\u{E01E4}'");
    aliases.entry("VS246", "'\u{E01E5}'");
    aliases.entry("VS247", "'\u{E01E6}'");
    aliases.entry("VS248", "'\u{E01E7}'");
    aliases.entry("VS249", "'\u{E01E8}'");
    aliases.entry("VS250", "'\u{E01E9}'");
    aliases.entry("VS251", "'\u{E01EA}'");
    aliases.entry("VS252", "'\u{E01EB}'");
    aliases.entry("VS253", "'\u{E01EC}'");
    aliases.entry("VS254", "'\u{E01ED}'");
    aliases.entry("VS255", "'\u{E01EE}'");
    aliases.entry("VS256", "'\u{E01EF}'");

    let aliases = aliases.build();
    writeln!(
        BufWriter::new(File::create(out_dir.join("aliases.rs")).unwrap()),
        "{aliases}",
    )
    .unwrap();
}
