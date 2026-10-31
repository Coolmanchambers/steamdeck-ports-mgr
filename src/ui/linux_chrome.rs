
use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ClientMessageEvent, ConnectionExt, EventMask, MapState, PropMode};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NativeWindow {
    X11(u32),
    Wayland,
}

pub fn native_window(window: &slint::Window) -> Option<NativeWindow> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match window.window_handle().window_handle().ok()?.as_raw() {
        RawWindowHandle::Xcb(h) => Some(NativeWindow::X11(h.window.get())),
        RawWindowHandle::Xlib(h) => Some(NativeWindow::X11(h.window as u32)),
        RawWindowHandle::Wayland(_) => Some(NativeWindow::Wayland),
        _ => None,
    }
}

// Fork (upstream bug O8): one socket per install folder, so this fork and the original (or two copies in
// different folders) can run side by side; a second start from the same folder still hands over. FNV-1a
// rather than std's hasher so the name stays the same across builds.
fn install_folder_tag(folder: &Path) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in folder.as_os_str().as_encoded_bytes() {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn instance_socket_path_for(runtime_dir: Option<PathBuf>, user: &str, install_folder: &Path) -> PathBuf {
    let tag = install_folder_tag(install_folder);
    match runtime_dir.filter(|dir| dir.is_dir()) {
        Some(dir) => dir.join(format!("ports_launcher-{tag}.sock")),
        None => std::env::temp_dir().join(format!("ports_launcher-{user}-{tag}.sock")),
    }
}

fn install_folder() -> PathBuf {
    std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)).unwrap_or_default()
}

fn instance_socket_path() -> PathBuf {
    instance_socket_path_for(std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from), &std::env::var("USER").unwrap_or_default(), &install_folder())
}

/// The one name every copy used before this fix (and upstream's copies still use).
fn legacy_socket_path() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|dir| dir.is_dir()) {
        Some(dir) => dir.join("ports_launcher.sock"),
        None => std::env::temp_dir().join(format!("ports_launcher-{}.sock", std::env::var("USER").unwrap_or_default())),
    }
}

/// The inode of the listening socket bound at `path`, from the text of `/proc/net/unix`
/// (`Num RefCount Protocol Flags Type St Inode Path`; a listening socket has flag `00010000`).
fn listening_inode(table: &str, path: &Path) -> Option<String> {
    table.lines().skip(1).find_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let listening = fields.get(3).and_then(|f| u32::from_str_radix(f, 16).ok()).is_some_and(|flags| flags & 0x0001_0000 != 0);
        (fields.len() >= 8 && listening && Path::new(fields[7]) == path).then(|| fields[6].to_string())
    })
}

/// The program of the process holding socket `inode`, when this user can see it.
fn socket_owner_exe(inode: &str) -> Option<PathBuf> {
    let needle = format!("socket:[{inode}]");
    fs::read_dir("/proc").ok()?.flatten().find_map(|entry| {
        let is_pid = entry.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit());
        let fds = fs::read_dir(entry.path().join("fd")).ok().filter(|_| is_pid)?;
        let owns = fds.flatten().any(|fd| fs::read_link(fd.path()).is_ok_and(|target| target.as_os_str() == needle.as_str()));
        owns.then(|| fs::read_link(entry.path().join("exe")).ok()).flatten()
    })
}

/// Hand over to a copy on the old socket name only when it runs from this folder (a copy from before the
/// fix, still open); one from another folder (upstream's app) is left alone. Unknown owner: hand over, as
/// two copies of one folder would both write its state.json.
fn hand_over_to_legacy(owner_exe: Option<&Path>, our_folder: &Path) -> bool {
    owner_exe.is_none_or(|exe| exe.parent() == Some(our_folder))
}

fn signal_running_copy(path: &Path, what: &str) {
    use std::os::unix::net::UnixStream;
    match UnixStream::connect(path).and_then(|mut stream| stream.write_all(&[0])) {
        Ok(()) => eprintln!("{what} is already running from this folder; asked it to come to the front, quitting ({})", path.display()),
        Err(e) => eprintln!("{what} is already running from this folder but did not answer ({e}); quitting ({})", path.display()),
    }
}

pub fn claim_single_instance(on_activate: impl Fn() + Send + 'static) -> bool {
    use std::os::unix::net::{UnixListener, UnixStream};
    let path = instance_socket_path();
    if UnixStream::connect(&path).is_ok() {
        signal_running_copy(&path, "this app");
        return false;
    }
    // Fork: decided from /proc without connecting, because a copy listening there brings itself to the
    // front on any connection, and upstream's app in another folder must be left alone.
    let legacy = legacy_socket_path();
    let legacy_from_here = match fs::read_to_string("/proc/net/unix") {
        Ok(table) => listening_inode(&table, &legacy).is_some_and(|inode| hand_over_to_legacy(socket_owner_exe(&inode).as_deref(), &install_folder())),
        Err(_) => UnixStream::connect(&legacy).is_ok(),
    };
    if legacy_from_here {
        signal_running_copy(&legacy, "an older copy of this app");
        return false;
    }
    let _ = fs::remove_file(&path);
    let Ok(listener) = UnixListener::bind(&path) else { return true };
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            drop(stream);
            on_activate();
        }
    });
    true
}

struct Session {
    conn: RustConnection,
    root: u32,
    screen_width: i32,
    screen_height: i32,
}

thread_local! {
    static SESSION: OnceCell<Option<Session>> = const { OnceCell::new() };
    static ATOMS: RefCell<HashMap<&'static str, u32>> = RefCell::new(HashMap::new());
    static ICON: OnceCell<Option<(Vec<u8>, u32, u32)>> = const { OnceCell::new() };
}

fn with_conn<T>(f: impl FnOnce(&Session) -> T) -> Option<T> {
    SESSION.with(|cell| {
        cell.get_or_init(|| {
            let (conn, screen_num) = x11rb::connect(None).ok()?;
            let screen = &conn.setup().roots[screen_num];
            let (root, screen_width, screen_height) = (screen.root, screen.width_in_pixels as i32, screen.height_in_pixels as i32);
            Some(Session { conn, root, screen_width, screen_height })
        })
        .as_ref()
        .map(f)
    })
}

fn cached_atom(conn: &RustConnection, name: &'static str) -> Option<u32> {
    if let Some(cached) = ATOMS.with(|c| c.borrow().get(name).copied()) {
        return Some(cached);
    }
    let value = conn.intern_atom(false, name.as_bytes()).ok()?.reply().ok()?.atom;
    ATOMS.with(|c| c.borrow_mut().insert(name, value));
    Some(value)
}

fn send_root_message(conn: &RustConnection, root: u32, window: u32, message_type: u32, data: [u32; 5]) -> Option<()> {
    let event = ClientMessageEvent::new(32, window, message_type, data);
    let mask = EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT;
    conn.send_event(false, root, mask, event).ok()?;
    conn.flush().ok()
}

fn set_property32(window: u32, property: &'static str, kind: AtomEnum, data: &[u32]) {
    with_conn(|s| -> Option<()> {
        let atom = cached_atom(&s.conn, property)?;
        s.conn.change_property32(PropMode::REPLACE, window, atom, kind, data).ok()?;
        s.conn.flush().ok()
    });
}

pub fn force_foreground_window(window: NativeWindow) {
    let NativeWindow::X11(win) = window else { return };
    with_conn(|s| -> Option<()> {
        let atom = cached_atom(&s.conn, "_NET_ACTIVE_WINDOW")?;
        send_root_message(&s.conn, s.root, win, atom, [1, 0, 0, 0, 0])
    });
}

// Fork (one window): the X11 window the window manager says is active; `None` when it can't say.
fn x11_active_window() -> Option<u32> {
    with_conn(|s| -> Option<u32> {
        let active_atom = cached_atom(&s.conn, "_NET_ACTIVE_WINDOW")?;
        let reply = s.conn.get_property(false, s.root, active_atom, AtomEnum::WINDOW, 0, 1).ok()?.reply().ok()?;
        let active = reply.value32()?.next();
        active
    })
    .flatten()
}

/// Fork (one window): whether `window` is the one in front with the keyboard: the window manager's answer on X11,
/// else (no `_NET_ACTIVE_WINDOW`, as under gamescope, and on Wayland) the window's own focus. `None` when nobody can say.
pub fn window_in_front(window: &slint::Window) -> Option<bool> {
    use slint::winit_030::WinitWindowAccessor;
    let focus = || window.with_winit_window(|w| w.has_focus());
    match native_window(window)? {
        NativeWindow::X11(win) => x11_active_window().filter(|&w| w != 0).map(|active| active == win).or_else(focus),
        NativeWindow::Wayland => focus(),
    }
}

/// Fork (one window): brings `window` back in front and gives it the keyboard, for a direct user action (the
/// main window's "Go back to it"). X11: un-minimize, then `_NET_ACTIVE_WINDOW` with source 2 ("a direct user
/// action", which KWin's focus-stealing prevention honors); Wayland: winit's focus request. What was asked, for the log.
pub fn bring_back(window: &slint::Window) -> String {
    use slint::winit_030::WinitWindowAccessor;
    window.set_minimized(false);
    match native_window(window) {
        Some(NativeWindow::X11(win)) => {
            with_conn(|s| -> Option<()> {
                let atom = cached_atom(&s.conn, "_NET_ACTIVE_WINDOW")?;
                send_root_message(&s.conn, s.root, win, atom, [2, 0, 0, 0, 0])
            });
            format!("x11 window 0x{win:x}")
        }
        Some(NativeWindow::Wayland) => {
            window.with_winit_window(|w| w.focus_window());
            "wayland focus request".into()
        }
        None => "no native window".into(),
    }
}

/// `None` when the X server can't say (no connection, or no `_NET_ACTIVE_WINDOW`, as under gamescope).
fn x11_foreground_is_ours() -> Option<bool> {
    with_conn(|s| -> Option<bool> {
        let active_atom = cached_atom(&s.conn, "_NET_ACTIVE_WINDOW")?;
        let reply = s.conn.get_property(false, s.root, active_atom, AtomEnum::WINDOW, 0, 1).ok()?.reply().ok()?;
        let active = reply.value32()?.next()?;
        let pid_atom = cached_atom(&s.conn, "_NET_WM_PID")?;
        let reply = s.conn.get_property(false, active, pid_atom, AtomEnum::CARDINAL, 0, 1).ok()?.reply().ok()?;
        // An active window with no _NET_WM_PID is "don't know", not "someone else".
        let pid = reply.value32()?.next()?;
        Some(pid == std::process::id())
    })
    .flatten()
}

// Fork: upstream only asked X11 who is in front, so a native Wayland window (or a gamescope session
// with no `_NET_ACTIVE_WINDOW`) never got controller input. Fall back to winit's focus there.
// `windows` is the main window first, then any open dialog: each dialog is its own top-level window
// and takes the focus from the main one on Wayland.
pub fn gamepad_input_allowed(windows: &[&slint::Window]) -> bool {
    use slint::winit_030::WinitWindowAccessor;
    let Some(main) = windows.first() else { return false };
    let x11_answer = x11_answer_for(native_window(main), x11_foreground_is_ours);
    decide_gamepad_input(x11_answer, || windows.iter().map(|w| w.with_winit_window(|w| w.has_focus())))
}

fn x11_answer_for(main: Option<NativeWindow>, ask_x11: impl FnOnce() -> Option<bool>) -> Option<bool> {
    match main {
        // XWayland can still answer for some other X client; it knows nothing about our windows.
        Some(NativeWindow::Wayland) => None,
        // The X11 check is per process, so it already covers dialogs.
        _ => ask_x11(),
    }
}

fn decide_gamepad_input<I: Iterator<Item = Option<bool>>>(x11_answer: Option<bool>, window_focus: impl FnOnce() -> I) -> bool {
    match x11_answer {
        Some(ours) => ours,
        None => window_focus().any(|focused| focused == Some(true)),
    }
}

// Fork: Game Mode has no tray host, so a window hidden "to the tray" can never come back. Slint
// can't tell us: a tray that fails to start only logs a line, and `show()` still returns Ok. So we
// look for the host ourselves; callers skip the tray (and quit on Close) when this is false.
pub fn tray_host_available() -> bool {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok();
    decide_tray_host(desktop.as_deref(), status_notifier_watcher_running)
}

fn decide_tray_host(current_desktop: Option<&str>, watcher_running: impl FnOnce() -> Option<bool>) -> bool {
    let desktop = current_desktop.filter(|d| !d.is_empty());
    if desktop.is_some_and(|d| d.to_ascii_lowercase().contains("gamescope")) {
        return false;
    }
    // When busctl can't answer, a named desktop keeps upstream's assumption of a tray; no desktop
    // at all means a bare session, where hiding the window would lose it.
    watcher_running().unwrap_or(desktop.is_some())
}

fn status_notifier_watcher_running() -> Option<bool> {
    use std::process::{Command, Stdio};
    // Runs on the UI thread at startup; `timeout` caps it even if the session bus never answers.
    let status = Command::new("timeout")
        .args(["3", "busctl", "--user", "--no-pager", "--timeout=2", "status", "org.kde.StatusNotifierWatcher"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    match status.code() {
        // 124: timed out, 125: timeout itself failed, 126/127: busctl missing or not runnable.
        // None of those says "no tray".
        Some(124..=127) | None => None,
        Some(code) => Some(code == 0),
    }
}

pub fn show_startup_error(message: &str) {
    use std::process::Command;
    let title = "Ports Launcher";
    let shown = |mut cmd: Command| cmd.status().is_ok_and(|s| s.success());
    if shown({
        let mut c = Command::new("zenity");
        c.args(["--error", "--no-markup", "--title", title, "--text", message]);
        c
    }) || shown({
        let mut c = Command::new("kdialog");
        c.args(["--title", title, "--error", message]);
        c
    }) {
        return;
    }
    eprintln!("{title}: {message}");
}

pub fn apply_window_icon(window: NativeWindow) {
    let NativeWindow::X11(win) = window else { return };
    let Some((rgba, width, height)) = extract_app_icon_rgba() else { return };
    let mut data = Vec::with_capacity(2 + (width * height) as usize);
    data.extend([width, height]);
    data.extend(rgba.as_chunks::<4>().0.iter().map(|&[r, g, b, a]| u32::from_be_bytes([a, r, g, b])));
    set_property32(win, "_NET_WM_ICON", AtomEnum::CARDINAL, &data);
}

pub fn enable_dark_context_menus() {}

pub fn extract_app_icon_rgba() -> Option<(Vec<u8>, u32, u32)> {
    ICON.with(|cell| cell.get_or_init(decode_app_icon_rgba).clone())
}

fn decode_app_icon_rgba() -> Option<(Vec<u8>, u32, u32)> {
    let ico = include_bytes!("../../Icon.ico");
    let count = u16::from_le_bytes([*ico.get(4)?, *ico.get(5)?]) as usize;
    let dimension = |b: u8| if b == 0 { 256u32 } else { b as u32 };
    let mut best: Option<(u32, u32, u32)> = None;
    for i in 0..count {
        let entry = ico.get(6 + i * 16..6 + i * 16 + 16)?;
        let area = dimension(entry[0]) * dimension(entry[1]);
        let size = u32::from_le_bytes(entry[8..12].try_into().ok()?);
        let offset = u32::from_le_bytes(entry[12..16].try_into().ok()?);
        if best.is_none_or(|(best_area, ..)| area > best_area) {
            best = Some((area, size, offset));
        }
    }
    let (_, size, offset) = best?;
    let png_bytes = ico.get(offset as usize..(offset + size) as usize)?;
    let image = image::load_from_memory(png_bytes).ok()?.to_rgba8();
    let (width, height) = image.dimensions();
    Some((image.into_raw(), width, height))
}

pub fn force_normal_window_visibility(window: NativeWindow) {
    let NativeWindow::X11(win) = window else { return };
    let Some(normal) = with_conn(|s| cached_atom(&s.conn, "_NET_WM_WINDOW_TYPE_NORMAL")).flatten() else { return };
    set_property32(win, "_NET_WM_WINDOW_TYPE", AtomEnum::ATOM, &[normal]);
}

pub fn begin_window_drag(window: &slint::Window) -> bool {
    const MOVERESIZE_MOVE: u32 = 8;
    match native_window(window) {
        Some(NativeWindow::X11(win)) => {
            with_conn(|s| -> Option<()> {
                let pointer = s.conn.query_pointer(s.root).ok()?.reply().ok()?;
                let moveresize = cached_atom(&s.conn, "_NET_WM_MOVERESIZE")?;
                send_root_message(&s.conn, s.root, win, moveresize, [pointer.root_x as u32, pointer.root_y as u32, MOVERESIZE_MOVE, 1, 1])
            });
            false
        }
        Some(NativeWindow::Wayland) => {
            use slint::winit_030::WinitWindowAccessor;
            window.with_winit_window(|winit_window| winit_window.drag_window().is_ok()).unwrap_or(false)
        }
        None => false,
    }
}

pub fn cursor_position() -> Option<(i32, i32)> {
    with_conn(|s| s.conn.query_pointer(s.root).ok()?.reply().ok().map(|r| (r.root_x as i32, r.root_y as i32))).flatten()
}

pub fn is_hidden_or_minimized(window: NativeWindow) -> bool {
    let NativeWindow::X11(win) = window else { return false };
    with_conn(|s| s.conn.get_window_attributes(win).ok()?.reply().ok().map(|a| a.map_state != MapState::VIEWABLE)).flatten().unwrap_or(false)
}

pub fn own_window(window: NativeWindow, owner: NativeWindow) {
    let (NativeWindow::X11(win), NativeWindow::X11(owner_win)) = (window, owner) else { return };
    set_property32(win, "WM_TRANSIENT_FOR", AtomEnum::WINDOW, &[owner_win]);
}

pub fn double_click_time_ms() -> u32 {
    500
}

pub fn restore_window(window: &slint::Window) {
    match native_window(window) {
        Some(NativeWindow::X11(win)) => {
            let _ = window.show();
            window.set_minimized(false);
            force_foreground_window(NativeWindow::X11(win));
        }
        _ => {
            let fullscreen = window.is_fullscreen();
            let _ = window.hide();
            let _ = window.show();
            window.set_fullscreen(fullscreen);
        }
    }
}

pub fn set_fullscreen(window: &slint::Window, on: bool) {
    window.set_fullscreen(on);
}

pub fn default_font_family() -> String {
    String::new()
}

pub fn work_area_under_cursor() -> (i32, i32, i32, i32) {
    with_conn(|s| {
        let workarea = || -> Option<(i32, i32, i32, i32)> {
            let atom = cached_atom(&s.conn, "_NET_WORKAREA")?;
            let reply = s.conn.get_property(false, s.root, atom, AtomEnum::CARDINAL, 0, 4).ok()?.reply().ok()?;
            let mut values = reply.value32()?;
            let (x, y, w, h) = (values.next()?, values.next()?, values.next()?, values.next()?);
            (w > 0 && h > 0).then_some((x as i32, y as i32, w as i32, h as i32))
        };
        workarea().unwrap_or((0, 0, s.screen_width, s.screen_height))
    })
    .unwrap_or((0, 0, 1920, 1080))
}

pub fn scale_factor_under_cursor() -> f32 {
    1.0
}

fn xdg_data_home() -> Option<PathBuf> {
    match std::env::var("XDG_DATA_HOME") {
        Ok(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => std::env::var("HOME").ok().map(|home| PathBuf::from(home).join(".local").join("share")),
    }
}

fn write_if_different(path: &Path, content: &[u8]) {
    if fs::read(path).ok().as_deref() == Some(content) {
        return;
    }
    if let Some(parent) = path.parent() {
        if fs::create_dir_all(parent).is_ok() {
            let _ = fs::write(path, content);
        }
    }
}

fn desktop_exec_arg(path: &str) -> String {
    let mut escaped = String::with_capacity(path.len() + 2);
    escaped.push('"');
    for c in path.chars() {
        match c {
            '"' | '`' | '$' | '\\' => {
                escaped.push('\\');
                escaped.push(c);
            }
            '%' => escaped.push_str("%%"),
            _ => escaped.push(c),
        }
    }
    escaped.push('"');
    escaped
}

// Fork: the menu entry and icon get this fork's own names, so the original Ports Launcher (which writes
// `ports_launcher.desktop` / `ports_launcher.png`) and this fork no longer overwrite each other's entry.
// The one-file installer (tools/installer.sh) writes the same bytes; keep the two in step.
const DESKTOP_ENTRY_FILE: &str = "steamdeck-ports-mgr.desktop";
const ICON_NAME: &str = "steamdeck-ports-mgr";
const UPSTREAM_ENTRY_FILE: &str = "ports_launcher.desktop";
const UPSTREAM_ICON_NAME: &str = "ports_launcher";

fn desktop_entry_text(exe: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Deck Ports Manager\nExec={}\nIcon={ICON_NAME}\nCategories=Game;\nTerminal=false\nStartupWMClass=ports_launcher\n",
        desktop_exec_arg(exe)
    )
}

/// True when an upstream-named entry was written by this very program (its Exec line is ours), so it
/// can go. An entry for any other program, the original Ports Launcher included, is never ours.
fn upstream_entry_is_ours(entry: &str, exe: &str) -> bool {
    let ours = format!("Exec={}", desktop_exec_arg(exe));
    entry.lines().any(|line| line.trim_end() == ours)
}

/// One-time move from upstream's names: remove `ports_launcher.desktop` only when it points at this
/// program, then the old icon only when no entry left in `applications/` still uses it.
fn migrate_upstream_entry(data_home: &Path, exe: &str) {
    let apps = data_home.join("applications");
    let old = apps.join(UPSTREAM_ENTRY_FILE);
    let Ok(text) = fs::read_to_string(&old) else { return };
    if !upstream_entry_is_ours(&text, exe) || fs::remove_file(&old).is_err() {
        return;
    }
    let icon_line = format!("Icon={UPSTREAM_ICON_NAME}");
    let Ok(entries) = fs::read_dir(&apps) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
            continue;
        }
        // Unreadable means unsure: leave the icon.
        let Ok(other) = fs::read_to_string(&path) else { return };
        if other.lines().any(|line| line.trim_end() == icon_line) {
            return;
        }
    }
    remove_icon_sizes(&data_home.join("icons").join("hicolor"), UPSTREAM_ICON_NAME, None);
}

pub fn register_desktop_entry() {
    let Some(data_home) = xdg_data_home() else { return };
    let Ok(exe) = std::env::current_exe() else { return };
    let exe = exe.to_string_lossy();

    write_if_different(&data_home.join("applications").join(DESKTOP_ENTRY_FILE), desktop_entry_text(&exe).as_bytes());
    migrate_upstream_entry(&data_home, &exe);

    let Some((rgba, width, height)) = extract_app_icon_rgba() else { return };
    let Some(img) = image::RgbaImage::from_raw(width, height, rgba) else { return };
    let mut png_bytes = Vec::new();
    if img.write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png).is_err() {
        return;
    }
    let hicolor_dir = data_home.join("icons").join("hicolor");
    let current_size = format!("{width}x{height}");
    write_if_different(&hicolor_dir.join(&current_size).join("apps").join(format!("{ICON_NAME}.png")), &png_bytes);
    remove_icon_sizes(&hicolor_dir, ICON_NAME, Some(&current_size));
}

/// Removes `<size>/apps/<name>.png` from every size folder except `keep`.
fn remove_icon_sizes(hicolor_dir: &Path, name: &str, keep: Option<&str>) {
    let Ok(entries) = fs::read_dir(hicolor_dir) else { return };
    for entry in entries.flatten().filter(|e| keep.is_none() || e.file_name().to_str() != keep) {
        let _ = fs::remove_file(entry.path().join("apps").join(format!("{name}.png")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_exec_arg_echappe_les_caracteres_reserves() {
        assert_eq!(desktop_exec_arg("/opt/Ports Launcher/ports_launcher"), "\"/opt/Ports Launcher/ports_launcher\"");
        assert_eq!(desktop_exec_arg("/a$b/100%/c\"d"), "\"/a\\$b/100%%/c\\\"d\"");
    }

    #[test]
    fn desktop_entry_has_the_forks_own_name_and_icon() {
        assert_eq!(DESKTOP_ENTRY_FILE, "steamdeck-ports-mgr.desktop");
        assert_eq!(
            desktop_entry_text("/home/u/Applications/steamdeck-ports-mgr/ports_launcher"),
            "[Desktop Entry]\nType=Application\nName=Deck Ports Manager\nExec=\"/home/u/Applications/steamdeck-ports-mgr/ports_launcher\"\nIcon=steamdeck-ports-mgr\nCategories=Game;\nTerminal=false\nStartupWMClass=ports_launcher\n"
        );
    }

    fn upstream_entry(exe: &str) -> String {
        format!(
            "[Desktop Entry]\nType=Application\nName=Ports Launcher\nExec={}\nIcon=ports_launcher\nCategories=Game;\nTerminal=false\nStartupWMClass=ports_launcher\n",
            desktop_exec_arg(exe)
        )
    }

    fn seed(data_home: &Path, entry: &str) -> (PathBuf, PathBuf) {
        let apps = data_home.join("applications");
        let icon = data_home.join("icons/hicolor/256x256/apps/ports_launcher.png");
        fs::create_dir_all(&apps).unwrap();
        fs::create_dir_all(icon.parent().unwrap()).unwrap();
        fs::write(apps.join("ports_launcher.desktop"), entry).unwrap();
        fs::write(&icon, b"png").unwrap();
        (apps.join("ports_launcher.desktop"), icon)
    }

    #[test]
    fn migration_removes_our_old_entry_and_its_icon() {
        let dir = tempfile::tempdir().unwrap();
        let exe = "/home/u/Applications/steamdeck-ports-mgr/ports_launcher";
        let (old, icon) = seed(dir.path(), &upstream_entry(exe));
        migrate_upstream_entry(dir.path(), exe);
        assert!(!old.exists());
        assert!(!icon.exists());
    }

    #[test]
    fn migration_never_touches_the_original_apps_entry() {
        let dir = tempfile::tempdir().unwrap();
        let entry = upstream_entry("/home/u/Applications/Ports Launcher/ports_launcher");
        let (old, icon) = seed(dir.path(), &entry);
        migrate_upstream_entry(dir.path(), "/home/u/Applications/steamdeck-ports-mgr/ports_launcher");
        assert_eq!(fs::read_to_string(&old).unwrap(), entry);
        assert!(icon.exists());
    }

    #[test]
    fn migration_keeps_the_old_icon_while_another_entry_uses_it() {
        let dir = tempfile::tempdir().unwrap();
        let exe = "/opt/dpm/ports_launcher";
        let (old, icon) = seed(dir.path(), &upstream_entry(exe));
        fs::write(dir.path().join("applications/other.desktop"), "[Desktop Entry]\nIcon=ports_launcher\n").unwrap();
        migrate_upstream_entry(dir.path(), exe);
        assert!(!old.exists());
        assert!(icon.exists());
    }

    #[test]
    fn upstream_entry_is_ours_matches_the_exact_exec_line_only() {
        let exe = "/a b/ports_launcher";
        assert!(upstream_entry_is_ours(&upstream_entry(exe), exe));
        assert!(!upstream_entry_is_ours(&upstream_entry("/a b/ports_launcher2"), exe));
        assert!(!upstream_entry_is_ours("[Desktop Entry]\n", exe));
    }

    fn focus(windows: &[Option<bool>]) -> impl FnOnce() -> std::vec::IntoIter<Option<bool>> {
        let windows = windows.to_vec();
        move || windows.into_iter()
    }

    #[test]
    fn gamepad_input_follows_x11_when_it_answers() {
        assert!(decide_gamepad_input(Some(true), focus(&[Some(false)])));
        assert!(!decide_gamepad_input(Some(false), focus(&[Some(true)])));
    }

    #[test]
    fn gamepad_input_falls_back_to_window_focus_without_x11() {
        assert!(decide_gamepad_input(None, focus(&[Some(true)])));
        assert!(!decide_gamepad_input(None, focus(&[Some(false)])));
        assert!(!decide_gamepad_input(None, focus(&[None])));
        assert!(!decide_gamepad_input(None, focus(&[])));
    }

    #[test]
    fn a_focused_dialog_keeps_the_controller_working_on_wayland() {
        // Main window lost focus to its dialog: the dialog's focus is enough.
        assert!(decide_gamepad_input(None, focus(&[Some(false), Some(true)])));
        assert!(!decide_gamepad_input(None, focus(&[Some(false), Some(false)])));
    }

    #[test]
    fn no_tray_in_game_mode_even_if_a_watcher_answers() {
        assert!(!decide_tray_host(Some("gamescope"), || Some(true)));
        assert!(!decide_tray_host(Some("Gamescope:KDE"), || Some(true)));
    }

    #[test]
    fn tray_follows_the_status_notifier_watcher_on_a_desktop() {
        assert!(decide_tray_host(Some("KDE"), || Some(true)));
        assert!(!decide_tray_host(Some("KDE"), || Some(false)));
        assert!(!decide_tray_host(None, || Some(false)));
        assert!(decide_tray_host(None, || Some(true)));
    }

    #[test]
    fn without_busctl_only_a_named_desktop_gets_a_tray() {
        assert!(decide_tray_host(Some("KDE"), || None));
        assert!(!decide_tray_host(None, || None));
        assert!(!decide_tray_host(Some(""), || None));
    }

    #[test]
    fn each_install_folder_gets_its_own_instance_socket() {
        let run = std::env::temp_dir();
        let fork = instance_socket_path_for(Some(run.clone()), "deck", Path::new("/home/deck/Applications/steamdeck-ports-mgr"));
        let original = instance_socket_path_for(Some(run.clone()), "deck", Path::new("/home/deck/Applications/Ports-Launcher"));
        assert_ne!(fork, original);
        assert_eq!(fork, instance_socket_path_for(Some(run.clone()), "deck", Path::new("/home/deck/Applications/steamdeck-ports-mgr")));
        assert_ne!(fork, run.join("ports_launcher.sock"), "never the name upstream's copy uses");
        let name = fork.file_name().unwrap().to_str().unwrap().to_string();
        assert!(name.starts_with("ports_launcher-") && name.ends_with(".sock") && name.len() == "ports_launcher-".len() + 16 + ".sock".len(), "{name}");
        let fallback = instance_socket_path_for(None, "deck", Path::new("/a"));
        assert!(fallback.starts_with(std::env::temp_dir()) && fallback.to_str().unwrap().contains("ports_launcher-deck-"), "{}", fallback.display());
        // FNV-1a of the empty input is its offset basis: the tag does not depend on the Rust version.
        assert_eq!(install_folder_tag(Path::new("")), "cbf29ce484222325");
    }

    #[test]
    fn a_second_start_from_the_same_folder_hands_over_and_another_folder_does_not() {
        use std::os::unix::net::{UnixListener, UnixStream};
        let dir = std::env::temp_dir().join(format!("dpm_instance_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let first = instance_socket_path_for(Some(dir.clone()), "u", Path::new("/opt/a"));
        let _listener = UnixListener::bind(&first).unwrap();
        assert!(UnixStream::connect(instance_socket_path_for(Some(dir.clone()), "u", Path::new("/opt/a"))).is_ok());
        assert!(UnixStream::connect(instance_socket_path_for(Some(dir.clone()), "u", Path::new("/opt/b"))).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_listening_socket_is_found_in_proc_net_unix() {
        let table = "Num       RefCount Protocol Flags    Type St Inode Path\n\
            0000000000000000: 00000003 00000000 00000000 0001 03 51234 /run/user/1000/ports_launcher.sock\n\
            0000000000000000: 00000002 00000000 00010000 0001 01 51200 /run/user/1000/ports_launcher.sock\n\
            0000000000000000: 00000002 00000000 00010000 0001 01 51300 /run/user/1000/other.sock\n";
        assert_eq!(listening_inode(table, Path::new("/run/user/1000/ports_launcher.sock")).as_deref(), Some("51200"));
        assert_eq!(listening_inode(table, Path::new("/run/user/1000/missing.sock")), None);
    }

    #[test]
    fn the_owner_of_a_live_socket_is_this_test_binary() {
        let dir = std::env::temp_dir().join(format!("dpm_legacy_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ports_launcher.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let inode = listening_inode(&fs::read_to_string("/proc/net/unix").unwrap(), &path).expect("listening socket listed");
        assert_eq!(socket_owner_exe(&inode), std::env::current_exe().ok());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_a_copy_from_this_folder_on_the_old_socket_gets_the_hand_over() {
        let ours = Path::new("/home/deck/Applications/steamdeck-ports-mgr");
        assert!(hand_over_to_legacy(Some(&ours.join("ports_launcher")), ours));
        assert!(!hand_over_to_legacy(Some(Path::new("/home/deck/Applications/Ports-Launcher/ports_launcher")), ours));
        assert!(hand_over_to_legacy(None, ours), "unknown owner: hand over");
    }

    #[test]
    fn wayland_window_ignores_what_xwayland_says() {
        assert_eq!(x11_answer_for(Some(NativeWindow::Wayland), || Some(false)), None);
        assert_eq!(x11_answer_for(Some(NativeWindow::Wayland), || Some(true)), None);
        assert_eq!(x11_answer_for(Some(NativeWindow::X11(7)), || Some(false)), Some(false));
        assert_eq!(x11_answer_for(None, || Some(true)), Some(true));
    }
}
