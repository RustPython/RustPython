//! Python adapters over the host rustls certificate engine.

use rustpython_host_env::ssl::cert as ssl_cert;
use rustpython_vm::{PyObjectRef, PyResult, VirtualMachine};

pub(super) use rustpython_host_env::ssl::cert::is_ca_certificate;
pub(super) use rustpython_host_env::ssl::verify::{
    CertLoader, CertStats, build_verified_chain, load_cert_chain_from_file, validate_cert_key_match,
};

/// Convert DER-encoded certificate to Python dict.
///
/// Includes OCSP, caIssuers, and crlDistributionPoints when present, and uses
/// the field order: issuer, notAfter, notBefore, serialNumber, subject, version
pub(super) fn cert_der_to_dict_helper(
    vm: &VirtualMachine,
    cert_der: &[u8],
) -> PyResult<PyObjectRef> {
    let decoded = ssl_cert::decode_certificate(cert_der).map_err(|e| vm.new_value_error(e))?;

    let name_to_tuple = |name: &ssl_cert::DistinguishedName| -> PyObjectRef {
        let mut entries = Vec::new();
        for rdn in name {
            for (key, value) in rdn {
                let entry =
                    vm.new_tuple((vm.ctx.new_str(key.as_str()), vm.ctx.new_str(value.as_str())));
                entries.push(vm.new_tuple((entry,)).into());
            }
        }
        vm.ctx.new_tuple(entries).into()
    };

    let dict = vm.ctx.new_dict();
    dict.set_item("issuer", name_to_tuple(&decoded.issuer), vm)?;
    dict.set_item("notAfter", vm.ctx.new_str(decoded.not_after).into(), vm)?;
    dict.set_item("notBefore", vm.ctx.new_str(decoded.not_before).into(), vm)?;
    dict.set_item(
        "serialNumber",
        vm.ctx.new_str(decoded.serial_number).into(),
        vm,
    )?;
    dict.set_item("subject", name_to_tuple(&decoded.subject), vm)?;
    dict.set_item("version", vm.ctx.new_int(decoded.version).into(), vm)?;

    if !decoded.ocsp.is_empty() {
        let urls = decoded
            .ocsp
            .into_iter()
            .map(|url| vm.ctx.new_str(url).into())
            .collect();
        dict.set_item("OCSP", vm.ctx.new_tuple(urls).into(), vm)?;
    }
    if !decoded.ca_issuers.is_empty() {
        let urls = decoded
            .ca_issuers
            .into_iter()
            .map(|url| vm.ctx.new_str(url).into())
            .collect();
        dict.set_item("caIssuers", vm.ctx.new_tuple(urls).into(), vm)?;
    }
    if !decoded.crl_distribution_points.is_empty() {
        let urls = decoded
            .crl_distribution_points
            .into_iter()
            .map(|url| vm.ctx.new_str(url).into())
            .collect();
        dict.set_item("crlDistributionPoints", vm.ctx.new_tuple(urls).into(), vm)?;
    }

    if !decoded.subject_alt_names.is_empty() {
        let mut san_entries = Vec::new();
        for name in decoded.subject_alt_names {
            if name.kind == "DirName" {
                san_entries.push(
                    vm.new_tuple(("DirName", name_to_tuple(&name.directory_name)))
                        .into(),
                );
            } else {
                san_entries.push(vm.new_tuple((name.kind, name.value)).into());
            }
        }
        dict.set_item("subjectAltName", vm.ctx.new_tuple(san_entries).into(), vm)?;
    }

    Ok(dict.into())
}
