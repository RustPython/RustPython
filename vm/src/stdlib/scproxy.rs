pub(crate) use _scproxy::make_module;

#[pymodule]
mod _scproxy {
    // straight-forward port of Modules/_scproxy.c

    use crate::{
        builtins::{PyDictRef, PyStr},
        IntoPyObject, ItemProtocol, PyResult, VirtualMachine,
    };
    use system_configuration::core_foundation::{
        array::CFArray,
        base::{CFType, FromVoid, TCFType},
        dictionary::CFDictionary,
        number::CFNumber,
        string::{CFString, CFStringRef},
    };
    use system_configuration::sys::{
        dynamic_store_copy_specific::SCDynamicStoreCopyProxies, schema_definitions::*,
    };

    fn proxy_dict() -> Option<CFDictionary<CFString, CFType>> {
        // Py_BEGIN_ALLOW_THREADS
        let proxy_dict = unsafe { SCDynamicStoreCopyProxies(std::ptr::null()) };
        // Py_END_ALLOW_THREADS
        if proxy_dict.is_null() {
            None
        } else {
            Some(unsafe { CFDictionary::wrap_under_create_rule(proxy_dict) })
        }
    }

    #[pyfunction]
    fn _get_proxy_settings(vm: &VirtualMachine) -> PyResult<Option<PyDictRef>> {
        let proxy_dict = if let Some(p) = proxy_dict() {
            p
        } else {
            return Ok(None);
        };

        let result = vm.ctx.new_dict();

        let v = 0
            != proxy_dict
                .find(unsafe { kSCPropNetProxiesExcludeSimpleHostnames })
                .and_then(|v| v.downcast::<CFNumber>())
                .and_then(|v| v.to_i32())
                .unwrap_or(0);
        result.set_item("exclude_simple", vm.ctx.new_bool(v), vm)?;

        if let Some(an_array) = proxy_dict
            .find(unsafe { kSCPropNetProxiesExceptionsList })
            .and_then(|v| v.downcast::<CFArray>())
        {
            let v = an_array
                .into_iter()
                .map(|s| {
                    unsafe { CFType::from_void(*s) }
                        .downcast::<CFString>()
                        .map(|s| {
                            let a_string: std::borrow::Cow<str> = (&s).into();
                            PyStr::from(a_string.into_owned())
                        })
                        .into_pyobject(vm)
                })
                .collect();
            result.set_item("exceptions", vm.ctx.new_tuple(v), vm)?;
        }

        Ok(Some(result))
    }

    #[pyfunction]
    fn _get_proxies(vm: &VirtualMachine) -> PyResult<Option<PyDictRef>> {
        let proxy_dict = if let Some(p) = proxy_dict() {
            p
        } else {
            return Ok(None);
        };

        let result = vm.ctx.new_dict();

        let set_proxy = |result: &PyDictRef,
                         proto: &str,
                         enabled_key: CFStringRef,
                         host_key: CFStringRef,
                         port_key: CFStringRef|
         -> PyResult<()> {
            let enabled = 0
                != proxy_dict
                    .find(enabled_key)
                    .and_then(|v| v.downcast::<CFNumber>())
                    .and_then(|v| v.to_i32())
                    .unwrap_or(0);
            if enabled {
                if let Some(host) = proxy_dict
                    .find(host_key)
                    .and_then(|v| v.downcast::<CFString>())
                {
                    let h = std::borrow::Cow::<str>::from(&host);
                    let v = if let Some(port) = proxy_dict
                        .find(port_key)
                        .and_then(|v| v.downcast::<CFNumber>())
                        .and_then(|v| v.to_i32())
                    {
                        format!("http://{}:{}", h, port)
                    } else {
                        format!("http://{}", h)
                    };
                    result.set_item(proto, vm.ctx.new_utf8_str(v), vm)?;
                }
            }
            Ok(())
        };

        unsafe {
            set_proxy(
                &result,
                "http",
                kSCPropNetProxiesHTTPEnable,
                kSCPropNetProxiesHTTPProxy,
                kSCPropNetProxiesHTTPPort,
            )?;
            set_proxy(
                &result,
                "https",
                kSCPropNetProxiesHTTPSEnable,
                kSCPropNetProxiesHTTPSProxy,
                kSCPropNetProxiesHTTPSPort,
            )?;
            set_proxy(
                &result,
                "ftp",
                kSCPropNetProxiesFTPEnable,
                kSCPropNetProxiesFTPProxy,
                kSCPropNetProxiesFTPPort,
            )?;
            set_proxy(
                &result,
                "gopher",
                kSCPropNetProxiesGopherEnable,
                kSCPropNetProxiesGopherProxy,
                kSCPropNetProxiesGopherPort,
            )?;
        }
        Ok(Some(result))
    }
}
