use super::Jvm;
use jvmti_bindings::embed::JavaVmBuilder;
use jvmti_bindings::jvm_call;
use jvmti_bindings::sys::jni;
use std::ffi::c_void;
use std::ptr;

#[test]
#[ignore = "requires JAVA_HOME pointing to a Windows x64 JDK"]
fn jdk21_lifecycle() {
    let java_home = std::env::var_os("JAVA_HOME").expect("set JAVA_HOME to the JDK directory");
    let embedded = JavaVmBuilder::default()
        .option("-Xcheck:jni")
        .unwrap()
        .create_from_java_home(java_home)
        .expect("create test JVM");

    let worker = std::thread::spawn(|| {
        for _ in 0..3 {
            let mut client = Jvm::find().expect("find running JVM");
            let mut env: *mut c_void = ptr::null_mut();
            let status =
                unsafe { jvm_call!(client.vm.as_ptr(), GetEnv, &mut env, jni::JNI_VERSION_1_8) };
            assert_eq!(status, jni::JNI_EDETACHED);

            client.initialize().expect("initialize JNI/JVMTI session");
            let status =
                unsafe { jvm_call!(client.vm.as_ptr(), GetEnv, &mut env, jni::JNI_VERSION_1_8) };
            assert_eq!(status, jni::JNI_EDETACHED);

            client.shutdown().expect("dispose JVMTI and detach worker");
            let status =
                unsafe { jvm_call!(client.vm.as_ptr(), GetEnv, &mut env, jni::JNI_VERSION_1_8) };
            assert_eq!(status, jni::JNI_EDETACHED);
        }
    });

    worker.join().expect("JVM lifecycle worker");
    drop(embedded);
}
