//! Core supervisor logic — app lifecycle management.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;
use edge_runtime::linker::create_component_linker;
use edge_runtime::RequestMeter;
use tokio::sync::{Mutex, RwLock};
use tokio::time::{sleep, Duration};
use wasmtime::component::InstancePre;

use crate::config::Config;
use crate::downloader::Downloader;
use crate::messages::{AppSpec, AppStatus, HeartbeatMessage, TaskMessage};
use crate::nats::NatsClient;
use crate::port_pool::PortPool;
use crate::state::{AppInstance, AppInstanceStatus, WorkerState};

/// The main supervisor — manages all running apps for this worker node.
pub struct Supervisor {
    pub config: Config,
    pub state: Arc<RwLock<WorkerState>>,
    pub downloader: Arc<Downloader>,
    pub port_pool: Arc<Mutex<PortPool>>,
    pub nats: Arc<NatsClient>,
}

impl Supervisor {
    /// Handle an incoming TaskMessage from NATS.
    ///
    /// Diffs the desired app set against currently running apps and
    /// starts/stops apps accordingly.
    pub async fn handle_task_message(&self, msg: TaskMessage) -> anyhow::Result<()> {
        let TaskMessage::TaskUpdate {
            tenant_id,
            apps: desired_apps,
            ..
        } = msg;

        let state = self.state.read().await;
        let current_apps: HashMap<String, (String, AppInstanceStatus)> = state
            .apps
            .iter()
            .map(|(name, inst)| {
                (
                    name.clone(),
                    (inst.deployment_id.clone(), inst.status.clone()),
                )
            })
            .collect();
        drop(state);

        // Stop apps no longer in the desired set
        for app_name in current_apps.keys() {
            if !desired_apps.contains_key(app_name) {
                if let Err(e) = self.stop_app(app_name).await {
                    tracing::error!(app_name, err = %e, "failed to stop app");
                }
            }
        }

        // Start or update apps in the desired set
        for (app_name, spec) in &desired_apps {
            let is_new = !current_apps.contains_key(app_name);
            let is_changed = current_apps
                .get(app_name)
                .map(|(dep_id, _)| dep_id != &spec.deployment_id)
                .unwrap_or(false);

            if is_new || is_changed {
                if let Err(e) = self.start_app(app_name, spec, &tenant_id).await {
                    tracing::error!(app_name, err = %e, "failed to start app");
                }
            }
        }

        Ok(())
    }

    /// Start a new app or restart a changed one.
    async fn start_app(
        &self,
        app_name: &str,
        spec: &AppSpec,
        tenant_id: &str,
    ) -> anyhow::Result<()> {
        tracing::info!(app_name, deployment_id = spec.deployment_id, "starting app");

        // Stop existing instance if present
        if self.state.read().await.apps.contains_key(app_name) {
            self.stop_app(app_name).await?;
        }

        // Acquire a port
        let port = {
            let mut pool = self.port_pool.lock().await;
            pool.acquire().expect("port pool exhausted")
        };

        // Download artifact (blocking on first request)
        let artifact = self
            .downloader
            .get_artifact(&spec.deployment_id, &spec.deployment_hash)
            .await?;

        // Compile the component using the shared engine
        let engine = &self.state.read().await.engine;
        let component = wasmtime::component::Component::from_binary(engine, &artifact)
            .with_context(|| format!("failed to compile component for {}", app_name))?;

        // Create the component linker and pre-instantiate
        let linker = create_component_linker(engine)?;
        let instance_pre = linker
            .instantiate_pre(&component)
            .with_context(|| format!("failed to pre-instantiate {}", app_name))?;

        // Set env vars in the process (global to the worker — see Known Issue #1)
        for (key, value) in &spec.env {
            std::env::set_var(key, value);
        }

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

        // Create request meter
        let meter = Arc::new(RequestMeter::new(
            tenant_id.to_string(),
            spec.deployment_id.clone(),
        ));

        let instance_pre_clone = instance_pre.clone();
        let app_name_str = app_name.to_string();
        let meter_clone = meter.clone();

        // Spawn the per-app task
        tokio::spawn(async move {
            Self::run_app_loop(instance_pre_clone, meter_clone, shutdown_rx).await;
            tracing::info!(app_name = %app_name_str, "app task exited");
        });

        // Register the app instance
        let instance = AppInstance {
            deployment_id: spec.deployment_id.clone(),
            app_name: app_name.to_string(),
            tenant_id: tenant_id.to_string(),
            port,
            status: AppInstanceStatus::Running,
            meter,
            shutdown_tx,
            instance_pre,
        };

        self.state
            .write()
            .await
            .apps
            .insert(app_name.to_string(), instance);

        tracing::info!(app_name, port, "app started");
        Ok(())
    }

    /// Stop an app gracefully.
    pub async fn stop_app(&self, app_name: &str) -> anyhow::Result<()> {
        let instance = match self.state.write().await.apps.remove(app_name) {
            Some(inst) => inst,
            None => return Ok(()), // already gone
        };

        // Signal shutdown
        let _ = instance.shutdown_tx.send(());

        // Free the port
        {
            let mut pool = self.port_pool.lock().await;
            pool.release(instance.port);
        }

        tracing::info!(app_name, "app stopped");
        Ok(())
    }

    /// Per-app task loop.
    ///
    /// Executes the component in a loop. Handles crashes with exponential
    /// backoff restart (max 5 restarts, then gives up).
    async fn run_app_loop(
        instance_pre: InstancePre<edge_runtime::RuntimeState>,
        meter: Arc<RequestMeter>,
        mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    ) {
        let mut restart_count = 0u32;
        let max_restarts = 5;
        let base_backoff = Duration::from_secs(1);
        let max_backoff = Duration::from_secs(60);

        loop {
            tokio::select! {
                // Graceful shutdown signal from supervisor
                _ = &mut shutdown_rx => {
                    tracing::info!("app received shutdown signal");
                    break;
                }

                // Run the component
                result = Self::execute_app(&instance_pre, &meter) => {
                    match result {
                        Ok(()) => {
                            tracing::info!("component exited normally");
                            break;
                        }
                        Err(e) => {
                            restart_count += 1;
                            if restart_count >= max_restarts {
                                tracing::error!(
                                    restart_count,
                                    err = %e,
                                    "max restarts exceeded, giving up"
                                );
                                break;
                            }

                            let backoff = std::cmp::min(
                                base_backoff * 2u32.pow(restart_count - 1),
                                max_backoff,
                            );
                            tracing::warn!(
                                err = %e,
                                restart_count,
                                "app crashed, restarting in {:?}",
                                backoff
                            );
                            sleep(backoff).await;
                        }
                    }
                }
            }
        }
    }

    /// Execute a single app invocation.
    async fn execute_app(
        instance_pre: &InstancePre<edge_runtime::RuntimeState>,
        _meter: &Arc<RequestMeter>,
    ) -> anyhow::Result<()> {
        let engine = instance_pre.engine();

        // Create a fresh RuntimeState for this invocation
        let runtime_state = edge_runtime::RuntimeState::new();

        // Create a store with per-invocation state
        let mut store = edge_runtime::create_store(engine, 256, runtime_state);

        // Memory limits are enforced via cgroups in production.
        // The wasmtime Store::limiter API (new_memory_limits) requires a ResourceLimiter
        // bound to the RuntimeState lifetime, which needs careful integration.
        // TODO: wire up Store::limiter with proper lifetime handling.

        // Instantiate
        let instance = instance_pre.instantiate(&mut store)?;

        // Try _start first (WASI Preview 2 canonical), then handle
        let has_start = instance
            .get_typed_func::<(), ()>(&mut store, "_start")
            .is_ok();

        if has_start {
            instance
                .get_typed_func::<(), ()>(&mut store, "_start")?
                .call(&mut store, ())?;
        } else {
            instance
                .get_typed_func::<(), ()>(&mut store, "handle")?
                .call(&mut store, ())?;
        }

        Ok(())
    }

    /// Build a heartbeat message from current app states.
    pub async fn build_heartbeat(&self) -> HeartbeatMessage {
        let mut msg =
            HeartbeatMessage::new(self.config.worker_id.clone(), self.config.region.clone());

        let state = self.state.read().await;
        for (app_name, inst) in &state.apps {
            let status = match &inst.status {
                AppInstanceStatus::Running => "running",
                AppInstanceStatus::Starting => "starting",
                AppInstanceStatus::Stopping => "stopping",
                AppInstanceStatus::Crashed { .. } => "crashed",
            };
            msg.apps.insert(
                app_name.clone(),
                AppStatus {
                    deployment_id: inst.deployment_id.clone(),
                    status: status.to_string(),
                    exit_code: None,
                },
            );
        }

        msg
    }

    /// Stop all running apps (used during graceful shutdown).
    pub async fn stop_all_apps(&self) {
        let app_names: Vec<String> = self.state.read().await.apps.keys().cloned().collect();
        for app_name in &app_names {
            if let Err(e) = self.stop_app(app_name).await {
                tracing::error!(app_name, err = %e, "failed to stop app during shutdown");
            }
        }
    }
}
