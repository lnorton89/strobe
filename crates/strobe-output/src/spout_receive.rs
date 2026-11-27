// ============================================================================
// Module:       strobe_output::spout_receive
// Description:  The Spout receiver: the senders this machine lists, and one
//               sender's picture read into a reused buffer on a thread with
//               Spout's own hidden GL context.
//
// Dependencies: crate::receive (Received, Order), crate::sink (Problem),
//               libloading and the Spout library on Windows
// ============================================================================

//! Spout input (D575): another program's texture behind `syn_Media`.
//!
//! **The calls.** The same `SpoutLibrary.dll` and the same object as the
//! sender ([`crate::spout`], D572), with the receiver's slots of its vtable.
//! Each slot is the one `cl` emits for a call through the 2.007.018 header
//! (`call [rax+8*slot]`), read from a listing of a probe compiled against it,
//! not counted: MSVC groups a virtual's overloads, so counting is wrong past
//! the first overload (D573).
//!
//! **The thread.** The receiver lives on the capture thread that opened it,
//! never the render thread: a read there would be a GPU round trip in the
//! frame. `CreateOpenGL` gives that thread Spout's own hidden window and GL
//! context, which `ReceiveImage` reads through (and which `CloseOpenGL` tears
//! down). The object's address is kept as a raw pointer, so the type is not
//! `Send` and cannot leave that thread.
//!
//! **The picture.** `ReceiveImage` copies the sender's newest frame into the
//! caller's buffer as RGBA, top row first (the shared texture's own order),
//! and the capture thread turns it over while converting, as it does a
//! camera's. Each read first connects with `ReceiveTexture` and no texture,
//! which copies nothing but refreshes the sender's size; the buffer is made
//! that size, and only then does `ReceiveImage` write into it. A size that
//! changes between the two is flagged by `IsUpdated` and written nowhere, so
//! Spout never writes past the buffer. `IsFrameNew` says whether a read
//! brought a new frame, when the sender counts frames (Spout's default).
//!
//! This is a readback and an upload per frame, the cost a camera has. A
//! zero-copy path (`ReceiveTexture` into the player's texture on the render
//! thread) is the later step D575 records.

use crate::receive::Received;
use crate::sink::Problem;

/// The Spout library's receiver slots and signatures (D575), file scope as in
/// [`crate::spout`].
#[cfg(windows)]
mod abi {
    use std::ffi::{c_char, c_void};

    /// `void SetReceiverName(const char*)`: slot 16, the sender to connect to.
    pub(super) const SET_RECEIVER_NAME: usize = 16;

    /// `void ReleaseReceiver()`: slot 18.
    pub(super) const RELEASE_RECEIVER: usize = 18;

    /// `bool ReceiveTexture(GLuint, GLuint, bool, GLuint)`: slot 19. With no
    /// texture it connects and reads the sender's details, copying nothing.
    pub(super) const RECEIVE_TEXTURE: usize = 19;

    /// `bool ReceiveImage(unsigned char*, GLenum, bool, GLuint)`: slot 20.
    pub(super) const RECEIVE_IMAGE: usize = 20;

    /// `bool IsUpdated()`: slot 21, true once after the sender's size
    /// changed (and on the first connection), when nothing was written.
    pub(super) const IS_UPDATED: usize = 21;

    /// `bool IsFrameNew()`: slot 23.
    pub(super) const IS_FRAME_NEW: usize = 23;

    /// `unsigned GetSenderWidth()`: slot 25.
    pub(super) const GET_SENDER_WIDTH: usize = 25;

    /// `unsigned GetSenderHeight()`: slot 26.
    pub(super) const GET_SENDER_HEIGHT: usize = 26;

    /// `int GetSenderCount()`: slot 58.
    pub(super) const GET_SENDER_COUNT: usize = 58;

    /// `bool GetSender(int, char*, int)`: slot 59.
    pub(super) const GET_SENDER: usize = 59;

    /// `bool CreateOpenGL(HWND)`: slot 98, Spout's hidden window and context.
    pub(super) const CREATE_OPENGL: usize = 98;

    /// `bool CloseOpenGL()`: slot 99.
    pub(super) const CLOSE_OPENGL: usize = 99;

    /// A virtual call with `this` first, as the x64 ABI passes it.
    pub(super) type NameFn = unsafe extern "system" fn(*mut c_void, *const c_char);

    /// A virtual call with `this` first.
    pub(super) type VoidFn = unsafe extern "system" fn(*mut c_void);

    /// A virtual call with `this` first.
    pub(super) type ReceiveImageFn =
        unsafe extern "system" fn(*mut c_void, *mut u8, u32, bool, u32) -> bool;

    /// A virtual call with `this` first.
    pub(super) type ConnectFn = unsafe extern "system" fn(*mut c_void, u32, u32, bool, u32) -> bool;

    /// A virtual call with `this` first.
    pub(super) type CountFn = unsafe extern "system" fn(*mut c_void) -> i32;

    /// A virtual call with `this` first.
    pub(super) type GetSenderFn =
        unsafe extern "system" fn(*mut c_void, i32, *mut c_char, i32) -> bool;

    /// A virtual call with `this` first.
    pub(super) type OpenGlFn = unsafe extern "system" fn(*mut c_void, *mut c_void) -> bool;

    /// `SPOUTHANDLE GetSpout(VOID)`: the library's only export.
    pub(super) const GET_SPOUT: &[u8] = b"GetSpout";

    /// `void Release()`: slot 174, deletes the object `GetSpout` built.
    pub(super) const RELEASE: usize = 174;

    /// `SPOUTHANDLE GetSpout(VOID)`.
    pub(super) type GetSpoutFn = unsafe extern "system" fn() -> *mut c_void;

    /// A virtual call with `this` first, returning nothing.
    pub(super) type ReleaseFn = unsafe extern "system" fn(*mut c_void);

    /// A virtual call with `this` first, answering yes or no.
    pub(super) type FlagFn = unsafe extern "system" fn(*mut c_void) -> bool;

    /// A virtual call with `this` first, answering a size.
    pub(super) type SizeFn = unsafe extern "system" fn(*mut c_void) -> u32;
}

/// The Spout senders this machine lists now, by name.
///
/// # Errors
///
/// No Spout library, or one Strobe cannot call (Windows only).
pub fn spout_senders() -> Result<Vec<String>, Problem> {
    imp::senders()
}

/// A Spout receiver connected to one sender by name, on the thread that
/// opened it.
#[derive(Debug)]
pub struct SpoutReceiver {
    inner: imp::Receiver,
}

impl SpoutReceiver {
    /// Opens a receiver for the sender called `name`. Whether that sender
    /// exists is learned by the first [`Self::receive`].
    ///
    /// # Errors
    ///
    /// No Spout library, or no GL context for it on this thread.
    pub fn open(name: &str) -> Result<Self, Problem> {
        if name.trim().is_empty() {
            return Err(Problem::new("a Spout receiver needs a sender's name"));
        }
        Ok(Self {
            inner: imp::Receiver::open(name)?,
        })
    }

    /// Reads the sender's newest frame into `pixels`, RGBA top row first,
    /// resizing it only when the sender's size changes. `Ok(None)` when no
    /// new frame came since the last read (or the size just changed and the
    /// next read fills the resized buffer).
    ///
    /// # Errors
    ///
    /// The sender is not running, or closed.
    pub fn receive(&mut self, pixels: &mut Vec<u8>) -> Result<Option<Received>, Problem> {
        self.inner.receive(pixels)
    }
}

/// No Spout off Windows.
#[cfg(not(windows))]
mod imp {
    use super::{Problem, Received};

    /// Never opened.
    #[derive(Debug)]
    pub(super) struct Receiver;

    fn unsupported() -> Problem {
        Problem::new("Spout shares Direct3D textures and exists only on Windows")
    }

    pub(super) fn senders() -> Result<Vec<String>, Problem> {
        Err(unsupported())
    }

    impl Receiver {
        pub(super) fn open(_name: &str) -> Result<Self, Problem> {
            Err(unsupported())
        }

        pub(super) fn receive(
            &mut self,
            _pixels: &mut Vec<u8>,
        ) -> Result<Option<Received>, Problem> {
            Err(unsupported())
        }
    }
}

/// The receiver through the library's vtable. Every call into the DLL is one
/// named, private, safe function holding one FFI block (D572).
#[cfg(windows)]
#[expect(unsafe_code, reason = "the Spout library has no safe Rust binding")]
mod imp {
    use super::abi::{
        CLOSE_OPENGL, CREATE_OPENGL, ConnectFn, CountFn, FlagFn, GET_SENDER, GET_SENDER_COUNT,
        GET_SENDER_HEIGHT, GET_SENDER_WIDTH, GET_SPOUT, GetSenderFn, GetSpoutFn, IS_FRAME_NEW,
        IS_UPDATED, NameFn, OpenGlFn, RECEIVE_IMAGE, RECEIVE_TEXTURE, RELEASE, RELEASE_RECEIVER,
        ReceiveImageFn, ReleaseFn, SET_RECEIVER_NAME, SizeFn, VoidFn,
    };
    use super::{Problem, Received};
    use crate::receive::Order;
    use libloading::Library;
    use std::ffi::{CStr, CString, c_char, c_void};
    use std::path::Path;

    /// `GL_RGBA`, the layout `ReceiveImage` is asked for.
    const GL_RGBA: u32 = 0x1908;

    /// Rows are read as the shared texture holds them, top first; the capture
    /// thread turns them over.
    const INVERT: bool = false;

    /// The host framebuffer `ReceiveImage` restores: the default one.
    const HOST_FBO: u32 = 0;

    /// The widest or tallest picture taken from a sender: a guard against a
    /// size read wrong, far above any texture a GPU would share.
    const MAX_SIDE: u32 = 16_384;

    /// The longest sender name Spout stores, NUL included.
    const NAME_BYTES: usize = 256;

    /// The library and one object built by it, released together.
    struct Object {
        /// Kept loaded for as long as the object: its vtable points into it.
        _library: Library,
        object: *mut c_void,
        vtable: *const usize,
        destroy: ReleaseFn,
    }

    impl Object {
        fn new() -> Result<Self, Problem> {
            let path = crate::probe::spout_library().ok_or_else(|| {
                Problem::new(
                    "Spout input needs SpoutLibrary.dll, which was not found beside Strobe or on \
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
            Ok(Self {
                _library: library,
                object,
                vtable,
                destroy: method(vtable, RELEASE),
            })
        }

        fn call<T: Copy>(&self, slot: usize) -> T {
            method(self.vtable, slot)
        }
    }

    impl Drop for Object {
        fn drop(&mut self) {
            // `Drop` runs ahead of the fields, so the library is still loaded.
            destroy(self.destroy, self.object);
        }
    }

    pub(super) fn senders() -> Result<Vec<String>, Problem> {
        let object = Object::new()?;
        let count = sender_count(object.call(GET_SENDER_COUNT), object.object).max(0);
        let get: GetSenderFn = object.call(GET_SENDER);
        let mut names = Vec::new();
        let mut buffer = [0 as c_char; NAME_BYTES];
        for index in 0..count {
            if sender_name(get, object.object, index, &mut buffer) {
                let name = text_of(buffer.as_ptr());
                if !name.is_empty() {
                    names.push(name);
                }
            }
        }
        Ok(names)
    }

    /// A receiver connected to one sender, on the thread that opened it.
    pub(super) struct Receiver {
        object: Object,
        connect: ConnectFn,
        receive: ReceiveImageFn,
        updated: FlagFn,
        frame_new: FlagFn,
        width_of: SizeFn,
        height_of: SizeFn,
        /// The sender's size the buffer was last made for.
        size: Option<(u32, u32)>,
        /// Whether a read has ever found the sender, for the failure's words.
        found: bool,
        /// The buffer was made for a new size and no picture has been handed
        /// over at it yet: the next whole read is, new frame or not, so a
        /// paused sender still shows rather than reading as stalled.
        unshown: bool,
    }

    impl std::fmt::Debug for Receiver {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Receiver")
                .field("size", &self.size)
                .finish_non_exhaustive()
        }
    }

    impl Receiver {
        pub(super) fn open(name: &str) -> Result<Self, Problem> {
            let name = CString::new(name)
                .map_err(|_| Problem::new("a Spout sender's name cannot contain a zero byte"))?;
            let object = Object::new()?;
            if !open_gl(object.call(CREATE_OPENGL), object.object) {
                return Err(Problem::new(
                    "Spout could not make an OpenGL context to receive with; the graphics driver \
                     may be missing",
                ));
            }
            set_name(object.call(SET_RECEIVER_NAME), object.object, &name);
            Ok(Self {
                connect: object.call(RECEIVE_TEXTURE),
                receive: object.call(RECEIVE_IMAGE),
                updated: object.call(IS_UPDATED),
                frame_new: object.call(IS_FRAME_NEW),
                width_of: object.call(GET_SENDER_WIDTH),
                height_of: object.call(GET_SENDER_HEIGHT),
                object,
                size: None,
                found: false,
                unshown: true,
            })
        }

        pub(super) fn receive(
            &mut self,
            pixels: &mut Vec<u8>,
        ) -> Result<Option<Received>, Problem> {
            let object = self.object.object;
            // Connect and learn the sender's size without reading: with no
            // texture, `ReceiveTexture` writes nothing.
            if !connect(self.connect, object) {
                self.size = None;
                return Err(Problem::new(if self.found {
                    "the sender closed (end of stream)"
                } else {
                    "not found: no Spout sender of that name is running"
                }));
            }
            self.found = true;
            if flag(self.updated, object) || self.size.is_none() {
                self.fit(pixels)?;
            }
            let Some((width, height)) = self.size else {
                return Ok(None);
            };
            // The buffer now holds exactly the sender's size. Should the size
            // change between the two calls, Spout writes nothing and flags it.
            if !read_image(self.receive, object, pixels.as_mut_ptr()) {
                self.size = None;
                return Err(Problem::new("the sender closed (end of stream)"));
            }
            if flag(self.updated, object) {
                self.fit(pixels)?;
                return Ok(None);
            }
            if !flag(self.frame_new, object) && !self.unshown {
                return Ok(None);
            }
            self.unshown = false;
            Ok(Some(Received {
                width,
                height,
                order: Order::Rgba,
            }))
        }

        /// Resizes `pixels` to the sender's size as Spout now holds it.
        fn fit(&mut self, pixels: &mut Vec<u8>) -> Result<(), Problem> {
            let object = self.object.object;
            let size = (
                size_of(self.width_of, object),
                size_of(self.height_of, object),
            );
            if size.0 == 0 || size.1 == 0 || size.0 > MAX_SIDE || size.1 > MAX_SIDE {
                self.size = None;
                return Err(Problem::new(format!(
                    "the sender reports a {}x{} picture, which Strobe cannot show",
                    size.0, size.1
                )));
            }
            if self.size != Some(size) {
                // Spout writes nothing for a read with no new frame, as from
                // a paused sender, so a buffer kept from the old size would
                // show its bytes at the new width. Black until a frame comes.
                pixels.clear();
                self.unshown = true;
            }
            pixels.resize(size.0 as usize * size.1 as usize * 4, 0);
            self.size = Some(size);
            Ok(())
        }
    }

    impl Drop for Receiver {
        fn drop(&mut self) {
            let object = self.object.object;
            release_receiver(self.object.call(RELEASE_RECEIVER), object);
            close_gl(self.object.call(CLOSE_OPENGL), object);
            // `object` releases itself, then unloads the library.
        }
    }

    /// `GetSenderCount()`.
    fn sender_count(f: CountFn, object: *mut c_void) -> i32 {
        // SAFETY: the object is the library's own, alive for this call; the
        // count reads Spout's shared sender list and touches nothing of ours.
        unsafe { f(object) }
    }

    /// `GetSender(index, buffer, size)`.
    fn sender_name(
        f: GetSenderFn,
        object: *mut c_void,
        index: i32,
        buffer: &mut [c_char; NAME_BYTES],
    ) -> bool {
        // SAFETY: the object is the library's own; the buffer is NAME_BYTES
        // long and that length is what Spout is told it may write, NUL
        // included.
        unsafe { f(object, index, buffer.as_mut_ptr(), NAME_BYTES as i32) }
    }

    /// `CreateOpenGL(nullptr)`: a hidden window and context, current on the
    /// calling thread.
    fn open_gl(f: OpenGlFn, object: *mut c_void) -> bool {
        // SAFETY: the object is the library's own; a null window asks Spout
        // to make its own, and the context it makes is current only on this
        // thread, which is the only one that uses this receiver.
        unsafe { f(object, std::ptr::null_mut()) }
    }

    /// `CloseOpenGL()`.
    fn close_gl(f: FlagFn, object: *mut c_void) {
        // SAFETY: the object is the library's own, and this runs on the
        // thread whose context `CreateOpenGL` made, once, from `Drop`.
        unsafe { f(object) };
    }

    /// `SetReceiverName(name)`.
    fn set_name(f: NameFn, object: *mut c_void, name: &CString) {
        // SAFETY: the object is the library's own and the name a
        // NUL-terminated string that outlives the call, which copies it.
        unsafe { f(object, name.as_ptr()) };
    }

    /// `ReceiveTexture(0, 0, false, 0)`: connect, and refresh the sender's
    /// details, with nothing copied.
    fn connect(f: ConnectFn, object: *mut c_void) -> bool {
        // SAFETY: the object is the library's own, on the thread whose GL
        // context `CreateOpenGL` made; with texture 0 Spout reads only its
        // shared sender record and writes nothing of ours.
        unsafe { f(object, 0, 0, INVERT, HOST_FBO) }
    }

    /// `ReceiveImage(pixels, GL_RGBA, false, 0)`.
    fn read_image(f: ReceiveImageFn, object: *mut c_void, pixels: *mut u8) -> bool {
        // SAFETY: the object is the library's own, on the thread whose GL
        // context `CreateOpenGL` made. Spout writes `width * height * 4`
        // bytes at the size its receiver holds, which `receive` read just
        // before (after `connect`) and resized `pixels` to; if the sender's
        // size changed since, Spout flags it and writes nothing.
        unsafe { f(object, pixels, GL_RGBA, INVERT, HOST_FBO) }
    }

    /// `IsUpdated()` or `IsFrameNew()`.
    fn flag(f: FlagFn, object: *mut c_void) -> bool {
        // SAFETY: the object is the library's own; both only report state
        // the last read left.
        unsafe { f(object) }
    }

    /// `GetSenderWidth()` or `GetSenderHeight()`.
    fn size_of(f: SizeFn, object: *mut c_void) -> u32 {
        // SAFETY: the object is the library's own; both only report the
        // connected sender's size.
        unsafe { f(object) }
    }

    /// `ReleaseReceiver()`.
    fn release_receiver(f: VoidFn, object: *mut c_void) {
        // SAFETY: the object is the library's own; this disconnects the
        // receiver it built, once, from `Drop`.
        unsafe { f(object) };
    }

    /// Opens the library at `path`.
    fn load_library(path: &Path) -> Result<Library, Problem> {
        // SAFETY: loading a DLL runs its initialisation, as any program that
        // uses Spout does. It is loaded by the whole path the probe found, and
        // the `Library` outlives every call made through it (`Object`).
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
        // SAFETY: the caller's type matches the library's documented
        // signature for that symbol; an absent symbol is an error, never a
        // call through a wrong pointer.
        let symbol = unsafe { library.get::<T>(name) };
        match symbol {
            Ok(symbol) => Ok(*symbol),
            Err(_) => Err(Problem::new(format!(
                "this SpoutLibrary.dll is not one Strobe can call (no `{}` export)",
                String::from_utf8_lossy(name)
            ))),
        }
    }

    /// A new `SPOUTLIBRARY` object.
    fn spout_object(get_spout: GetSpoutFn) -> *mut c_void {
        // SAFETY: `GetSpout` builds and returns an object, freed by its own
        // `Release` from `Object`'s `Drop`; the pointer only goes back to it.
        unsafe { get_spout() }
    }

    /// The object's vtable: its first word.
    fn vtable_of(object: *mut c_void) -> *const usize {
        // SAFETY: `object` is the non-null object `GetSpout` returned, and a
        // COM-like interface begins with its vtable pointer, which lives as
        // long as the library.
        unsafe { *(object as *const *const usize) }
    }

    /// The virtual call at `slot`.
    fn method<T: Copy>(vtable: *const usize, slot: usize) -> T {
        // SAFETY: `slot` is one of this file's constants, inside the 175
        // entries the 2.007 header declares, and `T` is the pointer-sized fn
        // type the header gives that call.
        unsafe { std::mem::transmute_copy(&*vtable.add(slot)) }
    }

    /// Deletes the object `GetSpout` built.
    fn destroy(f: ReleaseFn, object: *mut c_void) {
        // SAFETY: the object is this `Object`'s own, released once from its
        // `Drop` while the library holding its code is still loaded.
        unsafe { f(object) };
    }

    /// A C string Spout wrote, copied out; empty when there is none.
    fn text_of(text: *const c_char) -> String {
        if text.is_null() {
            return String::new();
        }
        // SAFETY: the pointer is to a NUL-terminated buffer of NAME_BYTES that
        // `GetSender` filled, alive for this call.
        unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn a_receiver_needs_a_sender_name() -> Result<()> {
        let Err(problem) = SpoutReceiver::open("  ") else {
            anyhow::bail!("an unnamed receiver was opened");
        };
        assert!(
            problem.note.contains("sender's name"),
            "the refusal says what is missing: {problem}"
        );
        Ok(())
    }

    #[test]
    fn listing_senders_answers_without_a_window() {
        // With or without Spout installed: a list, or a sentence saying why.
        match spout_senders() {
            Ok(names) => assert!(
                names.iter().all(|name| !name.is_empty()),
                "no empty names: {names:?}"
            ),
            Err(problem) => assert!(!problem.note.is_empty(), "a reason is given"),
        }
    }
}
