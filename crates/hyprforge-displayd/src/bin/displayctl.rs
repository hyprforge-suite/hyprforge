use clap::{Parser, Subcommand};
use hyprforge_core::displayd_proxy::DisplaydProxy;

#[derive(Parser)]
#[command(name = "hyprforge-displayctl")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List stored display profiles.
    ListProfiles,
    /// Print the fingerprint of the currently-connected output set.
    CurrentFingerprint,
    /// Force-apply a profile regardless of the current fingerprint.
    Apply { profile_id: String },
    /// Apply provisionally: the daemon rolls the change back on its own
    /// unless `confirm` arrives first. Prints the seconds you have.
    ApplyReversible { profile_id: String },
    /// Keep the layout a reversible apply put in place, cancelling the
    /// rollback.
    Confirm,
    /// Roll back a provisional layout now, without waiting out the timer.
    Revert,
    /// Rename a stored profile.
    Rename {
        profile_id: String,
        new_name: String,
    },
    /// Forget a stored profile. Note the currently-connected topology is
    /// auto-learned again on the next change, so this is a reset for the
    /// setup you're on and a true forget only for ones you're not.
    Delete { profile_id: String },
    /// Print one profile's full stored geometry as JSON.
    GetProfile { profile_id: String },
    /// Print the currently-applied live layout as JSON.
    CurrentLayout,
    /// List the modes a currently-connected head supports.
    Modes { connector: String },
    /// Toggle a swap override between two stored heads, for
    /// duplicate/blank-serial identities that EDID can't tell apart.
    SwapHeads {
        profile_id: String,
        connector_a: String,
        connector_b: String,
    },
    /// Set what happens to outputs a profile doesn't cover.
    SetPolicy {
        profile_id: String,
        /// One of: extend_right, mirror, disable.
        policy: String,
    },
    /// Set one head's full stored geometry.
    #[command(name = "set-geometry")]
    SetGeometry {
        profile_id: String,
        connector_hint: String,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        /// Vertical refresh in mHz (e.g. 60000 for 60Hz).
        refresh_mhz: i32,
        scale: f64,
        /// One of: Normal, Rotate90, Rotate180, Rotate270, Flipped,
        /// Flipped90, Flipped180, Flipped270.
        transform: String,
    },
    /// Only meaningful against `hyprforge-displayd run --mock`: trigger a
    /// fake topology change. `spec` is a comma-separated list of
    /// `make:model:serial` triples, e.g. `BOE:0x0BC9:,DELL:U2720Q:ABC123`.
    SimulateTopology { spec: String },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Quieter default than the daemon's: this is a one-shot CLI whose real
    // output is on stdout, so logs are opt-in via RUST_LOG.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let cli = Cli::parse();
    let connection = zbus::Connection::session().await.map_err(|e| {
        anyhow::anyhow!(
            "failed to connect to the D-Bus session bus: {e}\n\
             is hyprforge-displayd running? try: systemctl --user status hyprforge-displayd"
        )
    })?;

    match cli.command {
        Command::ListProfiles => list_profiles(&connection).await,
        Command::CurrentFingerprint => current_fingerprint(&connection).await,
        Command::Apply { profile_id } => apply(&connection, &profile_id).await,
        Command::ApplyReversible { profile_id } => {
            let seconds = proxy(&connection)
                .await?
                .apply_profile_reversible(&profile_id)
                .await?;
            println!(
                "applied {profile_id} provisionally — reverting in {seconds}s unless you run \
                 `hyprforge-displayctl confirm`"
            );
            Ok(())
        }
        Command::Confirm => {
            proxy(&connection).await?.confirm_layout().await?;
            println!("layout kept");
            Ok(())
        }
        Command::Revert => {
            proxy(&connection).await?.revert_layout().await?;
            println!("layout reverted");
            Ok(())
        }
        Command::Rename {
            profile_id,
            new_name,
        } => rename(&connection, &profile_id, &new_name).await,
        Command::Delete { profile_id } => delete(&connection, &profile_id).await,
        Command::GetProfile { profile_id } => {
            println!("{}", proxy(&connection).await?.get_profile(&profile_id).await?);
            Ok(())
        }
        Command::CurrentLayout => {
            println!("{}", proxy(&connection).await?.get_current_layout().await?);
            Ok(())
        }
        Command::Modes { connector } => modes(&connection, &connector).await,
        Command::SwapHeads {
            profile_id,
            connector_a,
            connector_b,
        } => {
            proxy(&connection)
                .await?
                .swap_heads(&profile_id, &connector_a, &connector_b)
                .await?;
            println!("toggled swap between {connector_a} and {connector_b}");
            Ok(())
        }
        Command::SetPolicy { profile_id, policy } => {
            proxy(&connection)
                .await?
                .set_extra_output_policy(&profile_id, &policy)
                .await?;
            println!("policy for {profile_id} is now {policy}");
            Ok(())
        }
        Command::SetGeometry {
            profile_id,
            connector_hint,
            x,
            y,
            width,
            height,
            refresh_mhz,
            scale,
            transform,
        } => {
            proxy(&connection)
                .await?
                .set_head_geometry(
                    &profile_id,
                    &connector_hint,
                    x,
                    y,
                    width,
                    height,
                    refresh_mhz,
                    scale,
                    &transform,
                )
                .await?;
            println!("updated {connector_hint} in {profile_id}");
            Ok(())
        }
        Command::SimulateTopology { spec } => simulate_topology(&connection, &spec).await,
    }
}

async fn proxy(connection: &zbus::Connection) -> anyhow::Result<DisplaydProxy<'_>> {
    Ok(DisplaydProxy::new(connection).await?)
}

async fn list_profiles(connection: &zbus::Connection) -> anyhow::Result<()> {
    let p = proxy(connection).await?;
    let profiles = p.list_profiles().await?;
    if profiles.is_empty() {
        println!("No profiles stored yet.");
        return Ok(());
    }
    for (id, name, head_count, last_used) in profiles {
        println!("{id}  {name}  ({head_count} head(s), last used {last_used})");
    }
    Ok(())
}

async fn current_fingerprint(connection: &zbus::Connection) -> anyhow::Result<()> {
    let p = proxy(connection).await?;
    println!("{}", p.get_current_fingerprint().await?);
    Ok(())
}

async fn apply(connection: &zbus::Connection, profile_id: &str) -> anyhow::Result<()> {
    let p = proxy(connection).await?;
    p.apply_profile(profile_id).await?;
    println!("applied {profile_id}");
    Ok(())
}

async fn rename(connection: &zbus::Connection, profile_id: &str, new_name: &str) -> anyhow::Result<()> {
    let p = proxy(connection).await?;
    p.rename_profile(profile_id, new_name).await?;
    println!("renamed {profile_id} to {new_name:?}");
    Ok(())
}

async fn delete(connection: &zbus::Connection, profile_id: &str) -> anyhow::Result<()> {
    let p = proxy(connection).await?;
    p.delete_profile(profile_id).await?;
    println!("deleted {profile_id}");
    Ok(())
}

async fn modes(connection: &zbus::Connection, connector: &str) -> anyhow::Result<()> {
    let p = proxy(connection).await?;
    let modes = p.get_available_modes(connector).await?;
    if modes.is_empty() {
        // Not an error: a profile can name a head that isn't plugged in.
        println!("No modes reported — is {connector} currently connected?");
        return Ok(());
    }
    for (width, height, refresh_mhz, preferred) in modes {
        let star = if preferred { " (preferred)" } else { "" };
        println!(
            "{width}x{height}@{:.3}Hz{star}",
            refresh_mhz as f64 / 1000.0
        );
    }
    Ok(())
}

async fn simulate_topology(connection: &zbus::Connection, spec: &str) -> anyhow::Result<()> {
    let reply = connection
        .call_method(
            Some("dev.hyprforge.Displayd"),
            "/dev/hyprforge/Displayd",
            Some("dev.hyprforge.DisplaydMock1"),
            "SimulateTopology",
            &(spec,),
        )
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "{e}\nsimulate-topology only works against `hyprforge-displayd run --mock`"
            )
        })?;
    reply.body().deserialize::<()>()?;
    println!("simulated topology: {spec}");
    Ok(())
}
