// spell-checker:disable
#![allow(non_snake_case)]

pub(crate) use _wmi::module_def;

// COM/WMI FFI declarations (not inside pymodule to avoid macro issues)
mod wmi_ffi {
    #![allow(unsafe_op_in_unsafe_fn)]
    use core::ffi::c_void;

    pub type HRESULT = i32;

    #[repr(C)]
    pub struct GUID {
        pub data1: u32,
        pub data2: u16,
        pub data3: u16,
        pub data4: [u8; 8],
    }

    // Opaque VARIANT type (24 bytes covers both 32-bit and 64-bit)
    #[repr(C, align(8))]
    pub struct VARIANT([u64; 3]);

    impl VARIANT {
        pub fn zeroed() -> Self {
            VARIANT([0u64; 3])
        }
    }

    // CLSID_WbemLocator = {4590F811-1D3A-11D0-891F-00AA004B2E24}
    pub const CLSID_WBEM_LOCATOR: GUID = GUID {
        data1: 0x4590F811,
        data2: 0x1D3A,
        data3: 0x11D0,
        data4: [0x89, 0x1F, 0x00, 0xAA, 0x00, 0x4B, 0x2E, 0x24],
    };

    // IID_IWbemLocator = {DC12A687-737F-11CF-884D-00AA004B2E24}
    pub const IID_IWBEM_LOCATOR: GUID = GUID {
        data1: 0xDC12A687,
        data2: 0x737F,
        data3: 0x11CF,
        data4: [0x88, 0x4D, 0x00, 0xAA, 0x00, 0x4B, 0x2E, 0x24],
    };

    // COM constants
    pub const COINIT_APARTMENTTHREADED: u32 = 0x2;
    pub const CLSCTX_INPROC_SERVER: u32 = 0x1;
    pub const RPC_C_AUTHN_LEVEL_DEFAULT: u32 = 0;
    pub const RPC_C_IMP_LEVEL_IMPERSONATE: u32 = 3;
    pub const RPC_C_AUTHN_LEVEL_CALL: u32 = 3;
    pub const RPC_C_AUTHN_WINNT: u32 = 10;
    pub const RPC_C_AUTHZ_NONE: u32 = 0;
    pub const EOAC_NONE: u32 = 0;
    pub const RPC_E_TOO_LATE: HRESULT = 0x80010119_u32 as i32;

    // WMI constants
    pub const WBEM_FLAG_FORWARD_ONLY: i32 = 0x20;
    pub const WBEM_FLAG_RETURN_IMMEDIATELY: i32 = 0x10;
    pub const WBEM_S_FALSE: HRESULT = 1;
    pub const WBEM_S_NO_MORE_DATA: HRESULT = 0x40005;
    pub const WBEM_INFINITE: i32 = -1;
    pub const WBEM_FLAVOR_MASK_ORIGIN: i32 = 0x60;
    pub const WBEM_FLAVOR_ORIGIN_SYSTEM: i32 = 0x40;

    #[link(name = "ole32")]
    unsafe extern "system" {
        pub fn CoInitializeEx(pvReserved: *mut c_void, dwCoInit: u32) -> HRESULT;
        pub fn CoUninitialize();
        pub fn CoInitializeSecurity(
            pSecDesc: *const c_void,
            cAuthSvc: i32,
            asAuthSvc: *const c_void,
            pReserved1: *const c_void,
            dwAuthnLevel: u32,
            dwImpLevel: u32,
            pAuthList: *const c_void,
            dwCapabilities: u32,
            pReserved3: *const c_void,
        ) -> HRESULT;
        pub fn CoCreateInstance(
            rclsid: *const GUID,
            pUnkOuter: *mut c_void,
            dwClsContext: u32,
            riid: *const GUID,
            ppv: *mut *mut c_void,
        ) -> HRESULT;
        pub fn CoSetProxyBlanket(
            pProxy: *mut c_void,
            dwAuthnSvc: u32,
            dwAuthzSvc: u32,
            pServerPrincName: *const u16,
            dwAuthnLevel: u32,
            dwImpLevel: u32,
            pAuthInfo: *const c_void,
            dwCapabilities: u32,
        ) -> HRESULT;
    }

    #[link(name = "oleaut32")]
    unsafe extern "system" {
        pub fn SysAllocString(psz: *const u16) -> *mut u16;
        pub fn SysFreeString(bstrString: *mut u16);
        pub fn VariantClear(pvarg: *mut VARIANT) -> HRESULT;
    }

    #[link(name = "propsys")]
    unsafe extern "system" {
        pub fn VariantToString(varIn: *const VARIANT, pszBuf: *mut u16, cchBuf: u32) -> HRESULT;
    }

    /// Release a COM object (IUnknown::Release, vtable index 2)
    pub unsafe fn com_release(this: *mut c_void) {
        if !this.is_null() {
            let vtable = *(this as *const *const usize);
            let release: unsafe extern "system" fn(*mut c_void) -> u32 =
                core::mem::transmute(*vtable.add(2));
            release(this);
        }
    }

    /// IWbemLocator::ConnectServer (vtable index 3)
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn locator_connect_server(
        this: *mut c_void,
        network_resource: *const u16,
        user: *const u16,
        password: *const u16,
        locale: *const u16,
        security_flags: i32,
        authority: *const u16,
        ctx: *mut c_void,
        services: *mut *mut c_void,
    ) -> HRESULT {
        let vtable = *(this as *const *const usize);
        let method: unsafe extern "system" fn(
            *mut c_void,
            *const u16,
            *const u16,
            *const u16,
            *const u16,
            i32,
            *const u16,
            *mut c_void,
            *mut *mut c_void,
        ) -> HRESULT = core::mem::transmute(*vtable.add(3));
        method(
            this,
            network_resource,
            user,
            password,
            locale,
            security_flags,
            authority,
            ctx,
            services,
        )
    }

    /// IWbemServices::ExecQuery (vtable index 20)
    pub unsafe fn services_exec_query(
        this: *mut c_void,
        query_language: *const u16,
        query: *const u16,
        flags: i32,
        ctx: *mut c_void,
        enumerator: *mut *mut c_void,
    ) -> HRESULT {
        let vtable = *(this as *const *const usize);
        let method: unsafe extern "system" fn(
            *mut c_void,
            *const u16,
            *const u16,
            i32,
            *mut c_void,
            *mut *mut c_void,
        ) -> HRESULT = core::mem::transmute(*vtable.add(20));
        method(this, query_language, query, flags, ctx, enumerator)
    }

    /// IEnumWbemClassObject::Next (vtable index 4)
    pub unsafe fn enum_next(
        this: *mut c_void,
        timeout: i32,
        count: u32,
        objects: *mut *mut c_void,
        returned: *mut u32,
    ) -> HRESULT {
        let vtable = *(this as *const *const usize);
        let method: unsafe extern "system" fn(
            *mut c_void,
            i32,
            u32,
            *mut *mut c_void,
            *mut u32,
        ) -> HRESULT = core::mem::transmute(*vtable.add(4));
        method(this, timeout, count, objects, returned)
    }

    /// IWbemClassObject::BeginEnumeration (vtable index 8)
    pub unsafe fn object_begin_enumeration(this: *mut c_void, enum_flags: i32) -> HRESULT {
        let vtable = *(this as *const *const usize);
        let method: unsafe extern "system" fn(*mut c_void, i32) -> HRESULT =
            core::mem::transmute(*vtable.add(8));
        method(this, enum_flags)
    }

    /// IWbemClassObject::Next (vtable index 9)
    pub unsafe fn object_next(
        this: *mut c_void,
        flags: i32,
        name: *mut *mut u16,
        val: *mut VARIANT,
        cim_type: *mut i32,
        flavor: *mut i32,
    ) -> HRESULT {
        let vtable = *(this as *const *const usize);
        let method: unsafe extern "system" fn(
            *mut c_void,
            i32,
            *mut *mut u16,
            *mut VARIANT,
            *mut i32,
            *mut i32,
        ) -> HRESULT = core::mem::transmute(*vtable.add(9));
        method(this, flags, name, val, cim_type, flavor)
    }

    /// IWbemClassObject::EndEnumeration (vtable index 10)
    pub unsafe fn object_end_enumeration(this: *mut c_void) -> HRESULT {
        let vtable = *(this as *const *const usize);
        let method: unsafe extern "system" fn(*mut c_void) -> HRESULT =
            core::mem::transmute(*vtable.add(10));
        method(this)
    }
}

#[pymodule]
mod _wmi {
    use super::wmi_ffi::*;
    use crate::builtins::PyStrRef;
    use crate::convert::ToPyException;
    use crate::{PyResult, VirtualMachine};
    use core::ffi::c_void;
    use core::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_BROKEN_PIPE, ERROR_MORE_DATA, ERROR_NOT_ENOUGH_MEMORY, GetLastError,
        HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::Storage::FileSystem::{ReadFile, WriteFile};
    use windows_sys::Win32::System::Pipes::CreatePipe;
    use windows_sys::Win32::System::Threading::{
        CreateEventW, CreateThread, GetExitCodeThread, SetEvent, WaitForSingleObject,
    };

    const BUFFER_SIZE: usize = 8192;

    fn hresult_from_win32(err: u32) -> HRESULT {
        if err == 0 {
            0
        } else {
            ((err & 0xFFFF) | 0x80070000) as HRESULT
        }
    }

    fn succeeded(hr: HRESULT) -> bool {
        hr >= 0
    }

    fn failed(hr: HRESULT) -> bool {
        hr < 0
    }

    fn wide_str(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(core::iter::once(0)).collect()
    }

    unsafe fn wcslen(s: *const u16) -> usize {
        unsafe {
            let mut len = 0;
            while *s.add(len) != 0 {
                len += 1;
            }
            len
        }
    }

    unsafe fn wait_event(event: HANDLE, timeout: u32) -> u32 {
        unsafe {
            match WaitForSingleObject(event, timeout) {
                WAIT_OBJECT_0 => 0,
                WAIT_TIMEOUT => WAIT_TIMEOUT,
                _ => GetLastError(),
            }
        }
    }

    struct QueryThreadData {
        query: Vec<u16>,
        write_pipe: HANDLE,
        init_event: HANDLE,
        connect_event: HANDLE,
    }

    // SAFETY: QueryThreadData contains HANDLEs (isize) which are safe to send across threads
    unsafe impl Send for QueryThreadData {}

    unsafe extern "system" fn query_thread(param: *mut c_void) -> u32 {
        unsafe { query_thread_impl(param) }
    }

    unsafe fn query_thread_impl(param: *mut c_void) -> u32 {
        unsafe {
            let data = Box::from_raw(param as *mut QueryThreadData);
            let write_pipe = data.write_pipe;
            let init_event = data.init_event;
            let connect_event = data.connect_event;

            let mut locator: *mut c_void = null_mut();
            let mut services: *mut c_void = null_mut();
            let mut enumerator: *mut c_void = null_mut();
            let mut hr: HRESULT = 0;

            // gh-125315: Copy the query string first
            let bstr_query = SysAllocString(data.query.as_ptr());
            if bstr_query.is_null() {
                hr = hresult_from_win32(ERROR_NOT_ENOUGH_MEMORY);
            }

            drop(data);

            if succeeded(hr) {
                hr = CoInitializeEx(null_mut(), COINIT_APARTMENTTHREADED);
            }

            if failed(hr) {
                CloseHandle(write_pipe);
                if !bstr_query.is_null() {
                    SysFreeString(bstr_query);
                }
                return hr as u32;
            }

            hr = CoInitializeSecurity(
                null(),
                -1,
                null(),
                null(),
                RPC_C_AUTHN_LEVEL_DEFAULT,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                null(),
                EOAC_NONE,
                null(),
            );
            // gh-96684: CoInitializeSecurity will fail if another part of the app has
            // already called it.
            if hr == RPC_E_TOO_LATE {
                hr = 0;
            }

            if succeeded(hr) {
                hr = CoCreateInstance(
                    &CLSID_WBEM_LOCATOR,
                    null_mut(),
                    CLSCTX_INPROC_SERVER,
                    &IID_IWBEM_LOCATOR,
                    &mut locator,
                );
            }
            if succeeded(hr) && SetEvent(init_event) == 0 {
                hr = hresult_from_win32(GetLastError());
            }

            if succeeded(hr) {
                let root_cimv2 = wide_str("ROOT\\CIMV2");
                let bstr_root = SysAllocString(root_cimv2.as_ptr());
                hr = locator_connect_server(
                    locator,
                    bstr_root,
                    null(),
                    null(),
                    null(),
                    0,
                    null(),
                    null_mut(),
                    &mut services,
                );
                if !bstr_root.is_null() {
                    SysFreeString(bstr_root);
                }
            }
            if succeeded(hr) && SetEvent(connect_event) == 0 {
                hr = hresult_from_win32(GetLastError());
            }

            if succeeded(hr) {
                hr = CoSetProxyBlanket(
                    services,
                    RPC_C_AUTHN_WINNT,
                    RPC_C_AUTHZ_NONE,
                    null(),
                    RPC_C_AUTHN_LEVEL_CALL,
                    RPC_C_IMP_LEVEL_IMPERSONATE,
                    null(),
                    EOAC_NONE,
                );
            }
            if succeeded(hr) {
                let wql = wide_str("WQL");
                let bstr_wql = SysAllocString(wql.as_ptr());
                hr = services_exec_query(
                    services,
                    bstr_wql,
                    bstr_query,
                    WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
                    null_mut(),
                    &mut enumerator,
                );
                if !bstr_wql.is_null() {
                    SysFreeString(bstr_wql);
                }
            }

            // Enumerate results and write to pipe
            let mut value: *mut c_void;
            let mut start_of_enum = true;
            let null_sep: u16 = 0;
            let eq_sign: u16 = b'=' as u16;

            while succeeded(hr) {
                let mut got: u32 = 0;
                let mut written: u32 = 0;
                value = null_mut();
                hr = enum_next(enumerator, WBEM_INFINITE, 1, &mut value, &mut got);

                if hr == WBEM_S_FALSE {
                    hr = 0;
                    break;
                }
                if failed(hr) || got != 1 || value.is_null() {
                    continue;
                }

                if !start_of_enum
                    && WriteFile(
                        write_pipe,
                        &null_sep as *const u16 as *const _,
                        2,
                        &mut written,
                        null_mut(),
                    ) == 0
                {
                    hr = hresult_from_win32(GetLastError());
                    com_release(value);
                    break;
                }
                start_of_enum = false;

                hr = object_begin_enumeration(value, 0);
                if failed(hr) {
                    com_release(value);
                    break;
                }

                while succeeded(hr) {
                    let mut prop_name: *mut u16 = null_mut();
                    let mut prop_value = VARIANT::zeroed();
                    let mut flavor: i32 = 0;

                    hr = object_next(
                        value,
                        0,
                        &mut prop_name,
                        &mut prop_value,
                        null_mut(),
                        &mut flavor,
                    );

                    if hr == WBEM_S_NO_MORE_DATA {
                        hr = 0;
                        break;
                    }

                    if succeeded(hr)
                        && (flavor & WBEM_FLAVOR_MASK_ORIGIN) != WBEM_FLAVOR_ORIGIN_SYSTEM
                    {
                        let mut prop_str = [0u16; BUFFER_SIZE];
                        hr =
                            VariantToString(&prop_value, prop_str.as_mut_ptr(), BUFFER_SIZE as u32);

                        if succeeded(hr) {
                            let cb_str1 = (wcslen(prop_name) * 2) as u32;
                            let cb_str2 = (wcslen(prop_str.as_ptr()) * 2) as u32;

                            if WriteFile(
                                write_pipe,
                                prop_name as *const _,
                                cb_str1,
                                &mut written,
                                null_mut(),
                            ) == 0
                                || WriteFile(
                                    write_pipe,
                                    &eq_sign as *const u16 as *const _,
                                    2,
                                    &mut written,
                                    null_mut(),
                                ) == 0
                                || WriteFile(
                                    write_pipe,
                                    prop_str.as_ptr() as *const _,
                                    cb_str2,
                                    &mut written,
                                    null_mut(),
                                ) == 0
                                || WriteFile(
                                    write_pipe,
                                    &null_sep as *const u16 as *const _,
                                    2,
                                    &mut written,
                                    null_mut(),
                                ) == 0
                            {
                                hr = hresult_from_win32(GetLastError());
                            }
                        }

                        VariantClear(&mut prop_value);
                        SysFreeString(prop_name);
                    }
                }

                object_end_enumeration(value);
                com_release(value);
            }

            // Cleanup
            if !bstr_query.is_null() {
                SysFreeString(bstr_query);
            }
            if !enumerator.is_null() {
                com_release(enumerator);
            }
            if !services.is_null() {
                com_release(services);
            }
            if !locator.is_null() {
                com_release(locator);
            }
            CoUninitialize();
            CloseHandle(write_pipe);

            hr as u32
        }
    }

    /// Runs a WMI query against the local machine.
    ///
    /// This returns a single string with 'name=value' pairs in a flat array separated
    /// by null characters.
    #[pyfunction]
    fn exec_query(query: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
        let query_str = query.as_str();

        if !query_str
            .get(..7)
            .is_some_and(|s| s.eq_ignore_ascii_case("select "))
        {
            return Err(vm.new_value_error("only SELECT queries are supported".to_owned()));
        }

        let query_wide = wide_str(query_str);

        let mut h_thread: HANDLE = null_mut();
        let mut err: u32 = 0;
        let mut buffer = [0u16; BUFFER_SIZE];
        let mut offset: u32 = 0;
        let mut bytes_read: u32 = 0;

        let mut read_pipe: HANDLE = null_mut();
        let mut write_pipe: HANDLE = null_mut();

        unsafe {
            let init_event = CreateEventW(null(), 1, 0, null());
            let connect_event = CreateEventW(null(), 1, 0, null());

            if init_event.is_null()
                || connect_event.is_null()
                || CreatePipe(&mut read_pipe, &mut write_pipe, null(), 0) == 0
            {
                err = GetLastError();
            } else {
                let thread_data = Box::new(QueryThreadData {
                    query: query_wide,
                    write_pipe,
                    init_event,
                    connect_event,
                });
                let thread_data_ptr = Box::into_raw(thread_data);

                h_thread = CreateThread(
                    null(),
                    0,
                    Some(query_thread),
                    thread_data_ptr as *const _ as *mut _,
                    0,
                    null_mut(),
                );

                if h_thread.is_null() {
                    err = GetLastError();
                    // Thread didn't start, so recover data and close write pipe
                    let data = Box::from_raw(thread_data_ptr);
                    CloseHandle(data.write_pipe);
                }
            }

            // gh-112278: Timeout for COM init and WMI connection
            if err == 0 {
                err = wait_event(init_event, 1000);
                if err == 0 {
                    err = wait_event(connect_event, 100);
                }
            }

            // Read results from pipe
            while err == 0 {
                let buf_ptr = (buffer.as_mut_ptr() as *mut u8).add(offset as usize);
                let buf_remaining = (BUFFER_SIZE * 2) as u32 - offset;

                if ReadFile(
                    read_pipe,
                    buf_ptr as *mut _,
                    buf_remaining,
                    &mut bytes_read,
                    null_mut(),
                ) != 0
                {
                    offset += bytes_read;
                    if offset >= (BUFFER_SIZE * 2) as u32 {
                        err = ERROR_MORE_DATA;
                    }
                } else {
                    err = GetLastError();
                }
            }

            if !read_pipe.is_null() {
                CloseHandle(read_pipe);
            }

            if !h_thread.is_null() {
                let thread_err: u32;
                match WaitForSingleObject(h_thread, 100) {
                    WAIT_OBJECT_0 => {
                        let mut exit_code: u32 = 0;
                        if GetExitCodeThread(h_thread, &mut exit_code) == 0 {
                            thread_err = GetLastError();
                        } else {
                            thread_err = exit_code;
                        }
                    }
                    WAIT_TIMEOUT => {
                        thread_err = WAIT_TIMEOUT;
                    }
                    _ => {
                        thread_err = GetLastError();
                    }
                }
                if err == 0 || err == ERROR_BROKEN_PIPE {
                    err = thread_err;
                }

                CloseHandle(h_thread);
            }

            CloseHandle(init_event);
            CloseHandle(connect_event);
        }

        if err == ERROR_MORE_DATA {
            return Err(vm.new_os_error(format!(
                "Query returns more than {} characters",
                BUFFER_SIZE,
            )));
        } else if err != 0 {
            return Err(std::io::Error::from_raw_os_error(err as i32).to_pyexception(vm));
        }

        if offset == 0 {
            return Ok(String::new());
        }

        let char_count = (offset as usize) / 2 - 1;
        Ok(String::from_utf16_lossy(&buffer[..char_count]))
    }
}
