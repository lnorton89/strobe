// ============================================================================
// Module:       strobe_output::spout
// Description:  The Spout sender: the picture shared as a GL texture, through
//               the Spout library found at runtime, with no readback at all.
//
// Dependencies: crate::sink (Sink, Problem), crate::texture (SharedTexture),
//               crate::Frame; libloading and the Spout library on Windows
// ============================================================================

//! Spout: the GPU-to-GPU path (D570, D572).
//!
//! Spout shares a Direct3D 11 texture with another program on the same
//! machine; a GL renderer hands its texture to Spout's interop layer, which
//! copies GPU-to-GPU. Nothing is read back, nothing is encoded, and the cost
//! is the interop copy and nothing else. That is why it is the Windows path
//! of choice for a VJ rig: Resolume, TouchDesigner, OBS and Notch all speak
//! it.
//!
//! **The library.** `SpoutLibrary.dll` as Spout ships it (2.007 and later)
//! exports one symbol, `GetSpout`, which returns a `SPOUTLIBRARY*`: an object
//! whose first word points at a vtable of 175 virtual calls, the shape
//! `SpoutLibrary/include/SpoutLibrary/SpoutLibrary.h` declares. There is no
//! C ABI and no import library, and the class's own methods are not exported
//! by name, so the object is kept as an address, its vtable is read once, and
//! the three calls the sender needs are taken from their declared slots
//! (D572). A library whose vtable layout differs is not detected here: Spout
//! has kept these three slots since 2.006, and the calls below are the
//! earliest ones it declares, ahead of every conditional section of the
//! header.
//!
//! **The thread.** `SendTexture` registers the GL texture with Direct3D on
//! the calling thread, so it must be called where the GL context is current
//! — the app's render thread — and the sender must not be moved to another
//! one. The object's address is kept as a `usize`, not a `*mut`, so the type
//! stays `Send` for the [`Sink`] trait; nothing else may use it off that
//! thread.
//!
//! **The size.** A sender's size is fixed when it is created or updated, so
//! the picture's size arrives with the first frame: the sender is created
//! then, and a resize creates it again. That also means a sender exists only
//! once a frame has been shared, which is when a receiver can first find it.
//!
//! **The flip.** Spout's shared texture is top-left first and a GL target is
//! bottom-left first, so the picture is sent inverted (`bInvert`), which is
//! what other GL engines do. If a receiver shows it upside down, that flag is
//! the one thing to change, and it is a constant with its reason here rather
//! than a setting nobody understands.

use crate::Frame;
use crate::sink::{Problem, Sink};
use crate::texture::SharedTexture;

/// The Spout library's own signatures and vtable slots, for the calls
/// `spout::imp` makes (D572). File scope, above every function: the
/// conventions check reads a module-level block as part of the nearest
/// function above it, and these are the shapes the operations take, not
/// operations.
#[cfg(windows)]
mod abi {
    use std::ffi::{c_char, c_void};

    /// `SPOUTHANDLE GetSpout(VOID)`: the library's only export, the factory
    /// for its one object.
    pub(super) const GET_SPOUT: &[u8] = b"GetSpout";

    /// `void ReleaseSender(DWORD)`: slot 2.
    pub(super) const RELEASE_SENDER: usize = 2;

    /// `bool SendTexture(GLuint, GLuint, unsigned, unsigned, bool, GLuint)`:
    /// slot 4.
    pub(super) const SEND_TEXTURE: usize = 4;

    /// `bool IsInitialized()`: slot 6, the sender's own word on whether it
    /// exists.
    pub(super) const IS_INITIALIZED: usize = 6;

    /// `const char* GetName()`: slot 8.
    pub(super) const GET_NAME: usize = 8;

    /// `unsigned GetWidth()`: slot 9.
    pub(super) const GET_WIDTH: usize = 9;

    /// `unsigned GetHeight()`: slot 10.
    pub(super) const GET_HEIGHT: usize = 10;

    /// `double GetFps()`: slot 11, the rate the library measures from the
    /// frames it is given, which is how many frames are really arriving.
    pub(super) const GET_FPS: usize = 11;

    /// `bool CreateSender(const char*, unsigned, unsigned, DWORD)`: slot 70.
    ///
    /// The SDK's header lists 69; the library Spout 2.007.018 ships puts it
    /// one slot later, and 70 is the slot the vendor's own compiled calls
    /// dispatch to (read from `cl`-built code, not from the header).
    pub(super) const CREATE_SENDER: usize = 70;

    /// `SPOUTHANDLE GetSpout(VOID)`.
    pub(super) type GetSpoutFn = unsafe extern "system" fn() -> *mut c_void;

    /// A virtual call with `this` first, as the x64 ABI passes it.
    pub(super) type CreateSenderFn =
        unsafe extern "system" fn(*mut c_void, *const c_char, u32, u32, u32) -> bool;

    /// A virtual call with `this` first, as the x64 ABI passes it.
    pub(super) type SendTextureFn =
        unsafe extern "system" fn(*mut c_void, u32, u32, u32, u32, bool, u32) -> bool;

    /// A virtual call with `this` first, as the x64 ABI passes it.
    pub(super) type ReleaseSenderFn = unsafe extern "system" fn(*mut c_void, u32);

    /// A virtual call with `this` first, as the x64 ABI passes it.
    pub(super) type FlagFn = unsafe extern "system" fn(*mut c_void) -> bool;

    /// A virtual call with `this` first, as the x64 ABI passes it.
    pub(super) type CountFn = unsafe extern "system" fn(*mut c_void) -> u32;

    /// A virtual call with `this` first, as the x64 ABI passes it.
    pub(super) type TextFn = unsafe extern "system" fn(*mut c_void) -> *const c_char;

    /// A virtual call with `this` first, answering a frame rate.
    pub(super) type RealFn = unsafe extern "system" fn(*mut c_void) -> f64;
}

/// A Spout sender: one named destination, sharing the renderer's texture.
#[derive(Debug)]
pub struct SpoutSender {
    name: String,
    inner: imp::Sender,
}

impl SpoutSender {
    /// Opens a sender called `name`, which is what a receiver lists.
    ///
    /// # Errors
    ///
    /// The library missing, or not one Strobe can call.
    pub fn open(name: &str) -> Result<Self, Problem> {
        if name.trim().is_empty() {
            return Err(Problem::new(
                "a Spout sender needs a name; that is what a receiver lists",
            ));
        }
        Ok(Self {
            name: name.to_owned(),
            inner: imp::Sender::open(name)?,
        })
    }

    /// The name a receiver sees.
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Sink for SpoutSender {
    fn send(&mut self, _frame: &Frame) -> Result<(), Problem> {
        Err(Problem::new(
            "a Spout sender shares the texture, not a frame of pixels",
        ))
    }

    fn send_texture(&mut self, texture: SharedTexture) -> Result<(), Problem> {
        self.inner.send_texture(texture)
    }

    fn close(&mut self) {
        self.inner.close();
    }
}

/// The platform's Spout sender: the library's vtable on Windows, and a
/// reasoned failure everywhere else (Spout is Direct3D, Windows only).
#[cfg(not(windows))]
mod imp {
    use super::{Problem, SharedTexture};

    /// No Spout here.
    #[derive(Debug)]
    pub(super) struct Sender;

    impl Sender {
        /// Always fails: Spout is Windows only.
        pub(super) fn open(_name: &str) -> Result<Self, Problem> {
            Err(Problem::new(
                "Spout shares Direct3D textures and exists only on Windows",
            ))
        }

        /// Never reached.
        pub(super) fn send_texture(&mut self, _texture: SharedTexture) -> Result<(), Problem> {
            Err(Problem::new("Spout is Windows only"))
        }

        /// Nothing to release.
        pub(super) fn close(&mut self) {}
    }
}

/// The library's object and vtable (D572). Every call into the DLL is one
/// named, private, safe function holding one FFI block, as the GL boundary is
/// (`strobe-conventions`'s unsafe boundary names this file).
#[cfg(windows)]
#[expect(unsafe_code, reason = "the Spout library has no safe Rust binding")]
mod imp {
    use super::abi::{
        CREATE_SENDER, CountFn, CreateSenderFn, FlagFn, GET_FPS, GET_HEIGHT, GET_NAME, GET_SPOUT,
        GET_WIDTH, GetSpoutFn, IS_INITIALIZED, RELEASE_SENDER, RealFn, ReleaseSenderFn,
        SEND_TEXTURE, SendTextureFn, TextFn,
    };
    use super::{Problem, SharedTexture};
    use libloading::Library;
    use std::ffi::{CStr, CString, c_char, c_void};
    use std::path::Path;

    /// Whether the picture is flipped on the way out: Spout's shared texture
    /// is top-left first, a GL target bottom-left first.
    const INVERT: bool = true;

    /// The `dwFormat` a sender passes when the shared texture's format is
    /// Spout's own choice (`DXGI_FORMAT_UNKNOWN`).
    const FORMAT_DEFAULT: u32 = 0;

    /// The millisecond count `ReleaseSender` takes to wait: none.
    const RELEASE_NOW: u32 = 0;

    /// The host framebuffer for the interop: the default one.
    const HOST_FBO: u32 = 0;

    /// A sender through one open library.
    #[derive(Debug)]
    pub(super) struct Sender {
        /// Kept alive for the whole sender: its vtable points into it.
        _library: Library,
        /// The `SPOUTLIBRARY` object's address. See the module note: only the
        /// thread that opened it, with its GL context, may call into it.
        object: usize,
        /// `CreateSender` from the object's vtable.
        create: CreateSenderFn,
        /// `SendTexture` from the object's vtable.
        send: SendTextureFn,
        /// `ReleaseSender` from the object's vtable.
        release: ReleaseSenderFn,
        /// `IsInitialized`: Spout's own word on whether the sender exists.
        initialized: FlagFn,
        /// `GetName`: the name Spout says it is publishing under.
        name_of: TextFn,
        /// `GetWidth`: the width Spout says the sender has.
        width_of: CountFn,
        /// `GetHeight`: the height Spout says the sender has.
        height_of: CountFn,
        /// `GetFps`: the rate Spout measures from the frames it receives.
        fps_of: RealFn,
        /// Whether the measured rate has been reported.
        fps_reported: bool,
        /// The name, NUL-terminated, kept for a resize's `CreateSender`.
        name: CString,
        /// The size the sender was created with, once a frame has been sent.
        size: Option<(u32, u32)>,
        /// Whether the sender exists and has not been released.
        open: bool,
    }

    impl Sender {
        /// Finds the library and takes the three calls from the object it
        /// hands out. No sender exists until the first frame gives it a size.
        pub(super) fn open(name: &str) -> Result<Self, Problem> {
            let path = crate::probe::spout_library().ok_or_else(|| {
                Problem::new(
                    "Spout output needs SpoutLibrary.dll, which was not found beside Strobe or on \
                     the PATH; install Spout from spout.zeal.co",
                )
            })?;
            let library = load_library(&path)?;
            let get_spout: GetSpoutFn = export(&library, GET_SPOUT)?;
            let object = spout_object(get_spout);
            if object.is_null() {
                return Err(Problem::new(
                    "the Spout library did not return an object to call",
                ));
            }
            let vtable = vtable_of(object);
            let create = method(vtable, CREATE_SENDER);
            let send = method(vtable, SEND_TEXTURE);
            let release = method(vtable, RELEASE_SENDER);
            let initialized = method(vtable, IS_INITIALIZED);
            let name_of = method(vtable, GET_NAME);
            let width_of = method(vtable, GET_WIDTH);
            let height_of = method(vtable, GET_HEIGHT);
            let fps_of = method(vtable, GET_FPS);
            let name = CString::new(name)
                .map_err(|_| Problem::new("a Spout sender's name cannot contain a zero byte"))?;
            Ok(Self {
                _library: library,
                object: object as usize,
                create,
                send,
                release,
                initialized,
                name_of,
                width_of,
                height_of,
                fps_of,
                fps_reported: false,
                name,
                size: None,
                open: true,
            })
        }

        /// Shares one frame of the renderer's texture, creating the sender on
        /// the first frame and again when the picture's size changes.
        pub(super) fn send_texture(&mut self, texture: SharedTexture) -> Result<(), Problem> {
            if !self.open {
                return Err(Problem::new("the Spout sender is closed"));
            }
            let size = (texture.width, texture.height);
            if !texture.is_whole() {
                return Err(Problem::new(
                    "the picture has no texture, or no size, to share yet",
                ));
            }
            if self.size != Some(size) {
                let created = make_sender(
                    self.create,
                    self.object as *mut c_void,
                    &self.name,
                    size.0,
                    size.1,
                );
                if !created {
                    return Err(Problem::new(
                        "Spout refused the sender; the name may be in use by another program, or \
                         this GPU has no Direct3D interop",
                    ));
                }
                self.size = Some(size);
                // Spout's own view, not ours: if it says nothing is live, the
                // picture is not being shared whatever the call returned.
                if !live(self.initialized, self.object as *mut c_void) {
                    self.size = None;
                    return Err(Problem::new(
                        "Spout accepted the sender but reports nothing live; the GPU may have no                          Direct3D/OpenGL interop, and SpoutSettings says what this machine                          supports",
                    ));
                }
                log::info!(
                    "Spout reports sender `{}` live at {}x{}; a receiver can list it",
                    name_at(self.name_of, self.object as *mut c_void),
                    measure(self.width_of, self.object as *mut c_void),
                    measure(self.height_of, self.object as *mut c_void),
                );
            }
            if share_texture(
                self.send,
                self.object as *mut c_void,
                (texture.id, texture.target, size.0, size.1),
            ) {
                // Spout counts and times the frames it is handed, so its rate
                // is the proof that the picture is arriving, not ours.
                if !self.fps_reported {
                    let fps = rate(self.fps_of, self.object as *mut c_void);
                    if fps > 0.0 {
                        log::info!("Spout is carrying the picture at {fps:.1} fps");
                        self.fps_reported = true;
                    }
                }
                Ok(())
            } else {
                Err(Problem::new(
                    "Spout could not share the texture; the Direct3D interop or the sender was \
                     lost",
                ))
            }
        }

        /// Releases the sender, so it disappears from a receiver's list.
        pub(super) fn close(&mut self) {
            if !self.open || self.size.is_none() {
                self.open = false;
                return;
            }
            release(self.release, self.object as *mut c_void);
            self.size = None;
            self.open = false;
        }
    }

    impl Drop for Sender {
        fn drop(&mut self) {
            self.close();
        }
    }

    /// Opens the library at `path`.
    fn load_library(path: &Path) -> Result<Library, Problem> {
        // SAFETY: loading a DLL runs its initialisation, which is what any
        // program that uses Spout does. It is loaded by the whole path the
        // probe found, so no other folder's copy can be picked up, and the
        // `Library` outlives every call made through it.
        let library = unsafe { Library::new(path) };
        library.map_err(|error| {
            Problem::new(format!(
                "could not open {}: {error}; Spout needs a 64-bit library",
                path.display()
            ))
        })
    }

    /// One export of `library`, as the type its call needs.
    fn export<T: Copy>(library: &Library, name: &[u8]) -> Result<T, Problem> {
        // SAFETY: the caller's type is written to match the library's
        // documented signature for that exact symbol, and a symbol that is
        // absent is an error, never a call through a wrong pointer.
        let symbol = unsafe { library.get::<T>(name) };
        match symbol {
            Ok(symbol) => Ok(*symbol),
            Err(_) => Err(Problem::new(format!(
                "this SpoutLibrary.dll is not one Strobe can call (no `{}` export); take it from \
                 the Spout 2 release, which exports it",
                String::from_utf8_lossy(name)
            ))),
        }
    }

    /// The library's one `SPOUTLIBRARY` object.
    fn spout_object(get_spout: GetSpoutFn) -> *mut c_void {
        // SAFETY: `GetSpout` returns the object the library builds on first
        // call and keeps for the process; the pointer is only ever passed
        // back to that object's own calls.
        unsafe { get_spout() }
    }

    /// The object's vtable: its first word, as `SpoutLibrary.h` declares a
    /// COM-like interface.
    fn vtable_of(object: *mut c_void) -> *const usize {
        // SAFETY: `object` is the non-null object `GetSpout` returned, and a
        // COM-like interface begins with its vtable pointer. The read is of
        // that one word, which lives as long as the library does.
        unsafe { *(object as *const *const usize) }
    }

    /// The virtual call at `slot`.
    fn method<T: Copy>(vtable: *const usize, slot: usize) -> T {
        // SAFETY: the vtable holds as many entries as the header declares and
        // `slot` is one of its constants; each entry is a function pointer of
        // the type the header gives that call, size-checked by `T` being a
        // pointer-sized fn type.
        unsafe { std::mem::transmute_copy(&*vtable.add(slot)) }
    }

    /// Creates or resizes the sender: `CreateSender(name, width, height,
    /// format)`.
    fn make_sender(
        create: CreateSenderFn,
        object: *mut c_void,
        name: &CString,
        width: u32,
        height: u32,
    ) -> bool {
        // SAFETY: `object` is the library's own, `name` is a valid
        // NUL-terminated string that outlives the call, and the remaining
        // arguments are plain numbers the library validates. Run on the
        // render thread with the GL context current, which Spout's interop
        // requires.
        unsafe { create(object, name.as_ptr(), width, height, FORMAT_DEFAULT) }
    }

    /// Shares `texture`: `SendTexture(id, target, width, height, invert,
    /// host)`.
    fn share_texture(
        send: SendTextureFn,
        object: *mut c_void,
        texture: (u32, u32, u32, u32),
    ) -> bool {
        let (id, target, width, height) = texture;
        // SAFETY: `object` is the library's own, still alive because the
        // library and the process are; the texture belongs to the renderer's
        // live target and the calling thread has its GL context current,
        // which is the contract `SendTexture` needs.
        unsafe { send(object, id, target, width, height, INVERT, HOST_FBO) }
    }

    /// Whether Spout says the sender exists.
    fn live(f: FlagFn, object: *mut c_void) -> bool {
        // SAFETY: the object is the library's own and `IsInitialized` only
        // reports the sender's state; it touches nothing the caller owns.
        unsafe { f(object) }
    }

    /// What Spout says the sender is called.
    fn name_at(f: TextFn, object: *mut c_void) -> String {
        text_of(name_of(f, object))
    }

    /// The library's string for the sender's name.
    fn name_of(f: TextFn, object: *mut c_void) -> *const c_char {
        // SAFETY: the object is the library's own and `GetName` returns a
        // pointer into its own storage, which outlives the call.
        unsafe { f(object) }
    }

    /// A C string the library owns, copied out; empty when it has none.
    fn text_of(text: *const c_char) -> String {
        if text.is_null() {
            return String::new();
        }
        // SAFETY: `GetName` returns a NUL-terminated string the library keeps,
        // read here and copied before it can change.
        unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned()
    }

    /// The rate Spout measures for the sender.
    fn rate(f: RealFn, object: *mut c_void) -> f64 {
        // SAFETY: the object is the library's own and `GetFps` only reports
        // the rate it has measured.
        unsafe { f(object) }
    }

    /// A size Spout reports for the sender.
    fn measure(f: CountFn, object: *mut c_void) -> u32 {
        // SAFETY: the object is the library's own; `GetWidth` and
        // `GetHeight` only report the sender's size.
        unsafe { f(object) }
    }

    /// Releases the sender.
    fn release(f: ReleaseSenderFn, object: *mut c_void) {
        // SAFETY: as for `share_texture`: the object is the library's own and
        // `ReleaseSender` only tears down the sender it built.
        unsafe { f(object, RELEASE_NOW) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn a_sender_needs_a_name() -> Result<()> {
        let error = match SpoutSender::open("   ") {
            Ok(_) => anyhow::bail!("an unnamed sender was created"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains("needs a name"),
            "the refusal says what is missing: {error}"
        );
        Ok(())
    }
}
