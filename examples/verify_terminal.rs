//! Real PTY acceptance without shell profiles, network, or user-state writes.
#[path = "../src/terminal.rs"]
mod terminal;
use std::{
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
use terminal::{Color, Launch, Session, Status};
fn wait(session: &Session, predicate: impl Fn(&terminal::Snapshot) -> bool) -> terminal::Snapshot {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(snapshot) = session.snapshot() {
            assert!(
                !matches!(snapshot.status, Status::Failed(_)),
                "{:?}",
                snapshot.status
            );
            if predicate(&snapshot) {
                return snapshot;
            }
        }
        assert!(
            Instant::now() < deadline,
            "Terminal acceptance timed out: {:?}",
            session.snapshot().map(|s| (s.status.clone(), s.text()))
        );
        thread::sleep(Duration::from_millis(10));
    }
}
fn main() {
    let directory = std::env::temp_dir().join(format!("excavator-terminal-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let session = Session::spawn(
        Launch::command(
            PathBuf::from("/bin/sh"),
            vec!["-i".into()],
            Some(directory.clone()),
        ),
        20,
        100,
    )
    .unwrap();
    wait(&session, |s| s.status == Status::Running);
    session
        .input(b"printf '\\033[2J\\033[H\\033[31mRED\\033[0m'; printf '\\nCWD='; pwd\r")
        .unwrap();
    let screen = wait(&session, |s| {
        s.text().contains("CWD=") && s.text().contains(directory.to_str().unwrap())
    });
    assert!(
        screen
            .cells
            .iter()
            .any(|c| c.text == "R" && c.foreground == Color::Indexed(1)),
        "ANSI colors must be parsed"
    );
    session
        .input(b"printf '\\033[?2004h\\033[?1h\\033[?1049h\\033[2J\\033[HMODE-READY'\r")
        .unwrap();
    let modes = wait(&session, |s| {
        s.bracketed_paste
            && s.application_cursor
            && s.alternate_screen
            && s.text().contains("MODE-READY")
    });
    assert_eq!(modes.scrollback, 0);
    session
        .input(b"printf '\\033[?1049l\\033[?1l\\033[?2004l'\r")
        .unwrap();
    wait(&session, |s| {
        !s.alternate_screen && !s.application_cursor && !s.bracketed_paste
    });
    session.resize(25, 110).unwrap();
    wait(&session, |s| s.rows == 25 && s.cols == 110);
    session.input(b"stty size; printf 'SIZE-END\\n'\r").unwrap();
    wait(&session, |s| {
        s.text().contains("25 110") && s.text().contains("SIZE-END")
    });
    // Ctrl-C must interrupt a foreground command while preserving the interactive shell.
    session.input(b"sleep 30\r").unwrap();
    thread::sleep(Duration::from_millis(100));
    session.input(&[3]).unwrap();
    session.input(b"printf 'AFTER-INTERRUPT\\n'\r").unwrap();
    wait(&session, |s| s.text().contains("AFTER-INTERRUPT\n"));
    session
        .input(b"printf 'FINAL-MARKER\\n'; exit 7\r")
        .unwrap();
    let final_screen = wait(&session, |s| s.status == Status::Exited(7));
    assert!(final_screen.text().contains("FINAL-MARKER"));
    let cancellation = Session::spawn(
        Launch::command(
            PathBuf::from("/bin/sh"),
            vec!["-i".into()],
            Some(directory.clone()),
        ),
        20,
        100,
    )
    .unwrap();
    wait(&cancellation, |s| s.status == Status::Running);
    cancellation
        .input(b"set +H; printf 'READY-FOR-JOB\\n'\r")
        .unwrap();
    wait(&cancellation, |s| s.text().contains("READY-FOR-JOB\n"));
    cancellation
        .input(b"sleep 30 & printf 'CHILD=%s\\n' \"$!\"; fg\r")
        .unwrap();
    let running = wait(&cancellation, |s| {
        s.text()
            .lines()
            .any(|line| line.starts_with("CHILD=") && line[6..].trim().parse::<i32>().is_ok())
    });
    let child_pid: i32 = running
        .text()
        .lines()
        .find_map(|line| {
            line.strip_prefix("CHILD=")
                .and_then(|pid| pid.trim().parse().ok())
        })
        .unwrap();
    thread::sleep(Duration::from_millis(100));
    cancellation.shutdown().unwrap();
    wait(&cancellation, |s| matches!(s.status, Status::Exited(_)));
    let deadline = Instant::now() + Duration::from_secs(5);
    while unsafe { libc::kill(child_pid, 0) } == 0 {
        assert!(
            Instant::now() < deadline,
            "Foreground sleep survived terminal shutdown"
        );
        thread::sleep(Duration::from_millis(10));
    }
    std::fs::remove_dir(directory).unwrap();
    println!(
        "PTY acceptance passed: cwd, ANSI grid/color, alternate screen/input modes, resize, Ctrl-C, final output/exit status, foreground-job shutdown/reap"
    );
}
