use crate::log;
use std::time::Duration;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_END};

/// Wait for END, allowing a fresh press to retry a failed unload.
pub fn wait_for_unload(unload: impl FnMut() -> Result<(), String>) {
    log::info("press END to unload");
    poll_end_key(
        || unsafe { GetAsyncKeyState(VK_END as i32) as u16 & 0x8000 != 0 },
        unload,
    );
}

fn poll_end_key(
    mut key_down: impl FnMut() -> bool,
    mut unload: impl FnMut() -> Result<(), String>,
) {
    loop {
        // Ignore a held key at startup and after a failed attempt.
        while key_down() {
            std::thread::sleep(Duration::from_millis(50));
        }
        while !key_down() {
            std::thread::sleep(Duration::from_millis(50));
        }
        match unload() {
            Ok(()) => return,
            Err(error) => log::info(&format!(
                "unload cancelled: {error}. Release END and press it again to retry"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn timeout_requires_release_and_new_end_press_before_retry() {
        let keys = [true, false, false, true, true, true, false, false, true];
        let reads = Cell::new(0);
        let attempts = Cell::new(0);
        poll_end_key(
            || {
                let index = reads.get();
                assert!(index < keys.len(), "retry must eventually succeed");
                if (4..=6).contains(&index) {
                    assert_eq!(attempts.get(), 1, "holding END must not retry");
                }
                reads.set(index + 1);
                keys[index]
            },
            || {
                attempts.set(attempts.get() + 1);
                if attempts.get() == 1 {
                    assert_eq!(reads.get(), 4, "ignore END held at startup");
                    Err("cleanup timed out".into())
                } else {
                    assert_eq!(reads.get(), 9, "retry needs a fresh END press");
                    Ok(())
                }
            },
        );
        assert_eq!(attempts.get(), 2);
        assert_eq!(reads.get(), keys.len());
    }
}
