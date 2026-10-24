//! whisper.cpp links the Vulkan loader, which Windows has only with a graphics driver: a virtual machine or a
//! computer on Microsoft Basic Display Adapter lacks it, and Windows would not start Nuzky there. The binaries load
//! it on first use instead (`/DELAYLOAD` in their build.rs). When it is missing, ggml gets a stand-in loader that
//! fails, so ggml leaves Vulkan out and speech runs on the processor.

use std::{
    ffi::{CStr, c_char, c_void},
    ptr::null,
};

/// The start of `DelayLoadInfo` from delayimp.h, up to the function asked for.
#[repr(C)]
struct DelayLoadInfo {
    _size: u32,
    _descriptor: *const c_void,
    _iat_entry: *mut c_void,
    dll: *const c_char,
    by_name: i32,
    name: *const c_char,
}

const FAIL_LOAD_LIB: u32 = 3;
const FAIL_GET_PROC: u32 = 4;
const VK_ERROR_INITIALIZATION_FAILED: i32 = -3;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
}

/// The delay-load helper calls this when a library or a function in it is missing. link.exe takes it from here
/// instead of from delayimp.lib because build.rs names it with `/INCLUDE`.
#[allow(non_upper_case_globals)]
#[unsafe(no_mangle)]
static __pfnDliFailureHook2: unsafe extern "system" fn(u32, *const DelayLoadInfo) -> *const c_void = missing;

unsafe extern "system" fn missing(event: u32, info: *const DelayLoadInfo) -> *const c_void {
    // SAFETY: the helper passes the DelayLoadInfo it filled in, whose names are NUL-terminated.
    let info = unsafe { &*info };
    if !unsafe { CStr::from_ptr(info.dll) }.to_bytes().eq_ignore_ascii_case(b"vulkan-1.dll") {
        return null();
    }
    match event {
        // Nuzky's executable stands in for the library. It has none of the loader's functions, so each one asked
        // for comes back here.
        FAIL_LOAD_LIB => unsafe { GetModuleHandleW(null()) }.cast_const(),
        FAIL_GET_PROC if info.by_name != 0 && unsafe { CStr::from_ptr(info.name) } == c"vkGetInstanceProcAddr" => {
            get_instance_proc_addr as *const c_void
        }
        _ => null(),
    }
}

/// ggml asks for the Vulkan version first and starts without Vulkan when that fails.
unsafe extern "system" fn get_instance_proc_addr(_instance: *mut c_void, name: *const c_char) -> *const c_void {
    // SAFETY: Vulkan function names are NUL-terminated.
    if unsafe { CStr::from_ptr(name) } == c"vkEnumerateInstanceVersion" {
        enumerate_instance_version as *const c_void
    } else {
        null()
    }
}

unsafe extern "system" fn enumerate_instance_version(_version: *mut u32) -> i32 {
    VK_ERROR_INITIALIZATION_FAILED
}
