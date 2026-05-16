#![allow(
    clippy::upper_case_acronyms,
    reason = "These names mirror the Windows COM and ABI types they wrap."
)]
#![allow(non_snake_case)]
#![allow(unsafe_op_in_unsafe_fn)]

use core::ffi::c_void;
use core::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_BROKEN_PIPE, ERROR_MORE_DATA, ERROR_NOT_ENOUGH_MEMORY, GetLastError, HANDLE,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CreateEventW, CreateThread, GetExitCodeThread, SetEvent, WaitForSingleObject,
};

const BUFFER_SIZE: usize = 8192;

pub enum ExecQueryError {
    MoreData,
    Code(u32),
}

type HRESULT = i32;

#[repr(C)]
struct GUID {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

#[repr(C, align(8))]
struct VARIANT([u64; 3]);

impl VARIANT {
    fn zeroed() -> Self {
        Self([0u64; 3])
    }
}

const CLSID_WBEM_LOCATOR: GUID = GUID {
    data1: 0x4590F811,
    data2: 0x1D3A,
    data3: 0x11D0,
    data4: [0x89, 0x1F, 0x00, 0xAA, 0x00, 0x4B, 0x2E, 0x24],
};

const IID_IWBEM_LOCATOR: GUID = GUID {
    data1: 0xDC12A687,
    data2: 0x737F,
    data3: 0x11CF,
    data4: [0x88, 0x4D, 0x00, 0xAA, 0x00, 0x4B, 0x2E, 0x24],
};

const COINIT_APARTMENTTHREADED: u32 = 0x2;
const CLSCTX_INPROC_SERVER: u32 = 0x1;
const RPC_C_AUTHN_LEVEL_DEFAULT: u32 = 0;
const RPC_C_IMP_LEVEL_IMPERSONATE: u32 = 3;
const RPC_C_AUTHN_LEVEL_CALL: u32 = 3;
const RPC_C_AUTHN_WINNT: u32 = 10;
const RPC_C_AUTHZ_NONE: u32 = 0;
const EOAC_NONE: u32 = 0;
const RPC_E_TOO_LATE: HRESULT = 0x80010119_u32 as i32;
const WBEM_FLAG_FORWARD_ONLY: i32 = 0x20;
const WBEM_FLAG_RETURN_IMMEDIATELY: i32 = 0x10;
const WBEM_S_FALSE: HRESULT = 1;
const WBEM_S_NO_MORE_DATA: HRESULT = 0x40005;
const WBEM_INFINITE: i32 = -1;
const WBEM_FLAVOR_MASK_ORIGIN: i32 = 0x60;
const WBEM_FLAVOR_ORIGIN_SYSTEM: i32 = 0x40;

#[link(name = "ole32")]
unsafe extern "system" {
    fn CoInitializeEx(pvReserved: *mut c_void, dwCoInit: u32) -> HRESULT;
    fn CoUninitialize();
    fn CoInitializeSecurity(
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
    fn CoCreateInstance(
        rclsid: *const GUID,
        pUnkOuter: *mut c_void,
        dwClsContext: u32,
        riid: *const GUID,
        ppv: *mut *mut c_void,
    ) -> HRESULT;
    fn CoSetProxyBlanket(
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
    fn SysAllocString(psz: *const u16) -> *mut u16;
    fn SysFreeString(bstrString: *mut u16);
    fn VariantClear(pvarg: *mut VARIANT) -> HRESULT;
}

#[link(name = "propsys")]
unsafe extern "system" {
    fn VariantToString(varIn: *const VARIANT, pszBuf: *mut u16, cchBuf: u32) -> HRESULT;
}

unsafe fn com_release(this: *mut c_void) {
    if !this.is_null() {
        let vtable = *(this as *const *const usize);
        let release: unsafe extern "system" fn(*mut c_void) -> u32 =
            core::mem::transmute(*vtable.add(2));
        release(this);
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn locator_connect_server(
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

unsafe fn services_exec_query(
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

unsafe fn enum_next(
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

unsafe fn object_begin_enumeration(this: *mut c_void, enum_flags: i32) -> HRESULT {
    let vtable = *(this as *const *const usize);
    let method: unsafe extern "system" fn(*mut c_void, i32) -> HRESULT =
        core::mem::transmute(*vtable.add(8));
    method(this, enum_flags)
}

unsafe fn object_next(
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

unsafe fn object_end_enumeration(this: *mut c_void) -> HRESULT {
    let vtable = *(this as *const *const usize);
    let method: unsafe extern "system" fn(*mut c_void) -> HRESULT =
        core::mem::transmute(*vtable.add(10));
    method(this)
}

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
    let mut len = 0;
    while unsafe { *s.add(len) } != 0 {
        len += 1;
    }
    len
}

unsafe fn wait_event(event: HANDLE, timeout: u32) -> u32 {
    match unsafe { WaitForSingleObject(event, timeout) } {
        WAIT_OBJECT_0 => 0,
        WAIT_TIMEOUT => WAIT_TIMEOUT,
        _ => unsafe { GetLastError() },
    }
}

struct QueryThreadData {
    query: Vec<u16>,
    write_pipe: HANDLE,
    init_event: HANDLE,
    connect_event: HANDLE,
}

unsafe impl Send for QueryThreadData {}

unsafe extern "system" fn query_thread(param: *mut c_void) -> u32 {
    unsafe { query_thread_impl(param) }
}

unsafe fn query_thread_impl(param: *mut c_void) -> u32 {
    let data = unsafe { Box::from_raw(param as *mut QueryThreadData) };
    let write_pipe = data.write_pipe;
    let init_event = data.init_event;
    let connect_event = data.connect_event;

    let mut locator: *mut c_void = null_mut();
    let mut services: *mut c_void = null_mut();
    let mut enumerator: *mut c_void = null_mut();
    let mut hr: HRESULT = 0;

    let bstr_query = unsafe { SysAllocString(data.query.as_ptr()) };
    if bstr_query.is_null() {
        hr = hresult_from_win32(ERROR_NOT_ENOUGH_MEMORY);
    }

    drop(data);

    if succeeded(hr) {
        hr = unsafe { CoInitializeEx(null_mut(), COINIT_APARTMENTTHREADED) };
    }

    if failed(hr) {
        unsafe {
            CloseHandle(write_pipe);
            if !bstr_query.is_null() {
                SysFreeString(bstr_query);
            }
        }
        return hr as u32;
    }

    hr = unsafe {
        CoInitializeSecurity(
            null(),
            -1,
            null(),
            null(),
            RPC_C_AUTHN_LEVEL_DEFAULT,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            null(),
            EOAC_NONE,
            null(),
        )
    };
    if hr == RPC_E_TOO_LATE {
        hr = 0;
    }

    if succeeded(hr) {
        hr = unsafe {
            CoCreateInstance(
                &CLSID_WBEM_LOCATOR,
                null_mut(),
                CLSCTX_INPROC_SERVER,
                &IID_IWBEM_LOCATOR,
                &mut locator,
            )
        };
    }
    if succeeded(hr) && unsafe { SetEvent(init_event) } == 0 {
        hr = hresult_from_win32(unsafe { GetLastError() });
    }

    if succeeded(hr) {
        let root_cimv2 = wide_str("ROOT\\CIMV2");
        let bstr_root = unsafe { SysAllocString(root_cimv2.as_ptr()) };
        hr = unsafe {
            locator_connect_server(
                locator,
                bstr_root,
                null(),
                null(),
                null(),
                0,
                null(),
                null_mut(),
                &mut services,
            )
        };
        if !bstr_root.is_null() {
            unsafe { SysFreeString(bstr_root) };
        }
    }
    if succeeded(hr) && unsafe { SetEvent(connect_event) } == 0 {
        hr = hresult_from_win32(unsafe { GetLastError() });
    }

    if succeeded(hr) {
        hr = unsafe {
            CoSetProxyBlanket(
                services,
                RPC_C_AUTHN_WINNT,
                RPC_C_AUTHZ_NONE,
                null(),
                RPC_C_AUTHN_LEVEL_CALL,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                null(),
                EOAC_NONE,
            )
        };
    }
    if succeeded(hr) {
        let wql = wide_str("WQL");
        let bstr_wql = unsafe { SysAllocString(wql.as_ptr()) };
        hr = unsafe {
            services_exec_query(
                services,
                bstr_wql,
                bstr_query,
                WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
                null_mut(),
                &mut enumerator,
            )
        };
        if !bstr_wql.is_null() {
            unsafe { SysFreeString(bstr_wql) };
        }
    }

    let mut value: *mut c_void;
    let mut start_of_enum = true;
    let null_sep: u16 = 0;
    let eq_sign: u16 = b'=' as u16;

    while succeeded(hr) {
        let mut got: u32 = 0;
        let mut written: u32 = 0;
        value = null_mut();
        hr = unsafe { enum_next(enumerator, WBEM_INFINITE, 1, &mut value, &mut got) };

        if hr == WBEM_S_FALSE {
            hr = 0;
            break;
        }
        if failed(hr) || got != 1 || value.is_null() {
            continue;
        }

        if !start_of_enum
            && unsafe {
                WriteFile(
                    write_pipe,
                    &null_sep as *const u16 as *const _,
                    2,
                    &mut written,
                    null_mut(),
                )
            } == 0
        {
            hr = hresult_from_win32(unsafe { GetLastError() });
            unsafe { com_release(value) };
            break;
        }
        start_of_enum = false;

        hr = unsafe { object_begin_enumeration(value, 0) };
        if failed(hr) {
            unsafe { com_release(value) };
            break;
        }

        while succeeded(hr) {
            let mut prop_name: *mut u16 = null_mut();
            let mut prop_value = VARIANT::zeroed();
            let mut flavor: i32 = 0;

            hr = unsafe {
                object_next(
                    value,
                    0,
                    &mut prop_name,
                    &mut prop_value,
                    null_mut(),
                    &mut flavor,
                )
            };

            if hr == WBEM_S_NO_MORE_DATA {
                hr = 0;
                break;
            }

            if succeeded(hr) && (flavor & WBEM_FLAVOR_MASK_ORIGIN) != WBEM_FLAVOR_ORIGIN_SYSTEM {
                let mut prop_str = [0u16; BUFFER_SIZE];
                hr = unsafe {
                    VariantToString(&prop_value, prop_str.as_mut_ptr(), BUFFER_SIZE as u32)
                };

                if succeeded(hr) {
                    let cb_str1 = (unsafe { wcslen(prop_name) } * 2) as u32;
                    let cb_str2 = (unsafe { wcslen(prop_str.as_ptr()) } * 2) as u32;

                    if unsafe {
                        WriteFile(
                            write_pipe,
                            prop_name as *const _,
                            cb_str1,
                            &mut written,
                            null_mut(),
                        )
                    } == 0
                        || unsafe {
                            WriteFile(
                                write_pipe,
                                &eq_sign as *const u16 as *const _,
                                2,
                                &mut written,
                                null_mut(),
                            )
                        } == 0
                        || unsafe {
                            WriteFile(
                                write_pipe,
                                prop_str.as_ptr() as *const _,
                                cb_str2,
                                &mut written,
                                null_mut(),
                            )
                        } == 0
                        || unsafe {
                            WriteFile(
                                write_pipe,
                                &null_sep as *const u16 as *const _,
                                2,
                                &mut written,
                                null_mut(),
                            )
                        } == 0
                    {
                        hr = hresult_from_win32(unsafe { GetLastError() });
                    }
                }

                unsafe {
                    VariantClear(&mut prop_value);
                    SysFreeString(prop_name);
                }
            }
        }

        unsafe {
            object_end_enumeration(value);
            com_release(value);
        }
    }

    unsafe {
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
    }

    hr as u32
}

pub fn exec_query(query_str: &str) -> Result<String, ExecQueryError> {
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
                let data = Box::from_raw(thread_data_ptr);
                CloseHandle(data.write_pipe);
            }
        }

        if err == 0 {
            err = wait_event(init_event, 1000);
            if err == 0 {
                err = wait_event(connect_event, 100);
            }
        }

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
        return Err(ExecQueryError::MoreData);
    }
    if err != 0 {
        return Err(ExecQueryError::Code(err));
    }
    if offset == 0 {
        return Ok(String::new());
    }

    let char_count = (offset as usize) / 2 - 1;
    Ok(String::from_utf16_lossy(&buffer[..char_count]))
}
