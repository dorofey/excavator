//! Application-owned PTY boundary. All process and terminal I/O runs on workers.
use portable_pty::{CommandBuilder, PtySize};
use std::{
    ffi::OsString,
    io::{Read, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI32, Ordering},
        mpsc::{self, SyncSender},
    },
    thread,
    time::Duration,
};

#[derive(Clone)]
pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: Option<PathBuf>,
}
impl Launch {
    pub fn local(cwd: PathBuf) -> Self {
        let shell = std::env::var_os("SHELL")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| PathBuf::from("/bin/zsh"));
        Self {
            program: shell,
            args: vec!["-l".into()],
            cwd: Some(cwd),
        }
    }
    pub fn command(program: PathBuf, args: Vec<OsString>, cwd: Option<PathBuf>) -> Self {
        Self { program, args, cwd }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Color {
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}
#[derive(Clone, Debug)]
pub struct Cell {
    pub text: String,
    pub foreground: Color,
    pub background: Color,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
    pub wide: bool,
    pub continuation: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Starting,
    Running,
    Exited(u32),
    Failed(String),
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub revision: u64,
    pub rows: u16,
    pub cols: u16,
    pub cells: Vec<Cell>,
    pub cursor: (u16, u16),
    pub hide_cursor: bool,
    pub application_cursor: bool,
    pub bracketed_paste: bool,
    pub alternate_screen: bool,
    pub scrollback: usize,
    pub status: Status,
}
impl Snapshot {
    pub fn text(&self) -> String {
        self.cells
            .chunks(self.cols as usize)
            .map(|row| {
                row.iter()
                    .filter(|c| !c.continuation)
                    .map(|c| {
                        if c.text.is_empty() {
                            " "
                        } else {
                            c.text.as_str()
                        }
                    })
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
enum Control {
    Input(Vec<u8>),
    Resize(u16, u16),
    Scroll(i32),
}
pub struct Session {
    sender: SyncSender<Control>,
    snapshot: Arc<Mutex<Snapshot>>,
    stop: Arc<AtomicBool>,
}
impl Session {
    pub fn spawn(launch: Launch, rows: u16, cols: u16) -> Result<Self, String> {
        let (rows, cols) = dimensions(rows, cols);
        let parser = vt100::Parser::new(rows, cols, 2000);
        let snapshot = Arc::new(Mutex::new(capture(parser.screen(), 0, Status::Starting)));
        let stop = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::sync_channel(128);
        let shared = snapshot.clone();
        let stopping = stop.clone();
        thread::Builder::new()
            .name("excavator-terminal".into())
            .spawn(move || {
                if let Err(error) = run(launch, rows, cols, receiver, &shared, &stopping) {
                    if let Ok(mut state) = shared.lock() {
                        state.revision += 1;
                        state.status = Status::Failed(error);
                    }
                }
            })
            .map_err(|e| format!("Cannot start terminal worker: {e}"))?;
        Ok(Self {
            sender,
            snapshot,
            stop,
        })
    }
    /// Read a complete snapshot for diagnostics and explicit callers.
    #[allow(dead_code)]
    pub fn snapshot(&self) -> Option<Snapshot> {
        self.snapshot_since(None)
    }
    /// Clone the cell grid only after output or terminal state actually changes.
    pub fn snapshot_since(&self, revision: Option<u64>) -> Option<Snapshot> {
        let state = self.snapshot.try_lock().ok()?;
        (revision != Some(state.revision)).then(|| state.clone())
    }
    pub fn input(&self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > 65536 {
            return Err("Terminal input exceeds 64 KiB; paste smaller chunks".into());
        }
        self.send(Control::Input(bytes.to_vec()))
    }
    pub fn resize(&self, rows: u16, cols: u16) -> Result<(), String> {
        let (r, c) = dimensions(rows, cols);
        self.send(Control::Resize(r, c))
    }
    pub fn scroll(&self, rows: i32) -> Result<(), String> {
        self.send(Control::Scroll(rows))
    }
    pub fn shutdown(&self) -> Result<(), String> {
        self.stop.store(true, Ordering::Release);
        Ok(())
    }
    fn send(&self, control: Control) -> Result<(), String> {
        self.sender.try_send(control).map_err(|e| match e {
            mpsc::TrySendError::Full(_) => "Terminal is busy; retry input".into(),
            mpsc::TrySendError::Disconnected(_) => "Terminal session has ended".into(),
        })
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}
fn dimensions(rows: u16, cols: u16) -> (u16, u16) {
    (rows.clamp(2, 160), cols.clamp(2, 400))
}
fn size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}
fn color(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Default,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}
fn capture(screen: &vt100::Screen, revision: u64, status: Status) -> Snapshot {
    let (rows, cols) = screen.size();
    let mut cells = Vec::with_capacity(rows as usize * cols as usize);
    for row in 0..rows {
        for col in 0..cols {
            let c = screen.cell(row, col).unwrap();
            cells.push(Cell {
                text: c.contents().into(),
                foreground: color(c.fgcolor()),
                background: color(c.bgcolor()),
                bold: c.bold(),
                dim: c.dim(),
                italic: c.italic(),
                underline: c.underline(),
                inverse: c.inverse(),
                wide: c.is_wide(),
                continuation: c.is_wide_continuation(),
            });
        }
    }
    Snapshot {
        revision,
        rows,
        cols,
        cells,
        cursor: screen.cursor_position(),
        hide_cursor: screen.hide_cursor(),
        application_cursor: screen.application_cursor(),
        bracketed_paste: screen.bracketed_paste(),
        alternate_screen: screen.alternate_screen(),
        scrollback: screen.scrollback(),
        status,
    }
}
#[derive(Default)]
struct Replies(Vec<u8>);
impl vt100::Callbacks for Replies {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        if self.0.len() > 4096 {
            return;
        }
        let value = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
        if i2.is_none() && c == 'n' && value == 6 {
            let (row, col) = screen.cursor_position();
            let prefix = if i1 == Some(b'?') { "?" } else { "" };
            self.0
                .extend_from_slice(format!("\x1b[{prefix}{};{}R", row + 1, col + 1).as_bytes());
        } else if i1.is_none() && i2.is_none() && c == 'n' && value == 5 {
            self.0.extend_from_slice(b"\x1b[0n");
        } else if i1.is_none() && i2.is_none() && c == 'c' && value == 0 {
            self.0.extend_from_slice(b"\x1b[?1;2c");
        }
    }
}
fn publish(parser: &vt100::Parser<Replies>, shared: &Arc<Mutex<Snapshot>>, status: Status) {
    if let Ok(mut s) = shared.lock() {
        *s = capture(parser.screen(), s.revision + 1, status);
    }
}
fn run(
    launch: Launch,
    rows: u16,
    cols: u16,
    controls: mpsc::Receiver<Control>,
    shared: &Arc<Mutex<Snapshot>>,
    stop: &Arc<AtomicBool>,
) -> Result<(), String> {
    let pair = portable_pty::native_pty_system()
        .openpty(size(rows, cols))
        .map_err(|e| format!("Cannot create terminal: {e}"))?;
    let mut command = CommandBuilder::new(launch.program);
    command.args(launch.args);
    if let Some(cwd) = launch.cwd {
        command.cwd(cwd);
    }
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    let mut child = pair
        .slave
        .spawn_command(command)
        .map_err(|e| format!("Cannot start terminal process: {e}"))?;
    drop(pair.slave);
    let mut killer = child.clone_killer();
    let cancellation = stop.clone();
    let process_id = child.process_id().unwrap_or(0) as i32;
    let foreground = Arc::new(AtomicI32::new(0));
    let foreground_cancel = foreground.clone();
    // The gate prevents signaling a PID/group after it has been reaped and reused.
    let cancellation_done = Arc::new(Mutex::new(false));
    let done = cancellation_done.clone();
    thread::spawn(move || {
        loop {
            let finished = done.lock().unwrap();
            if *finished {
                break;
            }
            if cancellation.load(Ordering::Acquire) {
                terminate_groups(
                    process_id,
                    foreground_cancel.load(Ordering::Acquire),
                    libc::SIGHUP,
                );
                let _ = killer.kill();
                thread::sleep(Duration::from_millis(50));
                terminate_groups(
                    process_id,
                    foreground_cancel.load(Ordering::Acquire),
                    libc::SIGKILL,
                );
                // Keep the gate held while signals are delivered. Reaping is worker-owned.
                drop(finished);
                break;
            }
            drop(finished);
            thread::sleep(Duration::from_millis(100));
        }
    });
    let result = (|| {
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("Cannot read terminal: {e}"))?;
        let mut writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("Cannot write terminal: {e}"))?;
        let (output_tx, output_rx) = mpsc::sync_channel::<Result<Vec<u8>, String>>(16);
        thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        if output_tx.send(Ok(buffer[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        if e.raw_os_error() != Some(5) {
                            let _ = output_tx.send(Err(e.to_string()));
                        }
                        break;
                    }
                }
            }
        });
        let mut parser = vt100::Parser::new_with_callbacks(rows, cols, 2000, Replies::default());
        publish(&parser, shared, Status::Running);
        loop {
            if let Some(fd) = pair.master.as_raw_fd() {
                // This descriptor belongs to this session's controlling terminal.
                let group = unsafe { libc::tcgetpgrp(fd) };
                foreground.store(group, Ordering::Release);
            }
            if stop.load(Ordering::Acquire) {
                break;
            }
            match controls.recv_timeout(Duration::from_millis(30)) {
                Ok(Control::Input(bytes)) => {
                    parser.screen_mut().set_scrollback(0);
                    writer
                        .write_all(&bytes)
                        .map_err(|e| format!("Terminal write failed: {e}"))?;
                    writer
                        .flush()
                        .map_err(|e| format!("Terminal flush failed: {e}"))?;
                    publish(&parser, shared, Status::Running);
                }
                Ok(Control::Resize(r, c)) => {
                    pair.master
                        .resize(size(r, c))
                        .map_err(|e| format!("Terminal resize failed: {e}"))?;
                    parser.screen_mut().set_size(r, c);
                    publish(&parser, shared, Status::Running);
                }
                Ok(Control::Scroll(delta)) => {
                    let offset = parser
                        .screen()
                        .scrollback()
                        .saturating_add_signed(delta as isize);
                    parser.screen_mut().set_scrollback(offset);
                    publish(&parser, shared, Status::Running);
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            let mut changed = false;
            // Bounded work per pass prevents endless output from starving cancellation/input.
            for _ in 0..16 {
                match output_rx.try_recv() {
                    Ok(Ok(bytes)) => {
                        parser.process(&bytes);
                        changed = true;
                    }
                    Ok(Err(error)) => return Err(format!("Terminal read failed: {error}")),
                    Err(_) => break,
                }
            }
            if !parser.callbacks().0.is_empty() {
                let reply = std::mem::take(&mut parser.callbacks_mut().0);
                writer
                    .write_all(&reply)
                    .map_err(|e| format!("Terminal reply failed: {e}"))?;
            }
            if changed {
                publish(&parser, shared, Status::Running);
            }
            let exit_status = {
                let mut finished = cancellation_done.lock().unwrap();
                let status = child
                    .try_wait()
                    .map_err(|e| format!("Cannot inspect terminal process: {e}"))?;
                if status.is_some() {
                    *finished = true;
                }
                status
            };
            if let Some(status) = exit_status {
                // Drain reader to EOF to preserve output emitted immediately before exit.
                let drain_deadline = std::time::Instant::now() + Duration::from_secs(2);
                while std::time::Instant::now() < drain_deadline && !stop.load(Ordering::Acquire) {
                    let Ok(output) = output_rx.recv_timeout(Duration::from_millis(100)) else {
                        break;
                    };
                    if let Ok(bytes) = output {
                        parser.process(&bytes);
                    } else {
                        break;
                    }
                }
                publish(&parser, shared, Status::Exited(status.exit_code()));
                return Ok(());
            }
        }
        Ok(())
    })();
    // Natural-exit try_wait already reaped the child and published its status.
    // Never signal identifiers after reaping: the OS may immediately reuse them.
    let mut finished = cancellation_done.lock().unwrap();
    if !*finished {
        terminate_groups(process_id, foreground.load(Ordering::Acquire), libc::SIGHUP);
        terminate_groups(
            process_id,
            foreground.load(Ordering::Acquire),
            libc::SIGKILL,
        );
        let _ = child.kill();
        let status = child.wait();
        *finished = true;
        if result.is_ok() {
            if let Ok(mut s) = shared.lock() {
                s.revision += 1;
                s.status = Status::Exited(status.map(|s| s.exit_code()).unwrap_or(1));
            }
        }
    }
    drop(finished);
    result
}

/// Signal only groups proven to be this terminal's shell or foreground job.
fn terminate_groups(pid: i32, foreground: i32, signal: i32) {
    unsafe {
        let host_group = libc::getpgrp();
        if pid > 1
            && foreground > 1
            && foreground != host_group
            && libc::getpgid(foreground) == foreground
            && libc::getsid(foreground) == pid
        {
            let _ = libc::kill(-foreground, signal);
        }
        if pid > 1 && libc::getpgid(pid) == pid && pid != host_group {
            let _ = libc::kill(-pid, signal);
        }
    }
}
