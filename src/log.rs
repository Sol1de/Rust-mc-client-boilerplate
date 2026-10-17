use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::Mutex;
use windows_sys::Win32::System::Diagnostics::Debug::OutputDebugStringW;

static LOG_FILE: Mutex<Option<File>> = Mutex::new(None);

pub fn init() {
    let path = std::env::temp_dir().join(format!("ares_client-{}.log", std::process::id()));
    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(file) => {
            *LOG_FILE.lock().unwrap_or_else(|error| error.into_inner()) = Some(file);
            info(&format!("log file: {}", path.display()));
        }
        Err(error) => info(&format!("cannot open log file {}: {error}", path.display())),
    }
}

pub fn info(message: &str) {
    let line = format!("[ares_client] {message}\r\n");
    let mut guard = LOG_FILE.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(file) = guard.as_mut()
        && let Err(error) = file.write_all(line.as_bytes())
    {
        debug(&format!("log write failed: {error}"));
    }
    debug(&line);
}

pub fn shutdown() {
    // FreeLibraryAndExitThread does not run Rust static destructors.
    LOG_FILE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take();
}

fn debug(message: &str) {
    let text: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    // SAFETY: The buffer remains valid and NUL-terminated until the call returns.
    unsafe { OutputDebugStringW(text.as_ptr()) };
}
