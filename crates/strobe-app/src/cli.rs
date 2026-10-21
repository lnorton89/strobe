// ============================================================================
// Module:       strobe_app::cli
// Description:  The command line: library folder, starting scene, GL backend,
//               render scale, and the headless screenshot mode.
//
// Dependencies: clap; strobe-gfx (GlChoice, Backend)
// ============================================================================

//! The command line.
//!
//! `--screenshot <path> --frames <n>` is how the UI is verified with no
//! human present: the app renders `n` frames, reads both windows back to
//! `<path>-control.png` and `<path>-output.png`, and exits.
//! The output window starts hidden; `--show-output` opens it at launch.
//! `--diagnose` prints the system report and exits (see [`crate::diagnose`]).

use clap::{ArgGroup, Parser, ValueEnum};
use std::path::{Path, PathBuf};
use strobe_gfx::{Backend, GlChoice};

/// The exit status of a run whose command line was refused (a flag that
/// does not exist, a value out of range, two flags that conflict), D454.
/// Clap's own usage status, and not the 1 of a run that failed or
/// stopped: running the same command again cannot help, so a launcher
/// that restarts Strobe can tell a configuration error from a fault and
/// stop instead of looping.
pub const REFUSED_EXIT: u8 = 2;

/// Which GL stack to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum GlArg {
    /// Native GL 3.3 first, then ANGLE if native fails.
    Auto,
    /// Native GL 3.3 only.
    Native,
    /// ANGLE (EGL, OpenGL ES 3.0) only.
    Angle,
}

impl GlArg {
    /// The renderer's own name for this choice.
    pub fn choice(self) -> GlChoice {
        match self {
            Self::Auto => GlChoice::Auto,
            Self::Native => GlChoice::Only(Backend::Native),
            Self::Angle => GlChoice::Only(Backend::Angle),
        }
    }
}

/// Where audio comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AudioArg {
    /// Capture from an input device; fall back to synthetic audio when
    /// there is none.
    Auto,
    /// Always synthetic audio: a fixed 120 BPM pattern, reproducible.
    Synthetic,
    /// The synthetic pattern with its hits and beat triggers removed and
    /// its levels replaced by a slow swell: scenes keep moving without a
    /// beat that the room cannot hear. For a show with no microphone.
    Ambient,
}

/// The command line.
#[derive(Parser, Debug)]
#[command(
    name = "strobe",
    version,
    about = "Strobe: an audio-reactive VJ engine",
    group(
        ArgGroup::new("venue_role")
            .args(["remote", "control"])
            .multiple(true)
    )
)]
pub struct Cli {
    /// Folder of scene folders.
    #[arg(long, default_value = "scenes")]
    pub library: PathBuf,
    /// Leave Synesthesia's scenes out of the library for this run, whatever
    /// Settings says. Where Synesthesia is installed, the scenes it ships
    /// (read-only) and those in the scene folders its settings name are
    /// listed after the library folder (D271). `STROBE_SYNESTHESIA_DIR` and
    /// `STROBE_SYNESTHESIA_DATA` name an install and its data folder in
    /// unusual places.
    #[arg(long)]
    pub no_synesthesia: bool,
    /// Scene to load at start: a folder name, or a unique prefix of one.
    /// The library folder's own scenes are found before Synesthesia's.
    #[arg(long)]
    pub scene: Option<String>,
    /// GL backend.
    #[arg(long, value_enum, default_value_t = GlArg::Auto)]
    pub gl: GlArg,
    /// Render resolution as a fraction of the output window, 0.25 to 1.0.
    #[arg(long, default_value_t = 1.0)]
    pub render_scale: f32,
    /// Render `--frames` frames, write `<PATH>-control.png` and
    /// `<PATH>-output.png`, and exit.
    #[arg(long, value_name = "PATH")]
    pub screenshot: Option<PathBuf>,
    /// Frames to render before the screenshot.
    #[arg(long, default_value_t = 60, requires = "screenshot")]
    pub frames: u32,
    /// Start with adaptive resolution off, so the render scale stays where
    /// `--render-scale` (or the slider) puts it.
    #[arg(long)]
    pub no_adaptive: bool,
    /// Audio source. `synthetic` makes frames reproducible, which is what
    /// the benchmark and screenshots want when a real device is present.
    #[arg(long, value_enum, default_value_t = AudioArg::Auto)]
    pub audio: AudioArg,
    /// Show the output window at launch. Without it only the control window
    /// opens, and the output is shown from the control window's button, F11,
    /// or the monitor picker (D96).
    #[arg(long)]
    pub show_output: bool,
    /// Put the output fullscreen at launch: on `--output-monitor` if given,
    /// else on the monitor the output window opens on. For a venue
    /// autostart, where nobody is there to press F11.
    #[arg(long)]
    pub fullscreen: bool,
    /// Show the output at launch on this monitor, numbered from 1 as the
    /// monitor picker numbers them.
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..))]
    pub output_monitor: Option<u32>,
    /// Accept shaders from the "Send to Strobe" browser userscript on
    /// 127.0.0.1 (off by default; the Import panel can switch it on too).
    #[arg(long)]
    pub listen: bool,
    /// The port the listener uses (127.0.0.1 only).
    #[arg(long, default_value_t = strobe_shadertoy::listener::DEFAULT_PORT)]
    pub listen_port: u16,
    /// Do not download Shadertoy textures when importing (they read black
    /// and are listed in the scene's images/pending-assets.json).
    #[arg(long)]
    pub no_texture_download: bool,
    /// Print a report of this machine, build, GL and audio setup, save it
    /// next to the logs, and exit. Attach it to bug reports.
    #[arg(long)]
    pub diagnose: bool,
    /// Open these panes at launch (comma separated): `browser`, `import`,
    /// `inspector`, `diagnostics`, `settings`. Added to whatever the saved
    /// layout opens; in screenshot mode there is no saved layout, so this
    /// alone decides.
    #[arg(long, value_name = "PANE", value_delimiter = ',')]
    pub open_pane: Vec<String>,
    /// Close every pane at launch (for a screenshot of the bare surface).
    /// Applied before `--open-pane`.
    #[arg(long)]
    pub no_panes: bool,
    /// Open the control window at this size instead of maximized, e.g.
    /// `1280x720`. Makes screenshots reproducible on a display with no
    /// window manager.
    #[arg(long, value_name = "WxH", value_parser = parse_size)]
    pub window_size: Option<(u32, u32)>,
    /// Prepare this scene in the browser's preview at launch, without
    /// making it live (screenshots of the Preview card).
    #[arg(long, value_name = "SCENE")]
    pub preview: Option<String>,
    /// Start with the output blacked out.
    #[arg(long)]
    pub blackout: bool,
    /// Show this picture (a .png or .jpg file) or folder of pictures (an
    /// image sequence, played in name order) behind `syn_Media` in every
    /// scene loaded by hand, in place of the scene's own `MEDIA`. It is
    /// decoded off the frame thread. Scenes a playlist plays keep their own.
    #[arg(long, value_name = "PATH")]
    pub media: Option<PathBuf>,
    /// In screenshot mode, ask for this scene to be loaded through the UI's
    /// own path on the frame before the control capture, so the capture shows
    /// the "preparing" state (the load itself still happens right after).
    #[arg(long, value_name = "SCENE", requires = "screenshot")]
    pub request_load: Option<String>,
    /// Run as a generate-controls worker on this scene and exit (the app
    /// starts itself this way; D202). Not for people.
    #[arg(long, value_name = "SCENE", hide = true)]
    pub generate_controls: Option<PathBuf>,
    /// The worker's library folder.
    #[arg(long, value_name = "DIR", hide = true, requires = "generate_controls")]
    pub generate_library: Option<PathBuf>,
    /// The worker's result file.
    #[arg(long, value_name = "FILE", hide = true, requires = "generate_controls")]
    pub generate_report: Option<PathBuf>,
    /// The worker adds to authored controls.
    #[arg(long, hide = true, requires = "generate_controls")]
    pub generate_merge: bool,
    /// Search text to put in the scene browser at launch.
    #[arg(long, value_name = "TEXT")]
    pub browser_query: Option<String>,
    /// Open the command palette at launch with this text typed (screenshots
    /// of the palette; an empty string opens it empty).
    #[arg(long, value_name = "TEXT")]
    pub palette: Option<String>,
    /// Use this theme (by id, e.g. `high-contrast`) for this run instead of
    /// the saved one. The Settings pane changes and saves it.
    #[arg(long, value_name = "ID")]
    pub theme: Option<String>,
    /// Use this density (`comfortable` or `compact`) for this run instead
    /// of the saved one (D220).
    #[arg(long, value_name = "DENSITY")]
    pub density: Option<String>,
    /// Draw the system's title bar instead of Strobe's for this run,
    /// whatever the saved setting says (D132).
    #[arg(long)]
    pub system_title_bar: bool,
    /// Load this playlist file at launch (TOML; see `playlists/example.toml`).
    /// Every scene it names is compiled before the first cue plays.
    #[arg(long, value_name = "FILE")]
    pub playlist: Option<PathBuf>,
    /// Do not resume the saved playlist position, and do not save one.
    /// (Screenshot mode never resumes or saves.)
    #[arg(long)]
    pub no_resume: bool,
    /// Listen for OSC messages on 127.0.0.1 (port 9000 unless `--osc-port`)
    /// for this run. The MAP pane's switch turns it on for good.
    #[arg(long)]
    pub osc: bool,
    /// The UDP port OSC listens on for this run.
    #[arg(long, value_name = "PORT")]
    pub osc_port: Option<u16>,
    /// Let OSC listen on every network interface, not only this machine, for
    /// this run. OSC has no password: anyone on the network can then change
    /// scenes and black out the projector (D172). Never saved.
    #[arg(long)]
    pub osc_all_interfaces: bool,
    /// Do not open MIDI input devices.
    #[arg(long)]
    pub no_midi: bool,
    /// Read mappings, macros and fav slots from this file instead of
    /// `show-control.json` in the data directory.
    #[arg(long, value_name = "FILE")]
    pub show_control: Option<PathBuf>,
    /// Watch this folder: a scene folder (or a folder of scenes) that
    /// appears in it is added to the library, and nothing plays (D240).
    /// What is in it at launch is left alone. For this run only, over the
    /// one Settings keeps (D265).
    #[arg(long, value_name = "DIR")]
    pub watch_folder: Option<PathBuf>,
    /// Start the LAN remote at launch: a phone or tablet on the same
    /// network opens the address Strobe shows and pairs with a code and the
    /// desktop's approval (D250). Off by default; Settings can switch it on
    /// for this run too. Plain HTTP: use it on a network you trust.
    #[arg(long)]
    pub remote: bool,
    /// The address the LAN remote listens on, for this run. Without it,
    /// this computer's LAN address. A public address is refused; `0.0.0.0`
    /// (every interface) is allowed with a warning.
    #[arg(long, value_name = "IP")]
    pub remote_bind: Option<std::net::IpAddr>,
    /// The port the LAN remote listens on.
    #[arg(long, value_name = "PORT", default_value_t = strobe_remote::server::DEFAULT_PORT)]
    pub remote_port: u16,
    /// The public name of your own HTTPS reverse proxy on this computer,
    /// such as `strobe.example.com` or `strobe.example.com:8443` (D258).
    /// The remote then also answers that name, from this computer only,
    /// and binds 127.0.0.1 unless `--remote-bind` says otherwise. The proxy
    /// and its certificate are yours to run.
    #[arg(long, value_name = "NAME[:PORT]", value_parser = parse_proxy_name)]
    pub remote_proxy: Option<String>,
    /// Switch "Allow full desktop control" on at launch, for this run, so a
    /// headless show machine can be controlled without anyone at it (D288).
    /// Needs `--remote`. It does not approve anyone: pair with the host's
    /// Approve button, or with `--remote-trust`. Never saved.
    #[arg(long, requires = "remote")]
    pub allow_desktop_control: bool,
    /// Venue mode (D288): with `--remote`, a controller that proves it holds
    /// this trust file is paired for full desktop control with no approval
    /// click, while full desktop access is on; read once at launch. With
    /// `--control`, the file this laptop proves to the host. Make the file
    /// with `--make-remote-trust`. Anyone who can read it has full control;
    /// keep it private. A relative path is taken from the folder Strobe is
    /// started in, and kept as the whole path (D463).
    #[arg(long, value_name = "FILE", requires = "venue_role", value_parser = parse_file_path)]
    pub remote_trust: Option<PathBuf>,
    /// Control another Strobe from this one (D275, D294): open straight into
    /// the viewer of the host at HOST:PORT (a bare IP takes port 47812). With
    /// `--remote-trust FILE` it connects by proving the file, with no
    /// approval at the host, and reconnects by itself after a host restart
    /// or a Wi-Fi drop. Without it, the "Control another Strobe" form opens
    /// with the address filled in, for pairing with a code. Not for a host:
    /// it cannot be combined with `--remote`.
    #[arg(
        long,
        value_name = "HOST:PORT",
        value_parser = parse_control,
        conflicts_with = "remote"
    )]
    pub control: Option<std::net::SocketAddr>,
    /// Make a trust file at FILE (32 random bytes from the operating
    /// system), print how to use it, and exit. Refuses to overwrite.
    #[arg(long, value_name = "FILE", value_parser = parse_file_path)]
    pub make_remote_trust: Option<PathBuf>,
}

/// A trust file's path, made whole against the folder Strobe was started in
/// (D463). Read once at launch either way, but a relative one in a venue
/// launcher is read from wherever the scheduler starts Strobe (often
/// `C:\Windows\System32`), a laptop remembers it for the form's next
/// launch from elsewhere, and the instructions `--make-remote-trust`
/// prints are copied into launchers: each says the whole path, so a
/// failure names the place that was looked in.
fn parse_file_path(text: &str) -> Result<PathBuf, String> {
    std::path::absolute(text).map_err(|why| format!("\"{text}\": {why}"))
}

/// The host `--control` names: an IP address and optionally a port.
fn parse_control(text: &str) -> Result<std::net::SocketAddr, String> {
    crate::desktop::parse_host(text).map_err(|why| format!("\"{text}\": {why}"))
}

/// A host name, optionally with a port: letters, digits, `-` and `.`.
fn parse_proxy_name(text: &str) -> Result<String, String> {
    let (name, port) = match text.rsplit_once(':') {
        Some((name, port)) => (name, Some(port)),
        None => (text, None),
    };
    let name_ok = !name.is_empty()
        && name.len() <= 253
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
        && !name.starts_with(['-', '.'])
        && name.bytes().any(|b| b.is_ascii_alphabetic());
    if !name_ok {
        return Err(format!(
            "\"{text}\" is not a host name (letters, digits, - and ., such as strobe.example.com)"
        ));
    }
    if let Some(port) = port
        && !port.parse::<u16>().is_ok_and(|p| p > 0)
    {
        return Err(format!("\"{port}\" is not a port (1 to 65535)"));
    }
    Ok(text.to_ascii_lowercase())
}

/// Parses `1280x720`.
fn parse_size(text: &str) -> Result<(u32, u32), String> {
    let (w, h) = text
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("`{text}` is not WIDTHxHEIGHT"))?;
    let w: u32 = w
        .trim()
        .parse()
        .map_err(|_| format!("`{w}` is not a width"))?;
    let h: u32 = h
        .trim()
        .parse()
        .map_err(|_| format!("`{h}` is not a height"))?;
    if !(320..=16_384).contains(&w) || !(240..=16_384).contains(&h) {
        return Err(format!("{w}x{h} is outside 320x240 to 16384x16384"));
    }
    Ok((w, h))
}

impl Cli {
    /// Whether a library that cannot be read is no fault: this run controls
    /// another Strobe (`--control`) and was not pointed at a library, so the
    /// default `scenes` folder (resolved against the working directory) has
    /// no reason to exist.
    pub fn library_is_optional(&self) -> bool {
        self.control.is_some() && self.library == Path::new("scenes")
    }

    /// Whether a box about a crash, or about Strobe stopping after its
    /// windows opened, would sit on the show instead of telling anyone
    /// (D450). `--fullscreen` is the venue autostart: a box waiting for a
    /// click there holds the process, so the launcher cannot restart it,
    /// and one raised after it died is drawn over the restarted show. Such
    /// failures are logged and written to `crashes\` as always; a failure
    /// to start (no windows yet, nothing to cover) still gets its box.
    pub fn no_boxes_over_the_show(&self) -> bool {
        self.fullscreen
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn defaults_load_the_scenes_folder_with_auto_backend() -> Result<()> {
        let cli = Cli::try_parse_from(["strobe"])?;
        assert_eq!(cli.library, PathBuf::from("scenes"), "library");
        assert_eq!(cli.gl, GlArg::Auto, "backend");
        assert!(cli.screenshot.is_none(), "no screenshot");
        Ok(())
    }

    #[test]
    fn osc_is_off_unless_asked_for_and_never_on_every_interface_by_default() -> Result<()> {
        let cli = Cli::try_parse_from(["strobe"])?;
        assert!(
            !cli.osc && !cli.osc_all_interfaces && !cli.no_midi,
            "all off"
        );
        let cli = Cli::try_parse_from(["strobe", "--osc", "--osc-port", "9100"])?;
        assert!(cli.osc && !cli.osc_all_interfaces, "loopback unless asked");
        assert_eq!(cli.osc_port, Some(9100), "port");
        Ok(())
    }

    #[test]
    fn a_forced_backend_maps_to_one_attempt() -> Result<()> {
        let cli = Cli::try_parse_from(["strobe", "--gl", "angle"])?;
        assert_eq!(
            cli.gl.choice(),
            GlChoice::Only(Backend::Angle),
            "angle forced"
        );
        Ok(())
    }

    #[test]
    fn audio_defaults_to_auto_and_can_be_forced_synthetic() -> Result<()> {
        assert_eq!(
            Cli::try_parse_from(["strobe"])?.audio,
            AudioArg::Auto,
            "auto"
        );
        let forced = Cli::try_parse_from(["strobe", "--audio", "synthetic"])?;
        assert_eq!(forced.audio, AudioArg::Synthetic, "forced");
        Ok(())
    }

    #[test]
    fn adaptive_resolution_is_on_unless_switched_off() -> Result<()> {
        assert!(
            !Cli::try_parse_from(["strobe"])?.no_adaptive,
            "on by default"
        );
        assert!(
            Cli::try_parse_from(["strobe", "--no-adaptive"])?.no_adaptive,
            "off on request"
        );
        Ok(())
    }

    #[test]
    fn diagnose_is_a_plain_flag() -> Result<()> {
        assert!(
            Cli::try_parse_from(["strobe", "--diagnose"])?.diagnose,
            "set"
        );
        assert!(!Cli::try_parse_from(["strobe"])?.diagnose, "off by default");
        Ok(())
    }

    #[test]
    fn fullscreen_and_a_monitor_can_be_asked_for_at_launch() -> Result<()> {
        let cli = Cli::try_parse_from(["strobe", "--fullscreen", "--output-monitor", "2"])?;
        assert!(cli.fullscreen, "fullscreen asked for");
        assert_eq!(
            cli.output_monitor,
            Some(2),
            "the monitor numbered as the picker numbers it"
        );
        assert!(
            Cli::try_parse_from(["strobe", "--output-monitor", "0"]).is_err(),
            "monitors are numbered from 1"
        );
        let plain = Cli::try_parse_from(["strobe"])?;
        assert!(
            !plain.fullscreen && plain.output_monitor.is_none(),
            "neither by default"
        );
        Ok(())
    }

    #[test]
    fn the_output_window_is_hidden_unless_asked_for() -> Result<()> {
        assert!(
            !Cli::try_parse_from(["strobe"])?.show_output,
            "hidden by default"
        );
        assert!(
            Cli::try_parse_from(["strobe", "--show-output"])?.show_output,
            "shown on request"
        );
        let shot = Cli::try_parse_from(["strobe", "--screenshot", "x", "--show-output"])?;
        assert!(
            shot.show_output && shot.screenshot.is_some(),
            "combines with screenshot mode"
        );
        Ok(())
    }

    #[test]
    fn the_listener_is_off_unless_asked_for_and_the_port_has_a_default() -> Result<()> {
        let off = Cli::try_parse_from(["strobe"])?;
        assert!(!off.listen, "off by default");
        assert_eq!(off.listen_port, 47811, "the documented port");
        assert!(!off.no_texture_download, "textures download by default");
        let on = Cli::try_parse_from(["strobe", "--listen", "--listen-port", "50000"])?;
        assert!(on.listen, "on when asked");
        assert_eq!(on.listen_port, 50000, "port");
        assert!(
            Cli::try_parse_from(["strobe", "--listen-port", "70000"]).is_err(),
            "not a port"
        );
        Ok(())
    }

    #[test]
    fn panes_and_the_window_size_are_parsed() -> Result<()> {
        let cli = Cli::try_parse_from([
            "strobe",
            "--open-pane",
            "browser,diagnostics",
            "--window-size",
            "1280x720",
        ])?;
        assert_eq!(cli.open_pane, vec!["browser", "diagnostics"], "panes");
        assert_eq!(cli.window_size, Some((1280, 720)), "size");
        assert!(
            Cli::try_parse_from(["strobe", "--window-size", "big"]).is_err(),
            "not a size"
        );
        assert!(
            Cli::try_parse_from(["strobe", "--window-size", "10x10"]).is_err(),
            "too small to be a window"
        );
        Ok(())
    }

    #[test]
    fn the_ui_flags_default_to_off() -> Result<()> {
        let cli = Cli::try_parse_from(["strobe"])?;
        assert!(cli.open_pane.is_empty() && !cli.no_panes, "panes");
        assert!(
            cli.window_size.is_none() && cli.preview.is_none(),
            "size, preview"
        );
        assert!(
            !cli.blackout && !cli.system_title_bar,
            "blackout, title bar"
        );
        Ok(())
    }

    #[test]
    fn a_playlist_can_be_named_and_resuming_can_be_switched_off() -> Result<()> {
        let none = Cli::try_parse_from(["strobe"])?;
        assert!(none.playlist.is_none() && !none.no_resume, "off by default");
        let cli = Cli::try_parse_from(["strobe", "--playlist", "playlists/a.toml", "--no-resume"])?;
        assert_eq!(
            cli.playlist,
            Some(PathBuf::from("playlists/a.toml")),
            "path"
        );
        assert!(cli.no_resume, "no resume");
        Ok(())
    }

    #[test]
    fn the_lan_remote_is_off_unless_asked_for() -> Result<()> {
        let cli = Cli::try_parse_from(["strobe"])?;
        assert!(!cli.remote, "no socket by default");
        assert_eq!(cli.remote_bind, None, "the LAN address unless named");
        assert_eq!(cli.remote_port, 47812, "the default port");
        let cli = Cli::try_parse_from([
            "strobe",
            "--remote",
            "--remote-bind",
            "192.168.1.20",
            "--remote-port",
            "50000",
        ])?;
        assert!(cli.remote, "on");
        assert_eq!(
            cli.remote_bind.map(|ip| ip.to_string()).as_deref(),
            Some("192.168.1.20"),
            "address"
        );
        assert_eq!(cli.remote_port, 50000, "port");
        let proxied = Cli::try_parse_from(["strobe", "--remote-proxy", "Strobe.Example.com:8443"])?;
        assert_eq!(
            proxied.remote_proxy.as_deref(),
            Some("strobe.example.com:8443"),
            "a proxy name, lower-cased"
        );
        for bad in [
            "",
            "192.168.1.2",
            "evil/x",
            "-x.com",
            "x.com:0",
            "x.com:http",
        ] {
            assert!(
                Cli::try_parse_from(["strobe", "--remote-proxy", bad]).is_err(),
                "{bad:?} is not a proxy name"
            );
        }
        assert!(
            Cli::try_parse_from(["strobe", "--remote-bind", "phone.local"]).is_err(),
            "an address, never a name"
        );
        Ok(())
    }

    #[test]
    fn venue_mode_flags_need_the_remote_and_are_off_by_default() -> Result<()> {
        let plain = Cli::try_parse_from(["strobe"])?;
        assert!(
            !plain.allow_desktop_control
                && plain.remote_trust.is_none()
                && plain.make_remote_trust.is_none(),
            "nothing by default"
        );
        let host = Cli::try_parse_from([
            "strobe",
            "--remote",
            "--allow-desktop-control",
            "--remote-trust",
            "venue.trust",
        ])?;
        assert!(host.allow_desktop_control, "switch on at launch");
        assert_eq!(
            host.remote_trust,
            Some(std::env::current_dir()?.join("venue.trust")),
            "file, as a whole path"
        );
        for lone in [
            ["strobe", "--allow-desktop-control"],
            ["strobe", "--remote-trust=venue.trust"],
        ] {
            assert!(
                Cli::try_parse_from(lone).is_err(),
                "{lone:?} does nothing without --remote, so it is refused"
            );
        }
        let make = Cli::try_parse_from(["strobe", "--make-remote-trust", "venue.trust"])?;
        assert_eq!(
            make.make_remote_trust,
            Some(std::env::current_dir()?.join("venue.trust")),
            "make"
        );
        Ok(())
    }

    #[test]
    fn a_relative_trust_file_is_kept_as_the_whole_path_it_named_at_launch() -> Result<()> {
        // A venue launcher run by the task scheduler starts in another
        // folder than the one the operator tested from, and a laptop
        // remembers the path for a later launch from a shortcut: a bare
        // `venue.trust` then names nothing, and the failure did not say
        // where it had looked (D463).
        let here = std::env::current_dir()?;
        for role in ["--remote", "--control=192.168.1.50"] {
            let cli = Cli::try_parse_from(["strobe", role, "--remote-trust", "venue.trust"])?;
            let Some(path) = cli.remote_trust else {
                anyhow::bail!("{role}: no file");
            };
            assert!(path.is_absolute(), "{role}: {}", path.display());
            assert_eq!(
                path,
                here.join("venue.trust"),
                "{role}: from where it was named"
            );
        }
        let whole = here.join("keys").join("show.trust");
        let cli = Cli::try_parse_from([
            "strobe".as_ref(),
            "--remote".as_ref(),
            "--remote-trust".as_ref(),
            whole.as_os_str(),
        ])?;
        assert_eq!(
            cli.remote_trust,
            Some(whole),
            "a whole path is kept as given"
        );
        assert!(
            Cli::try_parse_from(["strobe", "--remote", "--remote-trust", ""]).is_err(),
            "an empty path is refused"
        );
        Ok(())
    }

    #[test]
    fn a_controller_without_a_library_does_not_need_one() -> Result<()> {
        let controller = Cli::try_parse_from(["strobe", "--control", "10.0.0.5"])?;
        assert!(
            controller.library_is_optional(),
            "a controller run with the default library"
        );
        let named = Cli::try_parse_from(["strobe", "--control", "10.0.0.5", "--library", "mine"])?;
        assert!(
            !named.library_is_optional(),
            "a library the person named is reported when it is missing"
        );
        assert!(
            !Cli::try_parse_from(["strobe"])?.library_is_optional(),
            "a show machine needs its library"
        );
        Ok(())
    }

    #[test]
    fn control_names_a_host_and_takes_the_trust_file_for_the_laptop() -> Result<()> {
        assert!(
            Cli::try_parse_from(["strobe"])?.control.is_none(),
            "this Strobe controls nothing by default"
        );
        let laptop = Cli::try_parse_from([
            "strobe",
            "--control",
            "192.168.1.50:47812",
            "--remote-trust",
            "venue.trust",
        ])?;
        assert_eq!(
            laptop.control.map(|address| address.to_string()).as_deref(),
            Some("192.168.1.50:47812"),
            "the host"
        );
        assert_eq!(
            laptop.remote_trust,
            Some(std::env::current_dir()?.join("venue.trust")),
            "the laptop's file: allowed with --control, as with --remote"
        );
        assert!(!laptop.remote, "and it is not a host");
        let bare = Cli::try_parse_from(["strobe", "--control", "10.0.0.5"])?;
        assert_eq!(
            bare.control.map(|address| address.to_string()).as_deref(),
            Some("10.0.0.5:47812"),
            "a bare IP takes the remote's default port"
        );
        assert!(bare.remote_trust.is_none(), "no file: the form, for a code");
        for bad in [
            ["strobe", "--control", "show-pc"].as_slice(),
            &["strobe", "--control", "10.0.0.5:"],
            &["strobe", "--control", "8.8.8.8:99999"],
            &["strobe", "--control"],
            // A host and a controller are different roles.
            &["strobe", "--control", "10.0.0.5", "--remote"],
        ] {
            assert!(Cli::try_parse_from(bad).is_err(), "{bad:?} must be refused");
        }
        Ok(())
    }

    #[test]
    fn a_trust_file_alone_still_belongs_to_nothing_but_keeps_its_host_meaning() -> Result<()> {
        assert!(
            Cli::try_parse_from(["strobe", "--remote-trust", "venue.trust"]).is_err(),
            "a file with no role is refused, as before"
        );
        let host = Cli::try_parse_from(["strobe", "--remote", "--remote-trust", "venue.trust"])?;
        assert!(
            host.remote && host.control.is_none(),
            "with --remote it is the host's, as in D288"
        );
        Ok(())
    }

    #[test]
    fn a_watched_folder_is_off_unless_named() -> Result<()> {
        assert!(
            Cli::try_parse_from(["strobe"])?.watch_folder.is_none(),
            "off by default"
        );
        let cli = Cli::try_parse_from(["strobe", "--watch-folder", "inbox"])?;
        assert_eq!(cli.watch_folder, Some(PathBuf::from("inbox")), "named");
        Ok(())
    }

    #[test]
    fn synesthesias_scenes_are_listed_unless_switched_off_for_the_run() -> Result<()> {
        assert!(
            !Cli::try_parse_from(["strobe"])?.no_synesthesia,
            "listed by default, where installed"
        );
        assert!(
            Cli::try_parse_from(["strobe", "--no-synesthesia"])?.no_synesthesia,
            "off for this run"
        );
        Ok(())
    }

    #[test]
    fn a_fullscreen_launch_raises_no_crash_or_stop_box_over_the_show() -> Result<()> {
        let venue = Cli::try_parse_from([
            "strobe",
            "--playlist",
            "show.toml",
            "--output-monitor",
            "1",
            "--fullscreen",
            "--audio",
            "synthetic",
        ])?;
        assert!(
            venue.no_boxes_over_the_show(),
            "the venue launch: a box would hold the process or cover the restarted show"
        );
        for attended in [
            ["strobe"].as_slice(),
            &["strobe", "--show-output"],
            &["strobe", "--output-monitor", "2"],
        ] {
            assert!(
                !Cli::try_parse_from(attended)?.no_boxes_over_the_show(),
                "{attended:?}: someone is at the control window, so the box is how they hear"
            );
        }
        Ok(())
    }

    #[test]
    fn frames_without_a_screenshot_is_rejected() {
        assert!(
            Cli::try_parse_from(["strobe", "--frames", "5"]).is_err(),
            "--frames needs --screenshot"
        );
    }
}
