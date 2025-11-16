pub(super) use ssl_cert::{PySSLCertificate, cert_to_certificate, cert_to_py, obj2txt};

// Certificate type for SSL module

#[pymodule(sub)]
pub(crate) mod ssl_cert {
    use crate::{
        common::ascii,
        vm::{
            PyObjectRef, PyPayload, PyResult, VirtualMachine,
            convert::{ToPyException, ToPyObject},
            function::{FsPath, OptionalArg},
        },
    };
    use foreign_types_shared::ForeignTypeRef;
    use openssl::{
        asn1::Asn1ObjectRef,
        x509::{self, X509, X509Ref},
    };
    use openssl_sys as sys;
    use std::fmt;

    // Import constants and error converter from _ssl module
    use crate::openssl::_ssl::{ENCODING_DER, ENCODING_PEM, convert_openssl_error};

    pub(crate) fn obj2txt(obj: &Asn1ObjectRef, no_name: bool) -> Option<String> {
        let no_name = i32::from(no_name);
        let ptr = obj.as_ptr();
        let b = unsafe {
            let buflen = sys::OBJ_obj2txt(std::ptr::null_mut(), 0, ptr, no_name);
            assert!(buflen >= 0);
            if buflen == 0 {
                return None;
            }
            let buflen = buflen as usize;
            let mut buf = Vec::<u8>::with_capacity(buflen + 1);
            let ret = sys::OBJ_obj2txt(
                buf.as_mut_ptr() as *mut libc::c_char,
                buf.capacity() as _,
                ptr,
                no_name,
            );
            assert!(ret >= 0);
            // SAFETY: OBJ_obj2txt initialized the buffer successfully
            buf.set_len(buflen);
            buf
        };
        let s = String::from_utf8(b)
            .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
        Some(s)
    }

    #[pyattr]
    #[pyclass(module = "ssl", name = "Certificate")]
    #[derive(PyPayload)]
    pub(crate) struct PySSLCertificate {
        cert: X509,
    }

    impl fmt::Debug for PySSLCertificate {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.pad("Certificate")
        }
    }

    #[pyclass]
    impl PySSLCertificate {
        #[pymethod]
        fn public_bytes(
            &self,
            format: OptionalArg<i32>,
            vm: &VirtualMachine,
        ) -> PyResult<PyObjectRef> {
            let format = format.unwrap_or(ENCODING_PEM);

            match format {
                ENCODING_DER => {
                    // DER encoding
                    let der = self
                        .cert
                        .to_der()
                        .map_err(|e| convert_openssl_error(vm, e))?;
                    Ok(vm.ctx.new_bytes(der).into())
                }
                ENCODING_PEM => {
                    // PEM encoding
                    let pem = self
                        .cert
                        .to_pem()
                        .map_err(|e| convert_openssl_error(vm, e))?;
                    Ok(vm.ctx.new_bytes(pem).into())
                }
                _ => Err(vm.new_value_error("Unsupported format")),
            }
        }

        #[pymethod]
        fn get_info(&self, vm: &VirtualMachine) -> PyResult {
            cert_to_dict(vm, &self.cert)
        }
    }

    fn name_to_py(vm: &VirtualMachine, name: &x509::X509NameRef) -> PyResult {
        let list = name
            .entries()
            .map(|entry| {
                let txt = obj2txt(entry.object(), false).to_pyobject(vm);
                let asn1_str = entry.data();
                let data_bytes = asn1_str.as_slice();
                let data = match std::str::from_utf8(data_bytes) {
                    Ok(s) => vm.ctx.new_str(s.to_owned()),
                    Err(_) => vm
                        .ctx
                        .new_str(String::from_utf8_lossy(data_bytes).into_owned()),
                };
                Ok(vm.new_tuple(((txt, data),)).into())
            })
            .collect::<Result<_, _>>()?;
        Ok(vm.ctx.new_tuple(list).into())
    }

    // Helper to convert X509 to dict (for getpeercert with binary=False)
    fn cert_to_dict(vm: &VirtualMachine, cert: &X509Ref) -> PyResult {
        let dict = vm.ctx.new_dict();

        dict.set_item("subject", name_to_py(vm, cert.subject_name())?, vm)?;
        dict.set_item("issuer", name_to_py(vm, cert.issuer_name())?, vm)?;
        // X.509 version: OpenSSL uses 0-based (0=v1, 1=v2, 2=v3) but Python uses 1-based (1=v1, 2=v2, 3=v3)
        dict.set_item("version", vm.new_pyobj(cert.version() + 1), vm)?;

        let serial_num = cert
            .serial_number()
            .to_bn()
            .and_then(|bn| bn.to_hex_str())
            .map_err(|e| convert_openssl_error(vm, e))?;
        dict.set_item(
            "serialNumber",
            vm.ctx.new_str(serial_num.to_owned()).into(),
            vm,
        )?;

        dict.set_item(
            "notBefore",
            vm.ctx.new_str(cert.not_before().to_string()).into(),
            vm,
        )?;
        dict.set_item(
            "notAfter",
            vm.ctx.new_str(cert.not_after().to_string()).into(),
            vm,
        )?;

        if let Some(names) = cert.subject_alt_names() {
            let san: Vec<PyObjectRef> = names
                .iter()
                .map(|gen_name| {
                    if let Some(email) = gen_name.email() {
                        vm.new_tuple((ascii!("email"), email)).into()
                    } else if let Some(dnsname) = gen_name.dnsname() {
                        vm.new_tuple((ascii!("DNS"), dnsname)).into()
                    } else if let Some(ip) = gen_name.ipaddress() {
                        // Parse IP address properly (IPv4 or IPv6)
                        let ip_str = if ip.len() == 4 {
                            // IPv4
                            format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
                        } else if ip.len() == 16 {
                            // IPv6 - format with all zeros visible (not compressed)
                            let ip_addr = std::net::Ipv6Addr::from(ip[0..16]);
                            let s = ip_addr.segments();
                            format!(
                                "{:X}:{:X}:{:X}:{:X}:{:X}:{:X}:{:X}:{:X}",
                                s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]
                            )
                        } else {
                            // Fallback for unexpected length
                            String::from_utf8_lossy(ip).into_owned()
                        };
                        vm.new_tuple((ascii!("IP Address"), ip_str)).into()
                    } else if let Some(uri) = gen_name.uri() {
                        vm.new_tuple((ascii!("URI"), uri)).into()
                    } else {
                        // Handle DirName, Registered ID, and othername
                        // Check if this is a directory name
                        if let Some(dirname) = gen_name.directory_name()
                            && let Ok(py_name) = name_to_py(vm, dirname)
                        {
                            return vm.new_tuple((ascii!("DirName"), py_name)).into();
                        }

                        // TODO: Handle Registered ID (GEN_RID)
                        // CPython implementation uses i2t_ASN1_OBJECT to convert OID
                        // This requires accessing GENERAL_NAME union which is complex in Rust
                        // For now, we return <unsupported> for unhandled types

                        // For othername and other unsupported types
                        vm.new_tuple((ascii!("othername"), ascii!("<unsupported>")))
                            .into()
                    }
                })
                .collect();
            dict.set_item("subjectAltName", vm.ctx.new_tuple(san).into(), vm)?;
        };

        Ok(dict.into())
    }

    // Helper to create Certificate object from X509
    pub(crate) fn cert_to_certificate(vm: &VirtualMachine, cert: X509) -> PyResult {
        Ok(PySSLCertificate { cert }.into_ref(&vm.ctx).into())
    }

    // For getpeercert() - returns bytes or dict depending on binary flag
    pub(crate) fn cert_to_py(vm: &VirtualMachine, cert: &X509Ref, binary: bool) -> PyResult {
        if binary {
            let b = cert.to_der().map_err(|e| convert_openssl_error(vm, e))?;
            Ok(vm.ctx.new_bytes(b).into())
        } else {
            cert_to_dict(vm, cert)
        }
    }

    #[pyfunction]
    pub(crate) fn _test_decode_cert(path: FsPath, vm: &VirtualMachine) -> PyResult {
        let path = path.to_path_buf(vm)?;
        let pem = std::fs::read(path).map_err(|e| e.to_pyexception(vm))?;
        let x509 = X509::from_pem(&pem).map_err(|e| convert_openssl_error(vm, e))?;
        cert_to_py(vm, &x509, false)
    }
}
