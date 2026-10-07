//! Loading an O&C-family firmware (stock Ornaments & Crimes, Hemisphere Suite,
//! Phazerville Suite...) as a library of its own.
//!
//! build.rs builds each firmware as a shared library with a small C API
//! (`ocfw_*`, in vendor/o_c/host/oc_host_core.cpp). A firmware keeps its state in
//! globals, so a module gets a *copy* of the library file, loaded under its own
//! path: the loader treats each copy as a separate image with separate globals,
//! which is what lets four modules run at once, each on a firmware of its own
//! choosing. Dropping a `Firmware` stops its thread, unloads the copy and deletes
//! it.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg_attr(target_os = "linux", link(name = "dl"))]
unsafe extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
    fn dlerror() -> *const c_char;
}

const RTLD_NOW: c_int = 2;
// 4 on macOS, but 0 on Linux, where 4 is RTLD_NOLOAD ("only return a library that is
// already loaded"): dlopen then fails with no error message at all.
const RTLD_LOCAL: c_int = if cfg!(target_os = "macos") { 4 } else { 0 };

/// A firmware the build knows how to make.
pub struct Variant {
    pub id: &'static str,
    pub name: &'static str,
}

/// Every firmware, in menu order. Only those whose library was built are offered.
pub const VARIANTS: [Variant; 3] = [
    Variant { id: "stock", name: "Ornaments & Crimes" },
    Variant { id: "hemi", name: "Hemisphere Suite" },
    Variant { id: "phaz", name: "Phazerville Suite" },
];

fn library_dir() -> PathBuf {
    PathBuf::from(env!("PORTAMAX_OCFW_DIR"))
}

fn library_path(id: &str) -> PathBuf {
    library_dir().join(format!("libocfw_{id}.{}", if cfg!(target_os = "macos") { "dylib" } else { "so" }))
}

/// The variants whose libraries exist, as indices into `VARIANTS`.
pub fn available() -> Vec<usize> {
    (0..VARIANTS.len()).filter(|&i| library_path(VARIANTS[i].id).exists()).collect()
}

/// A loaded firmware. Its functions are the library's own `ocfw_*`.
pub struct Firmware {
    handle: *mut c_void,
    path: PathBuf,
    started: std::sync::atomic::AtomicBool,
    start: unsafe extern "C" fn(),
    stop: unsafe extern "C" fn(),
    timers_running: unsafe extern "C" fn() -> c_int,
    run_isrs: unsafe extern "C" fn(c_int, c_int),
    set_pin: unsafe extern "C" fn(c_int, c_int),
    set_cv: unsafe extern "C" fn(c_int, c_int),
    dac_mv: unsafe extern "C" fn(c_int) -> c_int,
    frame: unsafe extern "C" fn(*mut u8) -> u64,
    eeprom_read: unsafe extern "C" fn(*mut u8),
    eeprom_write: unsafe extern "C" fn(*const u8),
    app_name: unsafe extern "C" fn() -> *const c_char,
}

// The library's entry points are thread-safe by design (atomics and a mutex), and
// the handle is only used to unload it.
unsafe impl Send for Firmware {}
unsafe impl Sync for Firmware {}

static COPIES: AtomicU64 = AtomicU64::new(0);

fn symbol<T: Copy>(handle: *mut c_void, name: &str) -> Result<T, String> {
    let c = CString::new(name).unwrap();
    let p = unsafe { dlsym(handle, c.as_ptr()) };
    if p.is_null() {
        return Err(format!("the firmware library has no {name}"));
    }
    // A function pointer is the size of a data pointer on every platform we run on.
    Ok(unsafe { std::mem::transmute_copy::<*mut c_void, T>(&p) })
}

#[allow(dead_code)] // not used by the main binary
impl Firmware {
    /// Loads a fresh copy of variant `id`'s library. It is not running yet.
    pub fn load(id: &str) -> Result<Firmware, String> {
        let source = library_path(id);
        if !source.exists() {
            return Err(format!("{id} was not built into this binary"));
        }
        let ext = source.extension().and_then(|e| e.to_str()).unwrap_or("so").to_string();
        let copy = std::env::temp_dir().join(format!("portamax-ocfw-{}-{}-{}.{ext}", std::process::id(), id, COPIES.fetch_add(1, Ordering::Relaxed)));
        std::fs::copy(&source, &copy).map_err(|e| format!("could not copy the firmware: {e}"))?;
        let c = CString::new(copy.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        let handle = unsafe { dlopen(c.as_ptr(), RTLD_NOW | RTLD_LOCAL) };
        if handle.is_null() {
            let err = unsafe { dlerror() };
            let msg = if err.is_null() { "dlopen failed".to_string() } else { unsafe { CStr::from_ptr(err) }.to_string_lossy().into_owned() };
            std::fs::remove_file(&copy).ok();
            return Err(msg);
        }
        let loaded = (|| -> Result<Firmware, String> {
            Ok(Firmware {
                handle,
                path: copy.clone(),
                started: std::sync::atomic::AtomicBool::new(false),
                start: symbol(handle, "ocfw_start")?,
                stop: symbol(handle, "ocfw_stop")?,
                timers_running: symbol(handle, "ocfw_timers_running")?,
                run_isrs: symbol(handle, "ocfw_run_isrs")?,
                set_pin: symbol(handle, "ocfw_set_pin")?,
                set_cv: symbol(handle, "ocfw_set_cv_millivolts")?,
                dac_mv: symbol(handle, "ocfw_dac_millivolts")?,
                frame: symbol(handle, "ocfw_frame")?,
                eeprom_read: symbol(handle, "ocfw_eeprom_read")?,
                eeprom_write: symbol(handle, "ocfw_eeprom_write")?,
                app_name: symbol(handle, "ocfw_app_name")?,
            })
        })();
        if loaded.is_err() {
            unsafe { dlclose(handle) };
            std::fs::remove_file(&copy).ok();
        }
        loaded
    }

    /// Starts the firmware's own setup() and loop() on its thread.
    pub fn start(&self) {
        if !self.started.swap(true, Ordering::SeqCst) {
            unsafe { (self.start)() };
        }
    }

    pub fn timers_running(&self) -> bool {
        unsafe { (self.timers_running)() != 0 }
    }
    pub fn run_isrs(&self, core: i32, ui: i32) {
        unsafe { (self.run_isrs)(core, ui) }
    }
    pub fn set_pin(&self, pin: i32, level: i32) {
        unsafe { (self.set_pin)(pin, level) }
    }
    pub fn set_cv_millivolts(&self, channel: i32, mv: i32) {
        unsafe { (self.set_cv)(channel, mv) }
    }
    pub fn dac_millivolts(&self, channel: i32) -> i32 {
        unsafe { (self.dac_mv)(channel) }
    }
    /// Copies the latest OLED frame (128x64, 1024 bytes in 8 pages) and returns the frames drawn so far.
    pub fn frame(&self, out: &mut [u8; 1024]) -> u64 {
        unsafe { (self.frame)(out.as_mut_ptr()) }
    }
    pub fn eeprom_read(&self, out: &mut [u8]) {
        assert!(out.len() >= 8192);
        unsafe { (self.eeprom_read)(out.as_mut_ptr()) }
    }
    /// Sets the EEPROM; only meaningful before `start`, since the firmware reads its settings as it boots.
    pub fn eeprom_write(&self, data: &[u8]) {
        assert!(data.len() >= 8192);
        unsafe { (self.eeprom_write)(data.as_ptr()) }
    }
    /// The name of the app the firmware is showing ("CopierMaschine"...).
    pub fn app_name(&self) -> String {
        let p = unsafe { (self.app_name)() };
        if p.is_null() { String::new() } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned() }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Firmware {
    fn drop(&mut self) {
        // The thread leaves at its next checkpoint; the host has stopped running
        // the interrupts by now (see OcApp, which only drops a firmware nobody else holds).
        if self.started.load(Ordering::SeqCst) {
            unsafe { (self.stop)() };
        }
        unsafe { dlclose(self.handle) };
        std::fs::remove_file(&self.path).ok();
    }
}
