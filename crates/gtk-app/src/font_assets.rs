#[cfg(windows)]
pub(crate) fn register_bundled_fonts() {
    use std::os::windows::ffi::OsStrExt;

    const FR_PRIVATE: u32 = 0x10;
    #[link(name = "gdi32")]
    unsafe extern "system" {
        fn AddFontResourceExW(path: *const u16, flags: u32, reserved: *mut std::ffi::c_void)
            -> i32;
    }

    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    let Some(bundle_root) = executable.parent() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(bundle_root.join("fonts")) else {
        return;
    };
    for path in entries.flatten().map(|entry| entry.path()).filter(|path| {
        matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("ttf" | "otf")
        )
    }) {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            AddFontResourceExW(wide.as_ptr(), FR_PRIVATE, std::ptr::null_mut());
        }
    }
}

#[cfg(not(windows))]
pub(crate) fn register_bundled_fonts() {}
