use super::{AmplificationLimits, ErrorKind, ParserConfig, XmlEvent};

fn parse(source: &[u8], threshold: u64, factor: f32) -> (Option<ErrorKind>, String, Vec<String>) {
    let mut config = ParserConfig::new().coalesce_characters(false);
    config.amplification_limits = AmplificationLimits::new(factor, threshold);
    let mut reader = config.create_reader(source);
    let mut text = String::new();
    let mut attributes = Vec::new();
    loop {
        match reader.next() {
            Ok(XmlEvent::Characters(s) | XmlEvent::Whitespace(s)) => text.push_str(&s),
            Ok(XmlEvent::StartElement {
                attributes: attrs, ..
            }) => {
                attributes.extend(attrs.into_iter().map(|a| a.value));
            }
            Ok(XmlEvent::EndDocument) => return (None, text, attributes),
            Ok(_) => {}
            Err(error) => {
                assert_eq!(reader.next().unwrap_err(), error);
                return (Some(error.kind().clone()), text, attributes);
            }
        }
    }
}

#[test]
fn attribute_frontier_and_predefined_deferral() {
    let input = br#"<!DOCTYPE r [<!ENTITY a "xyz">]><r a="&a;"/>"#;
    assert_eq!(input.len(), 44);
    let (error, _, attributes) = parse(input, 0, 1.07);
    assert!(error.is_none());
    assert_eq!(attributes, ["xyz"]);
    assert!(matches!(
        parse(input, 0, 1.0).0,
        Some(ErrorKind::AmplificationLimit)
    ));
    assert!(parse(br#"<r a="&amp;"/>"#, 0, 1.0).0.is_none());
    let (error, text, _) = parse(b"<r>&amp;\nxxxxxxxxxxxxxxxxxxxx</r>", 0, 1.1);
    assert!(matches!(error, Some(ErrorKind::AmplificationLimit)));
    assert_eq!(text, "&");
}

#[test]
fn numeric_references_and_attribute_normalization() {
    let (error, text, _) = parse(b"<r>&#65;</r>", 0, 1.0);
    assert!(error.is_none());
    assert_eq!(text, "A");
    let (_, _, attrs) = parse(
        br#"<!DOCTYPE r [<!ENTITY a "&#9;&#10;&#13;">]><r a="&a;"/>"#,
        0,
        100.0,
    );
    assert_eq!(attrs, ["   "]);
    let (_, _, attrs) = parse(
        br#"<!DOCTYPE r [<!ENTITY a "&#13;&#10;">]><r a="&a;"/>"#,
        0,
        100.0,
    );
    assert_eq!(attrs, ["  "]);
    let (_, _, attrs) = parse(
        b"<!DOCTYPE r [<!ENTITY a \"\r\n\">]><r a=\"&a;\"/>",
        0,
        100.0,
    );
    assert_eq!(attrs, [" "]);
    for input in [
        br#"<!DOCTYPE r [<!ENTITY a "<">]><r a="&a;"/>"#.as_slice(),
        br#"<!DOCTYPE r [<!ENTITY a "&#38;">]><r a="&a;"/>"#.as_slice(),
    ] {
        assert!(matches!(
            parse(input, 0, 100.0).0,
            Some(ErrorKind::Syntax(_))
        ));
    }
}

#[test]
fn cdata_start_checks_deferred_predefined_bytes() {
    let input = b"<r>&amp;<![CDATA[xxxxxxxxxxxxxxxxxxxx]]></r>";
    let (error, text, _) = parse(input, 0, 1.05);
    assert!(matches!(error, Some(ErrorKind::AmplificationLimit)));
    assert_eq!(text, "&");
    let mut config = ParserConfig::new().cdata_to_characters(true);
    config.amplification_limits = AmplificationLimits::new(1.05, 0);
    let result = config
        .create_reader(input.as_slice())
        .into_iter()
        .collect::<Result<Vec<_>, _>>();
    assert!(matches!(
        result.unwrap_err().kind(),
        ErrorKind::AmplificationLimit
    ));
}

#[test]
fn encoded_input_uses_raw_bytes() {
    let source = "<!DOCTYPE r [<!ENTITY a \"xyz\"><!ENTITY b \"&a;\">]><r>&b;</r>";
    assert!(matches!(
        parse(source.as_bytes(), 0, 1.1).0,
        Some(ErrorKind::AmplificationLimit)
    ));
    let utf16: Vec<u8> = [0xff, 0xfe]
        .into_iter()
        .chain(source.encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    let (error, text, _) = parse(&utf16, 0, 1.1);
    assert!(error.is_none());
    assert_eq!(text, "xyz");
}

#[test]
fn fixed_caps_remain_independent_and_measure_nesting() {
    let sibling = "<!DOCTYPE r [<!ENTITY a \"x\">]><r>".to_owned() + &"&a;".repeat(20) + "</r>";
    assert_eq!(parse(sibling.as_bytes(), 10000, 1.0).1, "x".repeat(20));
    for depth in [4, 11, 20] {
        let mut source = "<!DOCTYPE r [<!ENTITY a0 \"x\">".to_owned();
        for n in 1..=depth {
            source += &format!("<!ENTITY a{n} \"&a{};\">", n - 1);
        }
        source += &format!("]><r>&a{depth};</r>");
        let (error, _, _) = parse(source.as_bytes(), 10000, f32::INFINITY);
        if depth == 4 {
            assert!(error.is_none());
        } else {
            assert!(matches!(error, Some(ErrorKind::Syntax(_))));
        }
    }
}
