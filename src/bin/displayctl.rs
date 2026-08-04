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
    /// Rename a stored profile.
    Rename {
        profile_id: String,
        new_name: String,
    },
    /// Only meaningful against `hyprforge-displayd run --mock`: trigger a
    /// fake topology change. `spec` is a comma-separated list of
    /// `make:model:serial` triples, e.g. `BOE:0x0BC9:,DELL:U2720Q:ABC123`.
    SimulateTopology { spec: String },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
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
        Command::Rename {
            profile_id,
            new_name,
        } => rename(&connection, &profile_id, &new_name).await,
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
