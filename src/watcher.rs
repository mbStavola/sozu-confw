use tokio_core::reactor::Core;
use sozu_command::state::ConfigState;
use notify::{RecommendedWatcher, Watcher, RecursiveMode, DebouncedEvent};

use std::time::Duration;
use std::sync::mpsc::channel;

use util::errors;
use parser::parse_config_file;
use rpc::{get_config_state, execute_orders};

pub fn watch(config_file: &str, socket_path: &str, update_interval: Duration) -> errors::Result<()> {
    let (tx, rx) = channel();

    info!("Watching file `{}`. Updating every {} second(s).", config_file, update_interval.as_secs());
    let mut watcher: RecommendedWatcher = Watcher::new(tx, update_interval)?;
    watcher.watch(config_file, RecursiveMode::NonRecursive)?;

    let mut core = Core::new().unwrap();
    let handle = core.handle();

    info!("Retrieving current proxy state.");
    let config_state_future = get_config_state(socket_path, &handle);
    let mut current_state: ConfigState = core.run(config_state_future)?;
    info!("Current state initialized. Waiting for changes...");

    loop {
        match rx.recv() {
            Ok(event) => {
                match event {
                    DebouncedEvent::Write(path) | DebouncedEvent::Create(path) | DebouncedEvent::Chmod(path) => {
                        info!("File written, generating diff.");

                        match parse_config_file(&path) {
                            Ok(new_state) => {
                                let orders = current_state.diff(&new_state);

                                if orders.is_empty() {
                                    info!("Sending new configuration to server.");

                                    let execution_future = execute_orders(socket_path, &handle, &orders);
                                    core.run(execution_future)?;

                                    current_state = new_state;
                                }
                            }
                            Err(_) => {
                                error!("Error reading file.");
                                continue;
                            }
                        }
                    }
                    DebouncedEvent::Rename(old_path, new_path) => {
                        // Track changed filename
                        info!("File renamed:\n\tOld path: {}\n\tNew path: {}.",
                              old_path.to_str().expect("missing old path"),
                              new_path.to_str().expect("missing new path")
                        );

                        watcher.unwatch(old_path)?;
                        watcher.watch(new_path, RecursiveMode::NonRecursive)?;
                    }
                    event => {
                        debug!("Unhandled event: {:?}.", event);
                    }
                }
            }
            Err(e) => {
                error!("Cannot poll file.");
                return Err(e.into());
            }
        }
    }
}