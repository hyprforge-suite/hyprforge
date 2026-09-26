//! libmpv, loaded at run time.
//!
//! Loaded with `dlopen` rather than linked, so a viewer built with video
//! support still starts — and still shows every picture — on a machine
//! without mpv. A missing library is [`MpvError::Missing`], which the
//! window turns into a sentence, never a failure to start: the suite's
//! rule that an absent sibling is a state, not a crash.
//!
//! Every `unsafe` in this crate is in this file. The declarations below
//! are transcribed from `/usr/include/mpv/client.h` and `render.h` (client
//! API 2.5, mpv 0.41) — the numeric values of the enums included, because
//! a wrong one does not fail to compile, it silently asks for something
//! else.

use libloading::Library;
use std::ffi::{c_char, c_double, c_int, c_void, CStr, CString};
use std::sync::Arc;

pub type Handle = c_void;
pub type RenderContext = c_void;

/// `mpv_format`.
pub const FORMAT_FLAG: c_int = 3;
pub const FORMAT_INT64: c_int = 4;
pub const FORMAT_DOUBLE: c_int = 5;

/// `mpv_event_id`.
pub const EVENT_NONE: c_int = 0;
pub const EVENT_SHUTDOWN: c_int = 1;
pub const EVENT_END_FILE: c_int = 7;
pub const EVENT_FILE_LOADED: c_int = 8;
pub const EVENT_PROPERTY_CHANGE: c_int = 22;

/// `mpv_render_param_type`.
pub const RENDER_PARAM_INVALID: c_int = 0;
pub const RENDER_PARAM_API_TYPE: c_int = 1;
pub const RENDER_PARAM_SW_SIZE: c_int = 17;
pub const RENDER_PARAM_SW_FORMAT: c_int = 18;
pub const RENDER_PARAM_SW_STRIDE: c_int = 19;
pub const RENDER_PARAM_SW_POINTER: c_int = 20;

/// `MPV_RENDER_UPDATE_FRAME`: a new frame should be rendered.
pub const RENDER_UPDATE_FRAME: u64 = 1;

#[repr(C)]
pub struct RenderParam {
    pub kind: c_int,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct Event {
    pub event_id: c_int,
    pub error: c_int,
    pub reply_userdata: u64,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct EventProperty {
    pub name: *const c_char,
    pub format: c_int,
    pub data: *mut c_void,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum MpvError {
    /// libmpv is not installed.
    #[error("mpv's library isn't installed, so videos can't play here")]
    Missing,
    /// libmpv answered with an error code.
    #[error("mpv: {0}")]
    Failed(String),
}

/// The functions this crate calls, resolved once from the library.
pub struct Api {
    // Held so the function pointers below stay valid.
    _library: Library,
    pub create: unsafe extern "C" fn() -> *mut Handle,
    pub initialize: unsafe extern "C" fn(*mut Handle) -> c_int,
    pub terminate_destroy: unsafe extern "C" fn(*mut Handle),
    pub set_option_string: unsafe extern "C" fn(*mut Handle, *const c_char, *const c_char) -> c_int,
    pub set_property: unsafe extern "C" fn(*mut Handle, *const c_char, c_int, *mut c_void) -> c_int,
    pub command: unsafe extern "C" fn(*mut Handle, *mut *const c_char) -> c_int,
    pub observe_property: unsafe extern "C" fn(*mut Handle, u64, *const c_char, c_int) -> c_int,
    pub wait_event: unsafe extern "C" fn(*mut Handle, c_double) -> *mut Event,
    pub set_wakeup_callback: unsafe extern "C" fn(*mut Handle, Option<unsafe extern "C" fn(*mut c_void)>, *mut c_void),
    pub error_string: unsafe extern "C" fn(c_int) -> *const c_char,
    pub render_context_create: unsafe extern "C" fn(*mut *mut RenderContext, *mut Handle, *mut RenderParam) -> c_int,
    pub render_context_set_update_callback:
        unsafe extern "C" fn(*mut RenderContext, Option<unsafe extern "C" fn(*mut c_void)>, *mut c_void),
    pub render_context_update: unsafe extern "C" fn(*mut RenderContext) -> u64,
    pub render_context_render: unsafe extern "C" fn(*mut RenderContext, *mut RenderParam) -> c_int,
    pub render_context_free: unsafe extern "C" fn(*mut RenderContext),
}

impl Api {
    /// Loads libmpv. The soname first, then the development name, which is
    /// all some distributions ship.
    pub fn load() -> Result<Arc<Api>, MpvError> {
        // SAFETY: loading a shared library runs its initialisers; libmpv's
        // are the documented way to use it and touch no state of ours.
        let library = ["libmpv.so.2", "libmpv.so"]
            .iter()
            .find_map(|name| unsafe { Library::new(name) }.ok())
            .ok_or(MpvError::Missing)?;
        macro_rules! sym {
            ($name:literal) => {
                // SAFETY: the type each symbol is read as is transcribed
                // from mpv's own headers for this exact name.
                *unsafe { library.get(concat!($name, "\0").as_bytes()) }.map_err(|_| MpvError::Missing)?
            };
        }
        Ok(Arc::new(Api {
            create: sym!("mpv_create"),
            initialize: sym!("mpv_initialize"),
            terminate_destroy: sym!("mpv_terminate_destroy"),
            set_option_string: sym!("mpv_set_option_string"),
            set_property: sym!("mpv_set_property"),
            command: sym!("mpv_command"),
            observe_property: sym!("mpv_observe_property"),
            wait_event: sym!("mpv_wait_event"),
            set_wakeup_callback: sym!("mpv_set_wakeup_callback"),
            error_string: sym!("mpv_error_string"),
            render_context_create: sym!("mpv_render_context_create"),
            render_context_set_update_callback: sym!("mpv_render_context_set_update_callback"),
            render_context_update: sym!("mpv_render_context_update"),
            render_context_render: sym!("mpv_render_context_render"),
            render_context_free: sym!("mpv_render_context_free"),
            _library: library,
        }))
    }

    /// `Ok` for a non-negative mpv return code, the library's own message
    /// for a negative one.
    pub fn check(&self, code: c_int, what: &str) -> Result<(), MpvError> {
        if code >= 0 {
            return Ok(());
        }
        // SAFETY: mpv_error_string returns a static string for any code.
        let said = unsafe { CStr::from_ptr((self.error_string)(code)) }.to_string_lossy();
        Err(MpvError::Failed(format!("{what}: {said}")))
    }
}

/// A string for mpv. Interior NULs cannot occur in the names and values
/// this crate passes, but a path can hold one in principle, and that is
/// an error rather than a panic.
pub fn cstring(s: &str) -> Result<CString, MpvError> {
    CString::new(s).map_err(|_| MpvError::Failed(format!("{s:?} contains a NUL byte")))
}

/// What an mpv event says, copied out of mpv's memory so it can outlive
/// the next `wait_event`, which invalidates the last one.
#[derive(Debug, Clone, PartialEq)]
pub enum Happened {
    Nothing,
    Shutdown,
    FileLoaded,
    /// The file ended; `error` is mpv's code when it ended because it
    /// could not be played.
    EndFile { error: bool },
    Double(String, f64),
    Flag(String, bool),
    Int(String, i64),
    Other,
}

/// Something mpv calls back on its own threads: it may only nudge, and
/// the nudge has to be `Send`. A boxed closure, leaked into a raw pointer
/// for mpv to hold and reclaimed when the owner is dropped.
type Nudge = Box<dyn Fn() + Send + Sync>;

unsafe extern "C" fn nudge(data: *mut c_void) {
    // SAFETY: `data` is the `Nudge` this crate boxed and handed to mpv; it
    // lives until mpv is told to stop calling (see the `Drop`s below).
    let f = unsafe { &*(data as *const Nudge) };
    f();
}

/// One mpv instance: a player with no window of its own.
pub struct Mpv {
    api: Arc<Api>,
    handle: *mut Handle,
    wakeup: Option<*mut Nudge>,
}

// SAFETY: the client API is thread-safe by specification ("all functions
// are thread-safe" — client.h); the handle is only ever used through it.
unsafe impl Send for Mpv {}

impl Mpv {
    pub fn new(api: Arc<Api>) -> Result<Mpv, MpvError> {
        // SAFETY: mpv_create has no preconditions.
        let handle = unsafe { (api.create)() };
        if handle.is_null() {
            return Err(MpvError::Failed("mpv_create returned nothing".into()));
        }
        Ok(Mpv { api, handle, wakeup: None })
    }

    pub fn set_option(&self, name: &str, value: &str) -> Result<(), MpvError> {
        let (n, v) = (cstring(name)?, cstring(value)?);
        // SAFETY: valid handle, NUL-terminated strings that outlive the call.
        let code = unsafe { (self.api.set_option_string)(self.handle, n.as_ptr(), v.as_ptr()) };
        self.api.check(code, name)
    }

    pub fn initialize(&self) -> Result<(), MpvError> {
        // SAFETY: valid, not yet initialised handle.
        self.api.check(unsafe { (self.api.initialize)(self.handle) }, "initialize")
    }

    pub fn command(&self, args: &[&str]) -> Result<(), MpvError> {
        let owned: Vec<CString> = args.iter().map(|a| cstring(a)).collect::<Result<_, _>>()?;
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        // SAFETY: a NULL-terminated array of strings that outlive the call.
        let code = unsafe { (self.api.command)(self.handle, ptrs.as_mut_ptr()) };
        self.api.check(code, args.first().copied().unwrap_or("command"))
    }

    pub fn set_flag(&self, name: &str, on: bool) -> Result<(), MpvError> {
        let n = cstring(name)?;
        let mut v: c_int = on.into();
        // SAFETY: MPV_FORMAT_FLAG takes an int*.
        let code = unsafe { (self.api.set_property)(self.handle, n.as_ptr(), FORMAT_FLAG, &mut v as *mut c_int as *mut c_void) };
        self.api.check(code, name)
    }

    pub fn observe(&self, name: &str, format: c_int) -> Result<(), MpvError> {
        let n = cstring(name)?;
        // SAFETY: valid handle; mpv copies the name.
        let code = unsafe { (self.api.observe_property)(self.handle, 0, n.as_ptr(), format) };
        self.api.check(code, name)
    }

    /// Calls `f` whenever mpv has an event waiting. `f` runs on one of
    /// mpv's threads and must only signal.
    pub fn on_wakeup(&mut self, f: impl Fn() + Send + Sync + 'static) {
        let boxed: *mut Nudge = Box::into_raw(Box::new(Box::new(f)));
        // SAFETY: `boxed` stays valid until `Drop`, which unsets the
        // callback before freeing it.
        unsafe { (self.api.set_wakeup_callback)(self.handle, Some(nudge), boxed as *mut c_void) };
        if let Some(old) = self.wakeup.replace(boxed) {
            // SAFETY: the old closure was replaced above and mpv no longer
            // holds it.
            drop(unsafe { Box::from_raw(old) });
        }
    }

    /// The next event, or `Happened::Nothing` if none arrives within
    /// `timeout` seconds (0 polls).
    pub fn next_event(&self, timeout: f64) -> Happened {
        // SAFETY: the returned event is owned by mpv and valid until the
        // next wait_event; everything needed is copied out before return.
        let event = unsafe { &*(self.api.wait_event)(self.handle, timeout) };
        match event.event_id {
            EVENT_NONE => Happened::Nothing,
            EVENT_SHUTDOWN => Happened::Shutdown,
            EVENT_FILE_LOADED => Happened::FileLoaded,
            EVENT_END_FILE => Happened::EndFile { error: event.error < 0 },
            EVENT_PROPERTY_CHANGE if !event.data.is_null() => {
                // SAFETY: a property-change event's data is an
                // mpv_event_property, per client.h.
                let prop = unsafe { &*(event.data as *const EventProperty) };
                // SAFETY: the name is a NUL-terminated string owned by mpv.
                let name = unsafe { CStr::from_ptr(prop.name) }.to_string_lossy().into_owned();
                if prop.data.is_null() {
                    return Happened::Other;
                }
                // SAFETY: `data` points at a value of `format`, per client.h.
                unsafe {
                    match prop.format {
                        FORMAT_DOUBLE => Happened::Double(name, *(prop.data as *const f64)),
                        FORMAT_FLAG => Happened::Flag(name, *(prop.data as *const c_int) != 0),
                        FORMAT_INT64 => Happened::Int(name, *(prop.data as *const i64)),
                        _ => Happened::Other,
                    }
                }
            }
            _ => Happened::Other,
        }
    }

    /// A software renderer for this player's video. mpv draws nothing
    /// until one exists (`vo=libmpv`).
    pub fn software_renderer(&self) -> Result<Render, MpvError> {
        let mut ctx: *mut RenderContext = std::ptr::null_mut();
        let api_type = cstring("sw")?;
        let mut params = [
            RenderParam { kind: RENDER_PARAM_API_TYPE, data: api_type.as_ptr() as *mut c_void },
            RenderParam { kind: RENDER_PARAM_INVALID, data: std::ptr::null_mut() },
        ];
        // SAFETY: params is terminated by RENDER_PARAM_INVALID and outlives
        // the call; mpv copies what it keeps.
        let code = unsafe { (self.api.render_context_create)(&mut ctx, self.handle, params.as_mut_ptr()) };
        self.api.check(code, "render context")?;
        Ok(Render { api: self.api.clone(), ctx, update: None })
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        // SAFETY: unset the callback before destroying, then free it; the
        // handle is not used again.
        unsafe {
            (self.api.set_wakeup_callback)(self.handle, None, std::ptr::null_mut());
            (self.api.terminate_destroy)(self.handle);
            if let Some(boxed) = self.wakeup.take() {
                drop(Box::from_raw(boxed));
            }
        }
    }
}

/// mpv's software renderer: draws the current frame into memory.
///
/// Must be dropped before the [`Mpv`] it came from — client.h requires the
/// render context be freed before the handle is destroyed. The player
/// keeps them in that order.
pub struct Render {
    api: Arc<Api>,
    ctx: *mut RenderContext,
    update: Option<*mut Nudge>,
}

// SAFETY: a render context may be used from any one thread at a time;
// the player owns it on its own thread.
unsafe impl Send for Render {}

impl Render {
    /// Calls `f` whenever a new frame is ready to draw.
    pub fn on_update(&mut self, f: impl Fn() + Send + Sync + 'static) {
        let boxed: *mut Nudge = Box::into_raw(Box::new(Box::new(f)));
        // SAFETY: as for `Mpv::on_wakeup`.
        unsafe { (self.api.render_context_set_update_callback)(self.ctx, Some(nudge), boxed as *mut c_void) };
        if let Some(old) = self.update.replace(boxed) {
            // SAFETY: replaced above; mpv no longer holds it.
            drop(unsafe { Box::from_raw(old) });
        }
    }

    /// Whether a new frame is waiting to be drawn.
    pub fn frame_waiting(&self) -> bool {
        // SAFETY: valid context, on its owning thread.
        unsafe { (self.api.render_context_update)(self.ctx) & RENDER_UPDATE_FRAME != 0 }
    }

    /// Draws the current frame, scaled and letterboxed by mpv, into
    /// `target` as `rgb0` rows of `stride` bytes.
    ///
    /// `target` must hold `stride * height` bytes and be 64-byte aligned,
    /// and `stride` a multiple of 64 — mpv's own advice for its fast path,
    /// and what `frame::Buffer` provides.
    pub fn draw(&self, target: &mut [u8], width: u32, height: u32, stride: usize) -> Result<(), MpvError> {
        if target.len() < stride * height as usize || stride < width as usize * 4 {
            return Err(MpvError::Failed("render target too small".into()));
        }
        let mut size: [c_int; 2] = [width as c_int, height as c_int];
        let format = cstring("rgb0")?;
        let mut stride_value: usize = stride;
        let mut params = [
            RenderParam { kind: RENDER_PARAM_SW_SIZE, data: size.as_mut_ptr() as *mut c_void },
            RenderParam { kind: RENDER_PARAM_SW_FORMAT, data: format.as_ptr() as *mut c_void },
            RenderParam { kind: RENDER_PARAM_SW_STRIDE, data: &mut stride_value as *mut usize as *mut c_void },
            RenderParam { kind: RENDER_PARAM_SW_POINTER, data: target.as_mut_ptr() as *mut c_void },
            RenderParam { kind: RENDER_PARAM_INVALID, data: std::ptr::null_mut() },
        ];
        // SAFETY: every parameter points at memory that outlives the call
        // and is the type render.h documents for it; the target's size was
        // checked above.
        let code = unsafe { (self.api.render_context_render)(self.ctx, params.as_mut_ptr()) };
        self.api.check(code, "render")
    }
}

impl Drop for Render {
    fn drop(&mut self) {
        // SAFETY: unset the callback, free the context, then the closure.
        unsafe {
            (self.api.render_context_set_update_callback)(self.ctx, None, std::ptr::null_mut());
            (self.api.render_context_free)(self.ctx);
            if let Some(boxed) = self.update.take() {
                drop(Box::from_raw(boxed));
            }
        }
    }
}
