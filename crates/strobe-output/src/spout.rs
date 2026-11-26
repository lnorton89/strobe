// ============================================================================
// Module:       strobe_output::spout
// Description:  The Spout sender: the picture shared as a GL texture, through
//               the Spout library found at runtime, with no readback at all.
//
// Dependencies: crate::sink (Sink, Problem), crate::texture (SharedTexture),
//               crate::Frame; libloading and the Spout library's exports on
//               Windows
// ============================================================================

//! Spout: the GPU-to-GPU path (D570, D571).
//!
//! Spout shares a Direct3D 11 texture with another program on the same
//! machine; a GL renderer hands its texture to Spout's interop layer, which
//! copies GPU-to-GPU. Nothing is read back, nothing is encoded, and the cost
//! is the interop copy and nothing else. That is why it is the Windows path
//! of choice for a VJ rig: Resolume, TouchDesigner, OBS and Notch all speak
//! it.
//!
//! **The library.** `SpoutLibrary.dll` as distributed (Synesthesia installs
//! one) exports `GetSpout`, which returns a `Spout*`, and then the class's
//! own methods under C++ name-mangled names — there is no C ABI and no
//! import library. So the library is opened with `libloading` at runtime and
//! its exports are resolved by those names; a build of the DLL that does not
//! carry them is reported as unusable rather than crashing the show (D571).
//! The names below are from the DLL on the owner's machine (Spout SDK
//! 2.006, 2017) and are the ones Spout has shipped for years.
//!
//! **The thread.** `SendTexture` registers the GL texture with Direct3D on
//! the calling thread, so it must be called where the GL context is current
//! — the app's render thread — and the sender must not be moved to another
//! one. The object's address is kept as a `usize`, not a `*mut`, so the type
//! stays `Send` for the [`Sink`] trait; nothing else may use it off that
//! thread.
//!
//! **The flip.** Spout's shared texture is top-left first and a GL target is
//! bottom-left first, so the picture is sent inverted (`bInvert`), which is
//! what other GL engines do. If a receiver shows it upside down, that flag is
//! the one thing to change, and it is a constant with its reason here rather
//! than a setting nobody understands.

use crate::Frame;
use crate::sink::{Problem, Sink};
use crate::texture::SharedTexture;

/// The Spout library's own signatures, for the calls `spout::imp` makes
/// (D571). File scope, above every function: the conventions check reads a
/// module-level block as part of the nearest function above it, and these are
/// the shapes the operations take, not operations.
#[cfg(windows)]
mod abi {
    use std::ffi::{c_char, c_void};

    pub(super) type GetSpoutFn = unsafe extern "system" fn() -> *mut c_void;
    pub(super) type CreateSenderFn =
        unsafe extern "system" fn(*mut c_void, *const c_char, u32, u32, u32) -> bool;
    pub(super) type SendTextureFn =
        unsafe extern "system" fn(*mut c_void, u32, u32, u32, u32, bool, u32) -> bool;
    pub(super) type ReleaseSenderFn = unsafe extern "system" fn(*mut c_void, u32);
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
    /// The library missing, or not one Strobe can call; or Spout refusing the
    /// sender (a name in use, or no address to share through).
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

/// The platform's Spout sender: the library's exports on Windows, and a
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

/// The library's exports (D571). Every call into the DLL is one named,
/// private, safe function holding one FFI block, as the GL boundary is
/// (`strobe-conventions`'s unsafe boundary names this file).
#[cfg(windows)]
#[expect(unsafe_code, reason = "the Spout library has no safe Rust binding")]
mod imp {
    use super::abi::{CreateSenderFn, GetSpoutFn, ReleaseSenderFn, SendTextureFn};
    use super::{Problem, SharedTexture};
    use libloading::Library;
    use std::ffi::{CString, c_void};
    use std::path::Path;

    /// `Spout* GetSpout()`: the one C export, returning the class instance.
    const GET_SPOUT: &[u8] = b"GetSpout";

    /// `bool Spout::CreateSender(const char*, unsigned, unsigned, DWORD)`.
    const CREATE_SENDER: &[u8] = b"?CreateSender@Spout@@QAE_NPBDIIK@Z";

    /// `bool Spout::SendTexture(GLuint, GLenum, unsigned, unsigned, bool, GLuint)`.
    const SEND_TEXTURE: &[u8] = b"?SendTexture@Spout@@QAE_NIIII_NI@Z";

    /// `void Spout::ReleaseSender(DWORD)`.
    const RELEASE_SENDER: &[u8] = b"?ReleaseSender@Spout@@QAEXK@Z";

    /// Whether the picture is flipped on the way out: Spout's shared texture
    /// is top-left first, a GL target bottom-left first.
    const INVERT: bool = true;

    /// The `dwFormat` a sender passes when the shared texture's format is
    /// Spout's own choice (`DXGI_FORMAT_UNKNOWN`).
    const FORMAT_DEFAULT: u32 = 0;

    /// The value `ReleaseSender` takes to release the sender: zero.
    const RELEASE: u32 = 0;

    /// The host framebuffer for the interop: the default one.
    const HOST_FBO: u32 = 0;

    /// A sender through one open library.
    #[derive(Debug)]
    pub(super) struct Sender {
        /// Kept alive for the whole sender: the resolved calls point into it.
        _library: Library,
        /// The `Spout` instance's address. See the module note: only the
        /// thread that opened the sender, with its GL context, may use it.
        instance: usize,
        send_texture: SendTextureFn,
        release_sender: ReleaseSenderFn,
        open: bool,
    }

    impl Sender {
        /// Finds the library, resolves its exports and creates a sender.
        pub(super) fn open(name: &str) -> Result<Self, Problem> {
            let path = crate::probe::spout_library().ok_or_else(|| {
                Problem::new(
                    "Spout output needs SpoutLibrary.dll, which was not found beside Strobe or on \
                     the PATH; install Spout from spout.zeal.co, or from an application that \
                     ships it",
                )
            })?;
            let library = load_library(&path)?;
            let get_spout: GetSpoutFn = export(&library, GET_SPOUT)?;
            let create_sender: CreateSenderFn = export(&library, CREATE_SENDER)?;
            let send_texture: SendTextureFn = export(&library, SEND_TEXTURE)?;
            let release_sender: ReleaseSenderFn = export(&library, RELEASE_SENDER)?;
            let instance = spout_instance(get_spout);
            if instance.is_null() {
                return Err(Problem::new(
                    "the Spout library did not return a sender object",
                ));
            }
            // The name is NUL-terminated and outlives the creation call.
            let name = CString::new(name)
                .map_err(|_| Problem::new("a Spout sender's name cannot contain a zero byte"))?;
            if !make_sender(create_sender, instance, &name) {
                return Err(Problem::new(
                    "Spout refused the sender; the name may be in use by another program",
                ));
            }
            Ok(Self {
                _library: library,
                instance: instance as usize,
                send_texture,
                release_sender,
                open: true,
            })
        }

        /// Shares one frame of the renderer's texture.
        ///
        /// A size change is a new sender: Spout fixes a sender's size when it
        /// is created, so a resize is refused here and the host reopens it.
        pub(super) fn send_texture(&mut self, texture: SharedTexture) -> Result<(), Problem> {
            if !self.open {
                return Err(Problem::new("the Spout sender is closed"));
            }
            if !texture.is_whole() {
                return Err(Problem::new("the picture has no texture to share yet"));
            }
            let instance = self.instance as *mut c_void;
            if share_texture(self.send_texture, instance, texture) {
                Ok(())
            } else {
                Err(Problem::new(
                    "Spout could not share the texture; the sender or the Direct3D interop was \
                     lost",
                ))
            }
        }

        /// Releases the sender, so it disappears from a receiver's list.
        pub(super) fn close(&mut self) {
            if !self.open {
                return;
            }
            let instance = self.instance as *mut c_void;
            release(self.release_sender, instance);
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
                "could not open {}: {error}; is it a 64-bit build?",
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
                "this SpoutLibrary.dll is not one Strobe can call (no `{}` export); it needs the \
                 Spout 2 library that ships with Spout or an application that uses it",
                String::from_utf8_lossy(name).trim_start_matches('?')
            ))),
        }
    }

    /// The library's one `Spout` instance.
    fn spout_instance(get_spout: GetSpoutFn) -> *mut c_void {
        // SAFETY: `GetSpout` returns the instance the library builds on first
        // call and keeps for the process; the pointer is only ever passed
        // back to that instance's own methods.
        unsafe { get_spout() }
    }

    /// Creates the sender: `CreateSender(name, width, height, format)`. The
    /// size is Spout's (zero: it takes the picture's), because the texture
    /// that first arrives decides it.
    fn make_sender(create: CreateSenderFn, instance: *mut c_void, name: &CString) -> bool {
        // SAFETY: `instance` is the library's own object, `name` is a valid
        // NUL-terminated string that outlives the call, and the remaining
        // arguments are plain numbers the library validates. Run on the
        // render thread with the GL context current, which Spout's interop
        // requires.
        unsafe { create(instance, name.as_ptr(), 0, 0, FORMAT_DEFAULT) }
    }

    /// Shares `texture`: `SendTexture(id, target, width, height, invert,
    /// host)`.
    fn share_texture(send: SendTextureFn, instance: *mut c_void, texture: SharedTexture) -> bool {
        // SAFETY: `instance` is the library's `Spout` object, still alive
        // because the library and the process are; the texture belongs to
        // the renderer's live target and the calling thread has its GL
        // context current, which is the contract `SendTexture` needs.
        unsafe {
            send(
                instance,
                texture.id,
                texture.target,
                texture.width,
                texture.height,
                INVERT,
                HOST_FBO,
            )
        }
    }

    /// Releases the sender.
    fn release(f: ReleaseSenderFn, instance: *mut c_void) {
        // SAFETY: as for `share_texture`: the object is the library's own and
        // `ReleaseSender` only tears down the sender it built.
        unsafe { f(instance, RELEASE) };
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
