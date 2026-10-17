//! Minimal Windows x64 DLL for connecting to an existing Java 21 JVM.

#[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
compile_error!("AresClient requires Windows x64");

mod hotkey;
mod jvm;
mod log;

use std::ffi::c_void;
use windows_sys::Win32::Foundation::{CloseHandle, HINSTANCE, TRUE};
use windows_sys::Win32::System::LibraryLoader::FreeLibraryAndExitThread;
use windows_sys::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows_sys::Win32::System::Threading::CreateThread;
use windows_sys::core::BOOL;

unsafe extern "system" fn client_thread(module: *mut c_void) -> u32 {
    log::init();
    let mut connection = match jvm::Jvm::find() {
        Ok(mut connection) => {
            if let Err(error) = connection.initialize() {
                log::info(&format!(
                    "initialization failed: {error}; press END to clean up"
                ));
            }
            Some(connection)
        }
        Err(error) => {
            log::info(&format!("connection failed: {error}; press END to unload"));
            None
        }
    };
    hotkey::wait_for_unload(|| {
        if let Some(connection) = &mut connection {
            connection.shutdown()?;
        }
        Ok(())
    });
    // FreeLibraryAndExitThread skips Rust stack destructors. Drop resources first.
    drop(connection);
    log::info("cleanup complete; unloading DLL");
    log::shutdown();
    // SAFETY: This is the last client thread; no callbacks or JNI references remain.
    unsafe { FreeLibraryAndExitThread(module, 0) }
}

/// Only schedule the worker while the Windows loader lock is held.
///
/// # Safety
/// Called only by the Windows loader with a valid module and notification reason.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllMain(module: HINSTANCE, reason: u32, _: *mut c_void) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        // SAFETY: Windows supplies a loaded module; the worker starts after DllMain returns.
        let thread = unsafe {
            CreateThread(
                std::ptr::null(),
                0,
                Some(client_thread),
                module,
                0,
                std::ptr::null_mut(),
            )
        };
        if thread.is_null() {
            return 0;
        }
        // SAFETY: Close the thread handle, not the running thread.
        unsafe { CloseHandle(thread) };
    }
    TRUE
}
