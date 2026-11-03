//! `fake-hc --socket <path> [--exit-on-stdin-eof]`: the fake herdr-core server
//! of `test_support::fake_hc` as a process, for tests that need a core server
//! of their own (its environ, its pid). It seeds one live terminal, `t1`.
//! It exits on SIGTERM, or when stdin reaches EOF if asked to.

use herdr_threads::test_support::fake_hc::{FakeHcState, FakeTerminal, serve};
use std::{
    io::Read,
    os::unix::net::UnixListener,
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool, atomic::Ordering},
};

fn main() {
    let mut socket: Option<PathBuf> = None;
    let mut stdin_eof = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" => socket = args.next().map(PathBuf::from),
            "--exit-on-stdin-eof" => stdin_eof = true,
            other => {
                eprintln!("fake-hc: unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    let Some(socket) = socket else {
        eprintln!("usage: fake-hc --socket <path> [--exit-on-stdin-eof]");
        std::process::exit(2);
    };
    let listener = match UnixListener::bind(&socket) {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("fake-hc: cannot bind {}: {e}", socket.display());
            std::process::exit(1);
        }
    };
    let state = Arc::new(FakeHcState::new());
    state.add_terminal(FakeTerminal::live("t1"));
    let stop = Arc::new(AtomicBool::new(false));
    if stdin_eof {
        let stop = stop.clone();
        std::thread::spawn(move || {
            let mut sink = Vec::new();
            let _ = std::io::stdin().read_to_end(&mut sink);
            stop.store(true, Ordering::SeqCst);
        });
    }
    serve(listener, state, stop);
    let _ = std::fs::remove_file(&socket);
}
