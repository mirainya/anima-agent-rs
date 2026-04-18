use crate::agent::{Agent, TaskExecutor};
use crate::channel::{Channel, ChannelRegistry, SessionStore};
use crate::cli::CliChannel;
use crate::dispatcher::{start_dispatcher_outbound_loop, Dispatcher};
use crate::hooks::{HookRegistry, StopHook};
use crate::runtime::RuntimeStateStore;
use anima_sdk::facade::{Client as SdkClient, ClientOptions as SdkClientOptions};
use std::sync::Arc;
use std::thread::JoinHandle;

pub struct RuntimeBootstrapBuilder {
    url: String,
    prompt: Option<String>,
    cli_enabled: bool,
    executor: Option<Arc<dyn TaskExecutor>>,
    sdk_options: SdkClientOptions,
    runtime_state_store: Option<Arc<RuntimeStateStore>>,
}

impl Default for RuntimeBootstrapBuilder {
    fn default() -> Self {
        Self {
            url: crate::cli::DEFAULT_URL.to_string(),
            prompt: Some(crate::cli::DEFAULT_PROMPT.to_string()),
            cli_enabled: true,
            executor: None,
            sdk_options: SdkClientOptions::default(),
            runtime_state_store: None,
        }
    }
}

impl RuntimeBootstrapBuilder {
    pub fn new() -> Self {
        Self::default()
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

    pub fn with_executor(mut self, executor: Arc<dyn TaskExecutor>) -> Self {
        self.executor = Some(executor);
        self
    }

    pub fn with_sdk_options(mut self, options: SdkClientOptions) -> Self {
        self.sdk_options = options;
        self
    }

    pub fn with_runtime_state_store(mut self, store: Arc<RuntimeStateStore>) -> Self {
        self.runtime_state_store = Some(store);
        self
    }

    pub fn with_in_memory_runtime_state(mut self) -> Self {
        self.runtime_state_store = Some(Arc::new(RuntimeStateStore::new()));
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
        let client = match std::env::current_dir() {
            Ok(dir) => SdkClient::with_options(self.url, self.sdk_options)
                .with_directory(dir.to_string_lossy().to_string()),
            Err(_) => SdkClient::with_options(self.url, self.sdk_options),
        };
        let mut agent = if let Some(runtime_state_store) = self.runtime_state_store {
            Agent::with_runtime_state_store(
                bus.clone(),
                Some(client),
                Some(session_store),
                self.executor,
                runtime_state_store,
            )
        } else {
            Agent::create(
                bus.clone(),
                Some(client),
                Some(session_store),
                self.executor,
            )
        };
        agent.register_builtin_tools();
        let mut hook_registry = HookRegistry::new();
        hook_registry.register_post_hook(Arc::new(StopHook));
        agent.set_hook_registry(hook_registry);

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
