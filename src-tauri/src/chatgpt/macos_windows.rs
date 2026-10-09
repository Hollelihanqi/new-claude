use std::collections::HashSet;
use std::ffi::c_void;

type Ref = *const c_void;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGWindowListCopyWindowInfo(options: u32, relative_to: u32) -> Ref;
    static kCGWindowOwnerPID: Ref;
    static kCGWindowLayer: Ref;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFArrayGetCount(array: Ref) -> isize;
    fn CFArrayGetValueAtIndex(array: Ref, index: isize) -> Ref;
    fn CFDictionaryGetValue(dictionary: Ref, key: Ref) -> Ref;
    fn CFNumberGetValue(number: Ref, number_type: isize, value: *mut c_void) -> u8;
    fn CFRelease(value: Ref);
}

struct WindowList(Ref);

impl Drop for WindowList {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) };
    }
}

unsafe fn integer(dictionary: Ref, key: Ref) -> Option<i32> {
    let number = CFDictionaryGetValue(dictionary, key);
    let mut value: i32 = 0;
    // kCFNumberIntType = 9. Both window layer and owner PID are CFNumber values.
    if !number.is_null() && CFNumberGetValue(number, 9, &mut value as *mut _ as *mut c_void) != 0 {
        Some(value)
    } else {
        None
    }
}

pub fn visible_pids() -> Result<HashSet<u32>, String> {
    // kCGWindowListOptionOnScreenOnly = 1; kCGNullWindowID = 0.
    // Read metadata only; no capture or accessibility permission is required.
    let raw = unsafe { CGWindowListCopyWindowInfo(1, 0) };
    if raw.is_null() {
        // NULL is not an empty window list: the GUI session is unavailable.
        return Err("无法读取当前桌面的窗口状态，请在桌面会话中重试".into());
    }
    let list = WindowList(raw);
    let mut pids = HashSet::new();
    unsafe {
        for index in 0..CFArrayGetCount(list.0) {
            let window = CFArrayGetValueAtIndex(list.0, index);
            if integer(window, kCGWindowLayer) == Some(0) {
                if let Some(pid) = integer(window, kCGWindowOwnerPID).filter(|pid| *pid > 0) {
                    pids.insert(pid as u32);
                }
            }
        }
    }
    Ok(pids)
}
