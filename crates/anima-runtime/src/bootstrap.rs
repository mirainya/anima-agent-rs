use crate::agent::{Agent, SdkTaskExecutor, TaskExecutor};
use crate::channel::{Channel, ChannelRegistry, SessionStore};
use crate::cli::CliChannel;
use crate::dispatcher::{start_dispatcher_outbound_loop, Dispatcher};
use crate::hooks::{HookRegistry, StopHook};
use crate::permissions::{PermissionChecker, PermissionDecision, PermissionMode, PermissionRule};
use crate::provider::{AnthropicProvider, Provider};
use crate::runtime::{JsonStateStore, RuntimeStateStore, SharedRuntimeStateStore, SqliteStateStore};
use anima_sdk::facade::{Client as SdkClient, ClientOptions as SdkClientOptions};
use anima_types::config::AnimaConfig;
use std::sync::Arc;
use std::thread::JoinHandle;

pub struct RuntimeBootstrapBuilder {
    url: String,
    prompt: Option<String>,
    cli_enabled: bool,
    builtin_tools_enabled: bool,
    sdk_directory_enabled: bool,
    executor: Option<Arc<dyn TaskExecutor>>,
    sdk_options: SdkClientOptions,
    runtime_state_store: Option<SharedRuntimeStateStore>,
    provider_config: Option<anima_types::config::ProviderConfig>,
    permission_config: Option<anima_types::config::PermissionConfig>,
}

impl Default for RuntimeBootstrapBuilder {
    fn default() -> Self {
        Self {
            url: crate::cli::DEFAULT_URL.to_string(),
            prompt: Some(crate::cli::DEFAULT_PROMPT.to_string()),
            cli_enabled: true,
            builtin_tools_enabled: true,
            sdk_directory_enabled: true,
            executor: None,
            sdk_options: SdkClientOptions::default(),
            runtime_state_store: None,
            provider_config: None,
            permission_config: None,
        }
    }
}

impl RuntimeBootstrapBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_config(config: &AnimaConfig) -> Self {
        let state_path = &config.runtime.state_path;
        let runtime_state_store: SharedRuntimeStateStore = match config.runtime.store.as_str() {
            "sqlite" => {
                let db_path = std::path::PathBuf::from(state_path).with_extension("db");
                Arc::new(SqliteStateStore::open(&db_path).unwrap_or_else(|_| {
                    SqliteStateStore::open_in_memory().expect("in-memory sqlite should not fail")
                }))
            }
            _ => Arc::new(JsonStateStore::with_persistence(
                std::path::PathBuf::from(state_path),
            )),
        };
        Self {
            url: config.server.url.clone(),
            prompt: Some(config.cli.prompt.clone()),
            cli_enabled: config.cli.enabled,
            builtin_tools_enabled: config.runtime.builtin_tools,
            sdk_directory_enabled: true,
            executor: None,
            sdk_options: SdkClientOptions::from(&config.sdk),
            runtime_state_store: Some(runtime_state_store),
            provider_config: Some(config.provider.clone()),
            permission_config: Some(config.permissions.clone()),
        }
    }

    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = url.into();
        self
    }

    pub fn with_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = Some(prompt.into());
        self
    }

    pub fn with_cli_enabled(mut self, enabled: bool) -> Self {
        self.cli_enabled = enabled;
        self
    }

    pub fn with_builtin_tools_enabled(mut self, enabled: bool) -> Self {
        self.builtin_tools_enabled = enabled;
        self
    }

    pub fn with_sdk_directory_enabled(mut self, enabled: bool) -> Self {
        self.sdk_directory_enabled = enabled;
        self
    }

    pub fn with_executor(mut self, executor: Arc<dyn TaskExecutor>) -> Self {
        self.executor = Some(executor);
        self
    }

    pub fn with_sdk_options(mut self, options: SdkClientOptions) -> Self {
        self.sdk_options = options;
        self
    }

    pub fn with_runtime_state_store(mut self, store: SharedRuntimeStateStore) -> Self {
        self.runtime_state_store = Some(store);
        self
    }

    pub fn with_in_memory_runtime_state(mut self) -> Self {
        self.runtime_state_store = Some(Arc::new(JsonStateStore::new()));
        self
    }

    pub fn build(self) -> RuntimeBootstrap {
        let bus = Arc::new(crate::bus::Bus::create());
        let registry = Arc::new(ChannelRegistry::new());
        let session_store = Arc::new(SessionStore::new());
        let cli_channel = if self.cli_enabled {
            let cli = Arc::new(CliChannel::new(
                Some(session_store.clone()),
                Some(bus.clone()),
                self.prompt.as_deref(),
            ));
            registry.register(cli.clone() as Arc<dyn Channel>, None);
            Some(cli)
        } else {
            None
        };
        let dispatcher = Arc::new(Dispatcher::new(registry.clone(), None));
        let client = if self.sdk_directory_enabled {
            match std::env::current_dir() {
                Ok(dir) => SdkClient::with_options(self.url, self.sdk_options)
                    .with_directory(dir.to_string_lossy().to_string()),
                Err(_) => SdkClient::with_options(self.url, self.sdk_options),
            }
        } else {
            SdkClient::with_options(self.url, self.sdk_options)
        };
        let has_custom_executor = self.executor.is_some();
        let executor: Arc<dyn TaskExecutor> = self
            .executor
            .unwrap_or_else(|| Arc::new(SdkTaskExecutor::new(client)));
        let runtime_state_store = self.runtime_state_store.unwrap_or_else(|| {
            Arc::new(RuntimeStateStore::with_persistence(
                std::path::PathBuf::from(".opencode/runtime/state.json"),
            ))
        });
        let mut agent = Agent::with_runtime_state_store(
            bus.clone(),
            Some(session_store),
            executor,
            runtime_state_store,
        );
        if self.builtin_tools_enabled {
            agent.register_builtin_tools().expect("register_builtin_tools failed: Arc has other references before agent start");
        }
        let mut hook_registry = HookRegistry::new();
        hook_registry.register_post_hook(Arc::new(StopHook));
        agent.set_hook_registry(hook_registry).expect("set_hook_registry failed: Arc has other references before agent start");
        if let Some(ref pc) = self.permission_config {
            let mode = match pc.mode.as_str() {
                "allow_all" => PermissionMode::AllowAll,
                "deny_all" => PermissionMode::DenyAll,
                _ => PermissionMode::RuleBased,
            };
            let mut checker = PermissionChecker::new(mode);
            for rule in &pc.rules {
                let decision = match rule.decision.as_str() {
                    "allow" => PermissionDecision::Allow,
                    "deny" => PermissionDecision::Deny(format!("denied by config: {}", rule.tool_pattern)),
                    _ => continue,
                };
                checker.add_rule(PermissionRule {
                    tool_pattern: rule.tool_pattern.clone(),
                    decision,
                    priority: rule.priority,
                });
            }
            let _ = agent.set_permission_checker(checker);
        }
        if !has_custom_executor {
            if let Some(ref pc) = self.provider_config {
                if pc.kind == "anthropic" {
                    if let Ok(p) = AnthropicProvider::from_config(pc) {
                        let _ = agent.set_provider(Arc::new(p) as Arc<dyn Provider>);
                    }
                }
            }
            agent.enable_llm_judge();
        }

        RuntimeBootstrap {
            bus,
            registry,
            dispatcher,
            agent,
            cli_channel,
            dispatcher_handle: None,
        }
    }
}

pub struct RuntimeBootstrap {
    pub bus: Arc<crate::bus::Bus>,
    pub registry: Arc<ChannelRegistry>,
    pub dispatcher: Arc<Dispatcher>,
    pub agent: Agent,
    cli_channel: Option<Arc<CliChannel>>,
    dispatcher_handle: Option<JoinHandle<()>>,
}

impl RuntimeBootstrap {
    pub fn start(&mut self) {
        if let Some(cli) = &self.cli_channel {
            cli.start();
        }
        self.agent.start();
        if self.dispatcher_handle.is_none() {
            self.dispatcher_handle = Some(start_dispatcher_outbound_loop(
                self.bus.clone(),
                self.dispatcher.clone(),
            ));
        }
    }

    pub fn stop(&mut self) {
        if let Some(cli) = &self.cli_channel {
            cli.stop();
        }
        self.agent.stop();
        self.registry.stop_all();
        self.bus.close();
        if let Some(handle) = self.dispatcher_handle.take() {
            let _ = handle.join();
        }
    }

    pub fn cli_channel(&self) -> Option<Arc<CliChannel>> {
        self.cli_channel.clone()
    }
}
