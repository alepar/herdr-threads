use herdr_threads::{
    app::{SystemClock, run_elected_guarded},
    cli,
    daemon::paths::{NodePaths, RuntimeContext},
    protocol::{
        ids::HostLocator,
        time::{Cancellation, Clock},
    },
    service::{
        config::ServiceConfig,
        host_sessions::{HostAttachPlan, HostFactory},
    },
};
use std::{
    ffi::OsString,
    io::{self, IsTerminal, Write},
    path::PathBuf,
    sync::Arc,
};

fn detached_child(args: &[OsString]) -> io::Result<()> {
    // Test builds: exit once the owning test process is gone (owner_watch).
    #[cfg(feature = "test-support")]
    herdr_threads::test_support::owner_watch::watch_from_env();
    // `daemon run --state-dir D [--host-endpoint H]`: the host is an attach hint.
    let host = match args.len() {
        5 => None,
        7 if args[5] == "--host-endpoint" => Some(PathBuf::from(&args[6])),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid detached daemon arguments",
            ));
        }
    };
    if args[3] != "--state-dir" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid detached daemon arguments",
        ));
    }
    let context = RuntimeContext::explicit(
        PathBuf::from(&args[4]),
        host,
        std::env::var_os("HERDR_BIN_PATH").map(PathBuf::from),
    )?;
    let paths = NodePaths::resolve(&context.state_dir)?;
    let config = ServiceConfig::load(&paths)?;
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    // Servers attach after the store opens (spec §3 Startup): the hint first,
    // then every remembered server whose socket exists. The hint is a
    // canonical locator or the start is refused.
    let hint = match context.host_endpoint.clone() {
        Some(endpoint) => Some(
            HostLocator::parse(endpoint.to_string_lossy().into_owned()).map_err(|error| {
                io::Error::new(io::ErrorKind::InvalidInput, error.detail.to_string())
            })?,
        ),
        None => None,
    };
    let factory_clock = Arc::clone(&clock);
    let plan = HostAttachPlan {
        hint,
        factory: HostFactory {
            build: Arc::new(move |locator| {
                let host = Arc::new(herdr_threads::host::native::NativeCli::new(
                    locator.as_path().to_path_buf(),
                    Arc::clone(&factory_clock),
                ));
                (host.clone(), host)
            }),
            exists: Arc::new(|path| {
                use std::os::unix::fs::FileTypeExt;
                std::fs::metadata(path).is_ok_and(|meta| meta.file_type().is_socket())
            }),
        },
    };
    runtime.block_on(run_elected_guarded(
        &paths,
        &context,
        clock,
        Cancellation::default(),
        config,
        plan,
        |_| Ok(()),
    ))?;
    Ok(())
}

fn main() {
    let args: Vec<OsString> = std::env::args_os().collect();
    if args.get(1).is_some_and(|s| s == "daemon") && args.get(2).is_some_and(|s| s == "run") {
        if let Err(error) = detached_child(&args) {
            // Pre-election stderr is the starter's attempt file (or a pipe the
            // starter may already have closed): a failed write must not turn
            // the real error into a panic.
            let _ = writeln!(io::stderr(), "herdr-threads daemon: {error}");
            std::process::exit(2);
        }
        return;
    }
    // Native hook entrypoint: never exits 2, which would block a Claude tool call.
    if let Some(parsed) = cli::hook::parse_hook_argv(&args) {
        std::process::exit(cli::hook::run_process(parsed));
    }
    // `peer --stdio`: the stateless ssh-side pump, kept off the clap path.
    if let Some(peer) = cli::peer::parse_peer_stdio_argv(&args) {
        let code = match RuntimeContext::from_environment(peer.state_dir, peer.host_endpoint) {
            Ok(context) => cli::peer::run_stdio(&context),
            Err(error) => {
                eprintln!("herdr-threads peer: daemon unavailable: {error}");
                cli::peer::EXIT_DAEMON_UNAVAILABLE
            }
        };
        std::process::exit(code);
    }
    let terminal = io::stdout().is_terminal();
    if let Err(error) = cli::run_terminal(args, &mut io::stdout().lock(), terminal) {
        match &error {
            // A command that rendered its own report (doctor) only sets the status.
            cli::RunError::Exit(_) => {}
            // Bare invocation: plain usage, not an error line.
            cli::RunError::Usage(_) => eprintln!("{error}"),
            _ => eprintln!("herdr-threads: {error}"),
        }
        std::process::exit(error.exit_code());
    }
}
