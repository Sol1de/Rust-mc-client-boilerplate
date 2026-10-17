//! A borrowed JVM and one owned JVMTI connection, confined to the client thread.

use crate::log;
use jvmti_bindings::env::Jvmti;
use jvmti_bindings::sys::jni::{self, JNI_GetCreatedJavaVMs, JavaVM};
use jvmti_bindings::{describe_jni_result, jvm_call};
use std::ffi::c_void;
use std::ptr::{self, NonNull};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

pub struct Jvm {
    vm: NonNull<JavaVM>,
    environment: Option<Jvmti>,
    attached_by_us: bool,
}

impl Jvm {
    pub fn find() -> Result<Self, String> {
        let name: Vec<u16> = "jvm.dll\0".encode_utf16().collect();
        // SAFETY: Query an already loaded module; never create or own the host JVM.
        unsafe {
            let module = GetModuleHandleW(name.as_ptr());
            if module.is_null() {
                return Err("jvm.dll is not loaded in this process".into());
            }
            let symbol = GetProcAddress(module, c"JNI_GetCreatedJavaVMs".as_ptr().cast())
                .ok_or("JNI_GetCreatedJavaVMs is unavailable")?;
            let get_vms: JNI_GetCreatedJavaVMs = std::mem::transmute(symbol);
            let mut vm = ptr::null_mut();
            let mut count = 0;
            let status = get_vms(&mut vm, 1, &mut count);
            check_jni("JNI_GetCreatedJavaVMs", status)?;
            if count == 0 {
                return Err("no running JVM was found".into());
            }
            Self::from_raw(vm)
        }
    }

    /// # Safety
    /// The invocation table must belong to a live JVM for this object's lifetime.
    unsafe fn from_raw(vm: *mut JavaVM) -> Result<Self, String> {
        let vm = NonNull::new(vm).ok_or("the JVM pointer is null")?;
        if unsafe { vm.as_ref() }.is_null() {
            return Err("the JVM invocation table is null".into());
        }
        Ok(Self {
            vm,
            environment: None,
            attached_by_us: false,
        })
    }

    pub fn initialize(&mut self) -> Result<(), String> {
        let result = self.attach_current_thread().and_then(|()| {
            if self.environment.is_none() {
                // SAFETY: The VM was discovered in this process; this thread is attached.
                let env = unsafe { Jvmti::from_java_vm_raw(self.vm.as_ptr()) }
                    .map_err(|code| format!("GetEnv(JVMTI): {}", describe_jni_result(code)))?;
                self.environment = Some(env);
            }
            if let Some(env) = &self.environment {
                let version = env
                    .get_version_number()
                    .map_err(|error| format!("GetVersionNumber: {error:?}"))?;
                log::info(&format!("connected to JVM; JVMTI version 0x{version:08x}"));
            }
            Ok(())
        });
        self.finish_operation(result)
    }

    pub fn shutdown(&mut self) -> Result<(), String> {
        if self.environment.is_none() && !self.attached_by_us {
            return Ok(());
        }
        let result = self.attach_current_thread().and_then(|()| {
            if let Some(env) = self.environment.take() {
                let raw = env.raw();
                // SAFETY: This base installs no callbacks and hands out no JVMTI handles.
                if let Err(error) = unsafe { env.dispose_environment() } {
                    // A failed disposal leaves the connection valid for another attempt.
                    self.environment = Some(unsafe { Jvmti::from_raw(raw) });
                    return Err(format!("DisposeEnvironment: {error:?}"));
                }
            }
            Ok(())
        });
        self.finish_operation(result)
    }

    fn attach_current_thread(&mut self) -> Result<(), String> {
        if self.attached_by_us {
            return Ok(());
        }
        let mut env: *mut c_void = ptr::null_mut();
        // SAFETY: All JNI calls stay on the thread owning this object.
        unsafe {
            let status = jvm_call!(self.vm.as_ptr(), GetEnv, &mut env, jni::JNI_VERSION_1_8);
            match status {
                jni::JNI_OK => {}
                jni::JNI_EDETACHED => {
                    let status = jvm_call!(
                        self.vm.as_ptr(),
                        AttachCurrentThreadAsDaemon,
                        &mut env,
                        ptr::null_mut()
                    );
                    check_jni("AttachCurrentThreadAsDaemon", status)?;
                    self.attached_by_us = true;
                }
                code => return Err(format!("GetEnv(JNI): {}", describe_jni_result(code))),
            }
        }
        if env.is_null() {
            return Err("the JVM returned a null JNIEnv".into());
        }
        Ok(())
    }

    fn finish_operation(&mut self, operation: Result<(), String>) -> Result<(), String> {
        let detached = if self.attached_by_us {
            // SAFETY: No JNIEnv or local reference escapes an operation.
            let status = unsafe { jvm_call!(self.vm.as_ptr(), DetachCurrentThread) };
            if status == jni::JNI_OK {
                self.attached_by_us = false;
            }
            check_jni("DetachCurrentThread", status)
        } else {
            Ok(())
        };
        match (operation, detached) {
            (Err(operation), Err(detach)) => Err(format!("{operation}; {detach}")),
            (Err(error), _) | (_, Err(error)) => Err(error),
            _ => Ok(()),
        }
    }
}

impl Drop for Jvm {
    fn drop(&mut self) {
        if let Err(error) = self.shutdown() {
            log::info(&format!("JVM cleanup failed: {error}"));
        }
    }
}

fn check_jni(operation: &str, status: jni::jint) -> Result<(), String> {
    if status == jni::JNI_OK {
        Ok(())
    } else {
        Err(format!("{operation}: {}", describe_jni_result(status)))
    }
}

#[cfg(test)]
mod smoke;
#[cfg(test)]
mod tests;
