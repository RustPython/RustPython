//! X.509 decode helpers used by `_ssl.Certificate` and `get_ca_certs()`.

use x509_parser::prelude::*;

/// One RDN, as a list of `(attribute name, value)` pairs.
pub type DistinguishedName = Vec<Vec<(String, String)>>;

/// One subjectAltName entry after the names `_ssl` publishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectAlternativeName {
    pub kind: &'static str,
    pub value: String,
    pub directory_name: DistinguishedName,
}

/// Owned projection of the X.509 fields exposed by `_test_decode_cert()`
/// and by `SSLContext.get_ca_certs()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedCertificate {
    pub issuer: DistinguishedName,
    pub subject: DistinguishedName,
    pub not_after: String,
    pub not_before: String,
    pub serial_number: String,
    pub version: i32,
    pub ocsp: Vec<String>,
    pub ca_issuers: Vec<String>,
    pub crl_distribution_points: Vec<String>,
    pub subject_alt_names: Vec<SubjectAlternativeName>,
}

/// Map a distinguished-name OID to the attribute name `_ssl` reports.
#[must_use]
pub fn oid_to_attribute_name(oid_str: &str) -> &str {
    match oid_str {
        "2.5.4.3" => "commonName",
        "2.5.4.6" => "countryName",
        "2.5.4.7" => "localityName",
        "2.5.4.8" => "stateOrProvinceName",
        "2.5.4.10" => "organizationName",
        "2.5.4.11" => "organizationalUnitName",
        "1.2.840.113549.1.9.1" => "emailAddress",
        _ => oid_str,
    }
}

/// Format raw IP address bytes the way `getpeercert()` prints them.
#[must_use]
pub fn format_ip_address(ip: &[u8]) -> String {
    if ip.len() == 4 {
        format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
    } else if ip.len() == 16 {
        let segments = [
            u16::from_be_bytes([ip[0], ip[1]]),
            u16::from_be_bytes([ip[2], ip[3]]),
            u16::from_be_bytes([ip[4], ip[5]]),
            u16::from_be_bytes([ip[6], ip[7]]),
            u16::from_be_bytes([ip[8], ip[9]]),
            u16::from_be_bytes([ip[10], ip[11]]),
            u16::from_be_bytes([ip[12], ip[13]]),
            u16::from_be_bytes([ip[14], ip[15]]),
        ];
        format!(
            "{:X}:{:X}:{:X}:{:X}:{:X}:{:X}:{:X}:{:X}",
            segments[0],
            segments[1],
            segments[2],
            segments[3],
            segments[4],
            segments[5],
            segments[6],
            segments[7]
        )
    } else {
        format!("{ip:?}")
    }
}

fn decode_name(name: &x509_parser::x509::X509Name<'_>) -> DistinguishedName {
    name.iter()
        .map(|rdn| {
            rdn.iter()
                .map(|attribute| {
                    let oid = attribute.attr_type().to_id_string();
                    let value = attribute.attr_value().as_str().map_or_else(
                        |_| match core::str::from_utf8(attribute.attr_value().data) {
                            Ok(s) => s.to_string(),
                            Err(_) => {
                                String::from_utf8_lossy(attribute.attr_value().data).into_owned()
                            }
                        },
                        str::to_string,
                    );
                    (oid_to_attribute_name(&oid).to_string(), value)
                })
                .collect()
        })
        .collect()
}

fn format_certificate_time(value: &x509_parser::time::ASN1Time) -> String {
    let date = value.to_datetime();
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    format!(
        "{} {:>2} {:02}:{:02}:{:02} {:04} GMT",
        MONTHS[u8::from(date.month()) as usize - 1],
        date.day(),
        date.hour(),
        date.minute(),
        date.second(),
        date.year(),
    )
}

/// Decode a DER certificate into the fields `_ssl` publishes.
pub fn decode_certificate(der: &[u8]) -> Result<DecodedCertificate, String> {
    use x509_parser::extensions::{DistributionPointName, GeneralName, ParsedExtension};
    use x509_parser::oid_registry::{
        OID_PKIX_AUTHORITY_INFO_ACCESS, OID_X509_EXT_CRL_DISTRIBUTION_POINTS,
    };

    let (_, cert) = x509_parser::parse_x509_certificate(der)
        .map_err(|error| format!("Failed to parse certificate: {error}"))?;

    let mut serial = cert.serial.to_str_radix(16).to_uppercase();
    if serial.len() % 2 == 1 {
        serial.insert(0, '0');
    }

    let mut decoded = DecodedCertificate {
        issuer: decode_name(cert.issuer()),
        subject: decode_name(cert.subject()),
        not_after: format_certificate_time(&cert.validity().not_after),
        not_before: format_certificate_time(&cert.validity().not_before),
        serial_number: serial,
        version: cert.version().0 as i32 + 1,
        ocsp: Vec::new(),
        ca_issuers: Vec::new(),
        crl_distribution_points: Vec::new(),
        subject_alt_names: Vec::new(),
    };

    if let Ok(extensions) = cert.tbs_certificate.extensions_map() {
        if let Some(extension) = extensions.get(&OID_PKIX_AUTHORITY_INFO_ACCESS)
            && let ParsedExtension::AuthorityInfoAccess(access) = extension.parsed_extension()
        {
            for description in &access.accessdescs {
                if let GeneralName::URI(uri) = &description.access_location {
                    match description.access_method.to_id_string().as_str() {
                        "1.3.6.1.5.5.7.48.1" => decoded.ocsp.push((*uri).to_string()),
                        "1.3.6.1.5.5.7.48.2" => decoded.ca_issuers.push((*uri).to_string()),
                        _ => {}
                    }
                }
            }
        }
        if let Some(extension) = extensions.get(&OID_X509_EXT_CRL_DISTRIBUTION_POINTS)
            && let ParsedExtension::CRLDistributionPoints(points) = extension.parsed_extension()
        {
            for point in &points.points {
                if let Some(DistributionPointName::FullName(names)) = &point.distribution_point {
                    for name in names {
                        if let GeneralName::URI(uri) = name {
                            decoded.crl_distribution_points.push((*uri).to_string());
                        }
                    }
                }
            }
        }
    }

    if let Ok(Some(extension)) = cert.subject_alternative_name() {
        for name in &extension.value.general_names {
            let entry = match name {
                GeneralName::DNSName(value) => SubjectAlternativeName {
                    kind: "DNS",
                    value: (*value).to_string(),
                    directory_name: Vec::new(),
                },
                GeneralName::IPAddress(value) => SubjectAlternativeName {
                    kind: "IP Address",
                    value: format_ip_address(value),
                    directory_name: Vec::new(),
                },
                GeneralName::RFC822Name(value) => SubjectAlternativeName {
                    kind: "email",
                    value: (*value).to_string(),
                    directory_name: Vec::new(),
                },
                GeneralName::URI(value) => SubjectAlternativeName {
                    kind: "URI",
                    value: (*value).to_string(),
                    directory_name: Vec::new(),
                },
                GeneralName::OtherName(_, _) => SubjectAlternativeName {
                    kind: "othername",
                    value: "<unsupported>".to_string(),
                    directory_name: Vec::new(),
                },
                GeneralName::DirectoryName(value) => SubjectAlternativeName {
                    kind: "DirName",
                    value: String::new(),
                    directory_name: decode_name(value),
                },
                GeneralName::RegisteredID(value) => SubjectAlternativeName {
                    kind: "Registered ID",
                    value: value.to_id_string(),
                    directory_name: Vec::new(),
                },
                _ => continue,
            };
            decoded.subject_alt_names.push(entry);
        }
    }

    Ok(decoded)
}

/// True when Basic Constraints mark the certificate as a CA, or when a
/// self-issued X.509v1 certificate has no extensions.
#[must_use]
pub fn is_ca_certificate(cert_der: &[u8]) -> bool {
    let Ok((_, cert)) = X509Certificate::from_der(cert_der) else {
        return false;
    };

    if let Ok(Some(ext)) = cert.basic_constraints() {
        return ext.value.ca;
    }

    cert.version().0 == 0 && cert.subject() == cert.issuer()
}

/// Parse PEM or a single DER certificate into DER bodies.
pub fn read_certificates(data: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    if data
        .windows(b"-----BEGIN ".len())
        .any(|w| w == b"-----BEGIN ")
    {
        let mut input = data;
        rustls_pemfile::certs(&mut input)
            .map(|cert| cert.map(|der| der.as_ref().to_vec()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Failed to parse PEM certificate: {error}"))
    } else {
        x509_parser::parse_x509_certificate(data)
            .map_err(|error| format!("Failed to parse certificate: {error}"))?;
        Ok(vec![data.to_vec()])
    }
}
