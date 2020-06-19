use actix::Actor;
use std::env;
use structopt::{clap, StructOpt};

mod events;
mod execution;
mod hardware;
mod market;
mod payments;
mod preset_cli;
mod provider_agent;
mod startup_config;
mod task_manager;
mod task_state;

use crate::hardware::Profiles;
use crate::provider_agent::{Initialize, Shutdown};
use provider_agent::ProviderAgent;
use startup_config::{Commands, ExeUnitsConfig, PresetsConfig, ProfileConfig, StartupConfig};

#[actix_rt::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();

    let cli_args = StartupConfig::from_args();
    let mut config = cli_args.config;
    let data_dir = config.data_dir.get_or_create()?;
    config.presets_file = data_dir.join(config.presets_file);
    config.hardware_file = data_dir.join(config.hardware_file);

    match cli_args.commands {
        Commands::Run(args) => {
            env::set_var("RUST_LOG", env::var("RUST_LOG").unwrap_or("info".into()));
            env_logger::init();

            let app_name = clap::crate_name!();
            log::info!("Starting {}...", app_name);

            let agent = ProviderAgent::new(args, config).await?.start();
            agent.send(Initialize).await??;

            let _ = tokio::signal::ctrl_c().await;
            log::info!("SIGINT received, Shutting down {}...", clap::crate_name!());
            agent.send(Shutdown).await??;
            Ok(())
        }
        Commands::Preset(presets_cmd) => match presets_cmd {
            PresetsConfig::List => ProviderAgent::list_presets(config),
            PresetsConfig::Active => ProviderAgent::active_presets(config),
            PresetsConfig::Create {
                no_interactive,
                params,
            } => {
                if no_interactive {
                    ProviderAgent::create_preset(config, params)
                } else {
                    ProviderAgent::create_preset_interactive(config)
                }
            }
            PresetsConfig::Remove { name } => ProviderAgent::remove_preset(config, name),
            PresetsConfig::Update {
                no_interactive,
                params,
                name,
            } => {
                if no_interactive {
                    ProviderAgent::update_preset(config, name, params)
                } else {
                    ProviderAgent::update_preset_interactive(config, name)
                }
            }
            PresetsConfig::Activate { name } => ProviderAgent::activate_preset(config, name),
            PresetsConfig::Deactivate { name } => ProviderAgent::deactivate_preset(config, name),
            PresetsConfig::ListMetrics => ProviderAgent::list_metrics(config),
        },
        Commands::Profile(profile_cmd) => {
            let path = config.hardware_file;
            match profile_cmd {
                ProfileConfig::List => {
                    let profiles = Profiles::load(&path)?.list();
                    println!("{}", serde_json::to_string_pretty(&profiles)?);
                }
                ProfileConfig::Create { name, resources } => {
                    let mut profiles = Profiles::load_or_create(&path)?;
                    if let Some(_) = profiles.get(&name) {
                        return Err(hardware::ProfileError::AlreadyExists(name).into());
                    }
                    profiles.add(name, resources)?;
                    profiles.save(path)?;
                }
                ProfileConfig::Update { name, resources } => {
                    let mut profiles = Profiles::load_or_create(&path)?;
                    match profiles.get_mut(&name) {
                        Some(profile) => *profile = resources,
                        _ => return Err(hardware::ProfileError::Unknown(name).into()),
                    }
                    profiles.save(path)?;
                }
                ProfileConfig::Remove { name } => {
                    let mut profiles = Profiles::load(&path)?;
                    profiles.remove(name)?;
                    profiles.save(path)?;
                }
                ProfileConfig::Activate { name } => {
                    let mut profiles = Profiles::load(&path)?;
                    profiles.set_active(name)?;
                    profiles.save(path)?;
                }
                ProfileConfig::Active => {
                    let profiles = Profiles::load(&path)?;
                    println!("{}", serde_json::to_string_pretty(profiles.active())?);
                }
            }
            Ok(())
        }
        Commands::ExeUnit(exeunit_cmd) => match exeunit_cmd {
            ExeUnitsConfig::List => ProviderAgent::list_exeunits(config),
        },
    }
}
