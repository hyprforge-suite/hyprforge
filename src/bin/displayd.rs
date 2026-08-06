use clap::{Parser, Subcommand};
use hyprforge_core::paths::display_profiles_path;
use hyprforge_displayd::backend::mock::MockBackend;
use hyprforge_displayd::backend::wlr::WlrBackend;
use hyprforge_displayd::backend::OutputBackend;
use hyprforge_displayd::daemon::Daemon;
use hyprforge_displayd::dbus::{self, DisplaydService, BUS_NAME, OBJECT_PATH};
use hyprforge_displayd::types::Identity;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "hyprforge-displayd")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the daemon in the foreground.
    Run {
        /// Use the in-memory mock backend instead of a real compositor
        /// connection — for manual testing via `simulate-topology`.
        #[arg(long)]
        mock: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Default to `info` rather than whatever `from_default_env` alone gives
    // with RUST_LOG unset — which is nothing at all. A long-running daemon
    // that prints zero output on start is indistinguishable from a hung one
    // (vision pillar #4: nothing should require faith that it worked).
    // RUST_LOG still overrides this when set.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Run { mock } => run(mock).await,
    }
}

async fn run(mock: bool) -> anyhow::Result<()> {
    let storage_path = display_profiles_path();

    let (backend, mock_backend): (Arc<dyn OutputBackend>, Option<Arc<MockBackend>>) = if mock {
        tracing::info!("starting with mock backend");
        let backend = Arc::new(MockBackend::new());
        (backend.clone(), Some(backend))
    } else {
        tracing::info!("connecting to compositor via wlr-output-management-v1");
        let backend = Arc::new(WlrBackend::connect()?);
        (backend, None)
    };

    let daemon = Arc::new(Daemon::new(backend.clone(), storage_path)?);
    warn_about_competing_monitor_rules(&daemon).await;

    let events = backend.subscribe();
    let (signal_tx, signal_rx) = tokio::sync::mpsc::unbounded_channel();

    let service = DisplaydService::new(daemon.clone());
    let connection = zbus::connection::Builder::session()?
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, service)?
        .build()
        .await?;
    tracing::info!(bus_name = BUS_NAME, object_path = OBJECT_PATH, "D-Bus service registered");

    if let Some(mock_backend) = mock_backend {
        register_mock_simulate_topology(&connection, mock_backend).await?;
        // A mock daemon with no outputs does nothing until told to, which
        // looks identical to a broken one. Say what the next move is.
        tracing::info!(
            "mock backend ready with no outputs connected — drive it from another \
             terminal, e.g. `hyprforge-displayctl simulate-topology \
             'BOE:0x0BC9:,DELL:U2720Q:ABC123'`, then `hyprforge-displayctl list-profiles`"
        );
    }

    let signal_forwarder = tokio::spawn(dbus::forward_signals(connection, signal_rx));
    let daemon_run = daemon.run(events, signal_tx);

    tokio::select! {
        _ = daemon_run => {}
        result = signal_forwarder => {
            if let Err(e) = result {
                tracing::error!(error = %e, "signal forwarding task panicked");
            }
        }
    }

    Ok(())
}

async fn warn_about_competing_monitor_rules(daemon: &Daemon) {
    let hypr_dir = hyprforge_core::paths::hypr_config_dir();
    let mut competing = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&hypr_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("lua") {
                continue;
            }
            let Ok(contents) = std::fs::read_to_string(&path) else {
                continue;
            };
            if contents.contains("hl.monitor(") {
                competing.push(path.display().to_string());
            }
        }
    }
    if !competing.is_empty() {
        tracing::warn!(
            files = ?competing,
            "found hl.monitor() rules in your own Hyprland Lua config; these are \
             re-applied on every `hyprctl reload` and may fight hyprforge-displayd's \
             auto-applied layout. hyprforge-displayd will never edit these files."
        );
    }
    daemon.set_competing_monitor_rules(competing).await;
}

/// Registers a second, mock-only D-Bus method for driving fake topology
/// changes from `hyprforge-displayctl simulate-topology`. Kept out of the
/// main `Displayd1` interface (and thus out of the real backend's surface)
/// since it's only meaningful against the mock backend.
async fn register_mock_simulate_topology(
    connection: &zbus::Connection,
    backend: Arc<MockBackend>,
) -> anyhow::Result<()> {
    struct MockControl {
        backend: Arc<MockBackend>,
    }

    #[zbus::interface(name = "dev.hyprforge.DisplaydMock1")]
    impl MockControl {
        /// `spec` is a comma-separated list of `make:model:serial` triples
        /// (any field may be empty), e.g. `BOE:0x0BC9:,DELL:U2720Q:ABC123`.
        async fn simulate_topology(&self, spec: &str) -> zbus::fdo::Result<()> {
            let identities = parse_topology_spec(spec)
                .map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?;
            self.backend.set_topology(identities);
            Ok(())
        }
    }

    connection
        .object_server()
        .at(OBJECT_PATH, MockControl { backend })
        .await?;
    Ok(())
}

fn parse_topology_spec(spec: &str) -> anyhow::Result<Vec<Identity>> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Ok(Vec::new());
    }
    spec.split(',')
        .map(|entry| {
            let parts: Vec<&str> = entry.splitn(3, ':').collect();
            if parts.len() != 3 {
                anyhow::bail!("invalid topology entry {entry:?}; expected make:model:serial");
            }
            Ok(Identity {
                make: parts[0].to_string(),
                model: parts[1].to_string(),
                serial: parts[2].to_string(),
            })
        })
        .collect()
}
