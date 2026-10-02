use herdr_threads::{
    app::{SystemClock, run_elected},
    cli,
    daemon::paths::{InstancePaths, RuntimeContext},
    protocol::time::{Cancellation, Clock},
    service::config::ServiceConfig,
};
use std::{
    ffi::OsString,
    io::{self, IsTerminal},
    path::PathBuf,
    sync::Arc,
};

fn detached_child(args: &[OsString]) -> io::Result<()> {
    // Test builds: exit once the owning test process is gone (owner_watch).
    #[cfg(feature = "test-support")]
    herdr_threads::test_support::owner_watch::watch_from_env();
    if args.len() != 7 || args[3] != "--state-dir" || args[5] != "--host-endpoint" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid detached daemon arguments",
        ));
    }
    let context = RuntimeContext::explicit(
        PathBuf::from(&args[4]),
        PathBuf::from(&args[6]),
        std::env::var_os("HERDR_BIN_PATH").map(PathBuf::from),
    )?;
    let paths = InstancePaths::resolve(&context)?;
    let config = ServiceConfig::load(&paths)?;
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let host = Arc::new(herdr_threads::host::native::NativeCli::new(
        context.host_endpoint.clone(),
        Arc::clone(&clock),
    ));
    runtime.block_on(run_elected(
        &paths,
        clock,
        Cancellation::default(),
        config,
        host,
        |_| Ok(()),
    ))?;
    Ok(())
}

fn main() {
    let args: Vec<OsString> = std::env::args_os().collect();
    if args.get(1).is_some_and(|s| s == "daemon") && args.get(2).is_some_and(|s| s == "run") {
        if let Err(error) = detached_child(&args) {
            eprintln!("herdr-threads daemon: {error}");
            std::process::exit(2);
        }
        return;
    }
    // Native hook entrypoint: never exits 2, which would block a Claude tool call.
    if let Some(parsed) = cli::hook::parse_hook_argv(&args) {
        std::process::exit(cli::hook::run_process(parsed));
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
