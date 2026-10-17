use super::*;
use jvmti_bindings::sys::jvmti;
use std::ffi::c_void;
use std::mem::MaybeUninit;
use std::ptr;
use std::sync::Mutex;

static TEST_LOCK: Mutex<()> = Mutex::new(());
static MOCK: Mutex<Mock> = Mutex::new(Mock::new());

struct Mock {
    attached: bool,
    jni_env: usize,
    jvmti_env: usize,
    attach_result: jni::jint,
    jvmti_result: jni::jint,
    version_fails: bool,
    dispose_failures: usize,
    detach_failures: usize,
    calls: Vec<&'static str>,
}

impl Mock {
    const fn new() -> Self {
        Self {
            attached: false,
            jni_env: 0,
            jvmti_env: 0,
            attach_result: jni::JNI_OK,
            jvmti_result: jni::JNI_OK,
            version_fails: false,
            dispose_failures: 0,
            detach_failures: 0,
            calls: Vec::new(),
        }
    }
}

unsafe extern "system" fn get_env(
    _vm: *mut jni::JavaVM,
    out: *mut *mut c_void,
    version: jni::jint,
) -> jni::jint {
    let mut mock = MOCK.lock().unwrap();
    match version {
        jni::JNI_VERSION_1_8 => {
            mock.calls.push("JNI GetEnv");
            if !mock.attached {
                return jni::JNI_EDETACHED;
            }
            unsafe { *out = mock.jni_env as *mut c_void };
            jni::JNI_OK
        }
        jvmti::JVMTI_VERSION_1_2 => {
            mock.calls.push("JVMTI GetEnv");
            if !mock.attached {
                return jni::JNI_EDETACHED;
            }
            unsafe { *out = mock.jvmti_env as *mut c_void };
            mock.jvmti_result
        }
        _ => jni::JNI_EVERSION,
    }
}

unsafe extern "system" fn attach(
    _vm: *mut jni::JavaVM,
    out: *mut *mut c_void,
    _args: *mut c_void,
) -> jni::jint {
    let mut mock = MOCK.lock().unwrap();
    mock.calls.push("attach daemon");
    if mock.attach_result == jni::JNI_OK {
        mock.attached = true;
        unsafe { *out = mock.jni_env as *mut c_void };
    }
    mock.attach_result
}

unsafe extern "system" fn unexpected_attach(
    _vm: *mut jni::JavaVM,
    _out: *mut *mut c_void,
    _args: *mut c_void,
) -> jni::jint {
    MOCK.lock()
        .unwrap()
        .calls
        .push("unexpected non-daemon attach");
    jni::JNI_ERR
}

unsafe extern "system" fn detach(_vm: *mut jni::JavaVM) -> jni::jint {
    let mut mock = MOCK.lock().unwrap();
    mock.calls.push("detach");
    if mock.detach_failures > 0 {
        mock.detach_failures -= 1;
        return jni::JNI_ERR;
    }
    mock.attached = false;
    jni::JNI_OK
}

unsafe extern "system" fn version(
    env: *mut jvmti::jvmtiEnv,
    out: *mut jni::jint,
) -> jvmti::jvmtiError {
    let mut mock = MOCK.lock().unwrap();
    mock.calls.push("version");
    if env as usize != mock.jvmti_env || !mock.attached || mock.version_fails {
        return jvmti::jvmtiError::INTERNAL;
    }
    unsafe { *out = jvmti::JVMTI_VERSION_1_2 };
    jvmti::jvmtiError::NONE
}

unsafe extern "system" fn dispose(env: *mut jvmti::jvmtiEnv) -> jvmti::jvmtiError {
    let mut mock = MOCK.lock().unwrap();
    mock.calls.push("dispose");
    if env as usize != mock.jvmti_env || !mock.attached {
        return jvmti::jvmtiError::INVALID_ENVIRONMENT;
    }
    if mock.dispose_failures > 0 {
        mock.dispose_failures -= 1;
        return jvmti::jvmtiError::INTERNAL;
    }
    jvmti::jvmtiError::NONE
}

fn with_vm(test: impl FnOnce(&mut Jvm)) {
    let mut table = jvmti::jvmtiInterface_1_::default();
    table.GetVersionNumber = Some(version);
    table.DisposeEnvironment = Some(dispose);
    let mut environment = jvmti::jvmtiEnv { functions: &table };

    // No JNI functions are called; only the returned JNIEnv pointer is checked.
    let native_table = MaybeUninit::<jni::JNINativeInterface_>::uninit();
    let mut native_env = native_table.as_ptr();
    let mut invoke_table = MaybeUninit::<jni::JNIInvokeInterface_>::uninit();
    let invoke_table = unsafe {
        let p = invoke_table.as_mut_ptr();
        ptr::addr_of_mut!((*p).reserved0).write(ptr::null_mut());
        ptr::addr_of_mut!((*p).reserved1).write(ptr::null_mut());
        ptr::addr_of_mut!((*p).reserved2).write(ptr::null_mut());
        ptr::addr_of_mut!((*p).DestroyJavaVM).write(detach);
        ptr::addr_of_mut!((*p).AttachCurrentThread).write(unexpected_attach);
        ptr::addr_of_mut!((*p).DetachCurrentThread).write(detach);
        ptr::addr_of_mut!((*p).GetEnv).write(get_env);
        ptr::addr_of_mut!((*p).AttachCurrentThreadAsDaemon).write(attach);
        invoke_table.assume_init()
    };
    let mut raw_vm = &invoke_table as jni::JavaVM;
    *MOCK.lock().unwrap() = Mock {
        jni_env: ptr::addr_of_mut!(native_env) as usize,
        jvmti_env: ptr::addr_of_mut!(environment) as usize,
        ..Mock::new()
    };
    let mut vm = unsafe { Jvm::from_raw(&mut raw_vm) }.unwrap();
    test(&mut vm);
    drop(vm);
}

fn count(call: &str) -> usize {
    MOCK.lock()
        .unwrap()
        .calls
        .iter()
        .filter(|&&c| c == call)
        .count()
}

#[test]
fn raw_vm_rejects_null_pointer_and_null_table() {
    let _lock = TEST_LOCK.lock().unwrap();
    assert!(unsafe { Jvm::from_raw(ptr::null_mut()) }.is_err());
    let mut raw_vm = ptr::null();
    assert!(unsafe { Jvm::from_raw(&mut raw_vm) }.is_err());
}

#[test]
fn shutdown_disposes_original_environment_once_while_attached() {
    let _lock = TEST_LOCK.lock().unwrap();
    with_vm(|vm| {
        vm.initialize().unwrap();
        assert_eq!(count("JVMTI GetEnv"), 1);
        assert!(!MOCK.lock().unwrap().attached);
        vm.shutdown().unwrap();
        assert_eq!(
            count("JVMTI GetEnv"),
            1,
            "shutdown must reuse its environment"
        );
        assert_eq!(count("dispose"), 1);
        assert_eq!(count("attach daemon"), 2);
        assert_eq!(count("detach"), 2);
        assert!(!MOCK.lock().unwrap().attached);
        let calls = MOCK.lock().unwrap().calls.len();
        vm.shutdown().unwrap();
        assert_eq!(MOCK.lock().unwrap().calls.len(), calls);
    });
    assert_eq!(
        count("dispose"),
        1,
        "Drop must not dispose an environment twice"
    );
}

#[test]
fn existing_jni_attachment_is_borrowed_and_never_detached() {
    let _lock = TEST_LOCK.lock().unwrap();
    with_vm(|vm| {
        MOCK.lock().unwrap().attached = true;
        vm.initialize().unwrap();
        vm.shutdown().unwrap();
        assert_eq!(count("attach daemon"), 0);
        assert_eq!(count("detach"), 0);
        assert_eq!(count("dispose"), 1);
        assert!(MOCK.lock().unwrap().attached);
    });
    assert_eq!(count("detach"), 0);
}

#[test]
fn failed_startup_releases_only_attachments_it_created() {
    let _lock = TEST_LOCK.lock().unwrap();
    for failure in ["attach", "null JNI", "JVMTI"] {
        with_vm(|vm| {
            {
                let mut mock = MOCK.lock().unwrap();
                match failure {
                    "attach" => mock.attach_result = jni::JNI_ERR,
                    "null JNI" => mock.jni_env = 0,
                    "JVMTI" => mock.jvmti_result = jni::JNI_EVERSION,
                    _ => unreachable!(),
                }
            }
            assert!(vm.initialize().is_err(), "{failure}");
            assert!(!MOCK.lock().unwrap().attached, "{failure}");
            assert_eq!(
                count("detach"),
                usize::from(failure != "attach"),
                "{failure}"
            );
            assert_eq!(count("dispose"), 0, "{failure}");
            let calls = MOCK.lock().unwrap().calls.len();
            vm.shutdown().unwrap();
            assert_eq!(MOCK.lock().unwrap().calls.len(), calls, "{failure}");
        });
    }
}

#[test]
fn failed_version_query_keeps_the_environment_for_cleanup() {
    let _lock = TEST_LOCK.lock().unwrap();
    with_vm(|vm| {
        MOCK.lock().unwrap().version_fails = true;
        assert!(vm.initialize().is_err());
        assert!(vm.environment.is_some());
        assert!(!MOCK.lock().unwrap().attached);
        vm.shutdown().unwrap();
        assert!(vm.environment.is_none());
        assert_eq!(count("JVMTI GetEnv"), 1);
        assert_eq!(count("dispose"), 1);
    });
}

#[test]
fn dispose_and_detach_failures_can_be_retried_with_the_same_environment() {
    let _lock = TEST_LOCK.lock().unwrap();
    with_vm(|vm| {
        vm.initialize().unwrap();
        MOCK.lock().unwrap().dispose_failures = 1;
        assert!(vm.shutdown().is_err());
        assert!(vm.environment.is_some());
        assert!(
            !MOCK.lock().unwrap().attached,
            "dispose failure still needs detach"
        );

        MOCK.lock().unwrap().detach_failures = 1;
        assert!(vm.shutdown().is_err());
        assert!(
            vm.environment.is_none(),
            "successful dispose must not be repeated"
        );
        assert!(
            vm.attached_by_us,
            "failed detach must retain ownership for retry"
        );

        vm.shutdown().unwrap();
        assert!(!vm.attached_by_us);
        assert!(!MOCK.lock().unwrap().attached);
        assert_eq!(count("JVMTI GetEnv"), 1);
        assert_eq!(count("dispose"), 2);
        assert_eq!(count("detach"), 4);
    });
}

#[test]
fn drop_cleans_up_an_initialized_environment() {
    let _lock = TEST_LOCK.lock().unwrap();
    with_vm(|vm| vm.initialize().map(|_| ()).unwrap());
    assert_eq!(count("dispose"), 1);
    assert!(!MOCK.lock().unwrap().attached);
}
