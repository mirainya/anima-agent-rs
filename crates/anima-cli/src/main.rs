use anima_runtime::bootstrap::RuntimeBootstrapBuilder;
use anima_runtime::cli::{DEFAULT_PROMPT, DEFAULT_URL};
use anima_runtime::metrics::MetricsCollector;
use anima_sdk::runtime::{
    RuntimeBootstrapOptionsBuilder, RuntimeFacadeDispatcherStatus, RuntimeFacadeRouteReport,
    RuntimeFacadeStatus,
};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Clone, PartialEq)]
struct CliOptions {
    url: String,
    prompt: String,
    help: bool,
    status: bool,
    dispatcher_status: bool,
    route_reports: bool,
    channels: bool,
    health: bool,
    metrics: bool,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> CliOptions {
    let mut iter = args.into_iter();
    let mut opts = CliOptions {
        url: DEFAULT_URL.to_string(),
        prompt: DEFAULT_PROMPT.to_string(),
        help: false,
        status: false,
        dispatcher_status: false,
        route_reports: false,
        channels: false,
        health: false,
        metrics: false,
    };

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--url" => {
                if let Some(value) = iter.next() {
                    opts.url = value;
                }
            }
            "--prompt" => {
                if let Some(value) = iter.next() {
                    opts.prompt = value;
                }
            }
            "--help" | "-h" => opts.help = true,
            "--status" => opts.status = true,
            "--dispatcher-status" => opts.dispatcher_status = true,
            "--route-reports" => opts.route_reports = true,
            "--channels" => opts.channels = true,
            "--health" => opts.health = true,
            "--metrics" => opts.metrics = true,
            _ => {}
        }
    }

    opts
}

fn print_help() {
    println!("Anima Agent CLI - Interactive AI Assistant");
    println!();
    println!("Usage:");
    println!("  anima-agent-rs");
    println!("  anima-agent-rs --url <url>");
    println!("  anima-agent-rs --prompt <prompt>");
    println!("  anima-agent-rs --status");
    println!("  anima-agent-rs --dispatcher-status");
    println!("  anima-agent-rs --route-reports");
    println!("  anima-agent-rs --channels");
    println!("  anima-agent-rs --health");
    println!("  anima-agent-rs --metrics");
    println!("  anima-agent-rs --help");
    println!();
    println!("Options:");
    println!("  --url <url>              OpenCode server URL (default: http://127.0.0.1:9711)");
    println!("  --prompt <prompt>        CLI prompt string (default: 'anima> ')");
    println!("  --status                 Show runtime façade status snapshot");
    println!("  --dispatcher-status      Show dispatcher status snapshot");
    println!("  --route-reports          Show dispatcher route reports");
    println!("  --channels               Show registered channel entries");
    println!("  --health                 Show channel health report");
    println!("  --metrics                Show bootstrap metrics snapshot");
    println!("  --help, -h               Show this help");
}

fn make_runtime_status() -> RuntimeFacadeStatus {
    let bootstrap_opts = RuntimeBootstrapOptionsBuilder::new().build();
    let mut runtime = RuntimeBootstrapBuilder::new()
        .with_url(bootstrap_opts.url)
        .with_prompt(bootstrap_opts.prompt)
        .with_cli_enabled(bootstrap_opts.cli_enabled)
        .build();
    runtime.start();
    let dispatcher_status = runtime.dispatcher.status();
    let route_reports = runtime
        .dispatcher
        .route_reports()
        .into_iter()
        .map(|report| RuntimeFacadeRouteReport {
            channel: report.channel,
            pending_queue_depth: report.pending_queue_depth,
            target_count: report.target_count,
            healthy_target_count: report.healthy_target_count,
            open_circuit_count: report.open_circuit_count,
        })
        .collect::<Vec<_>>();
    runtime.stop();

    RuntimeFacadeStatus {
        dispatcher: RuntimeFacadeDispatcherStatus {
            state: format!("{:?}", dispatcher_status.state),
            queue_depth: dispatcher_status.queue_depth,
            route_count: dispatcher_status.routes.len(),
        },
        route_reports,
    }
}

fn render_status() -> String {
    let status = make_runtime_status();
    format!(
        "Runtime Status\n  dispatcher_state: {}\n  queue_depth: {}\n  route_reports: {}",
        status.dispatcher.state,
        status.dispatcher.queue_depth,
        status.route_reports.len()
    )
}

fn render_dispatcher_status() -> String {
    let status = make_runtime_status();
    format!(
        "Dispatcher Status\n  state: {}\n  queue_depth: {}\n  routes: {}",
        status.dispatcher.state, status.dispatcher.queue_depth, status.dispatcher.route_count
    )
}

fn render_route_reports() -> String {
    let reports = make_runtime_status().route_reports;
    if reports.is_empty() {
        return "Route Reports\n  (none)".to_string();
    }

    let mut lines = vec!["Route Reports".to_string()];
    for report in reports {
        lines.push(format!(
            "  - {} | pending={} | targets={} | healthy={} | open_circuits={}",
            report.channel,
            report.pending_queue_depth,
            report.target_count,
            report.healthy_target_count,
            report.open_circuit_count
        ));
    }
    lines.join("\n")
}

fn render_channels() -> String {
    let runtime = RuntimeBootstrapBuilder::new().build();
    let entries = runtime.registry.entries();
    if entries.is_empty() {
        return "Channel Entries\n  (none)".to_string();
    }
    let mut lines = vec!["Channel Entries".to_string()];
    for entry in entries {
        lines.push(format!("  - {} [{}]", entry.channel, entry.account_id));
    }
    lines.join("\n")
}

fn render_health() -> String {
    let runtime = RuntimeBootstrapBuilder::new().build();
    let report = runtime.registry.health_report();
    format!(
        "Health Report\n  total: {}\n  healthy: {}\n  unhealthy: {}\n  all_healthy: {}",
        report.total, report.healthy, report.unhealthy, report.all_healthy
    )
}

fn render_metrics() -> String {
    let metrics = MetricsCollector::new(Some("anima"));
    metrics.register_agent_metrics();
    let snapshot = metrics.snapshot();
    format!(
        "Metrics Snapshot\n  prefix: {}\n  counters: {}\n  gauges: {}\n  histograms: {}",
        metrics.prefix(),
        snapshot.counters.len(),
        snapshot.gauges.len(),
        snapshot.histograms.len()
    )
}

fn start_cli(opts: &CliOptions) -> std::io::Result<()> {
    println!("╔══════════════════════════════════════════╗");
    println!("║         Anima Agent CLI (Rust)          ║");
    println!("╚══════════════════════════════════════════╝");
    println!();
    println!("  Server: {}", opts.url);
    println!("  Commands: help, status, history, clear, exit");
    println!();

    let bootstrap_opts = RuntimeBootstrapOptionsBuilder::new()
        .with_url(opts.url.clone())
        .with_prompt(opts.prompt.clone())
        .with_cli_enabled(true)
        .build();
    let mut runtime = RuntimeBootstrapBuilder::new()
        .with_url(bootstrap_opts.url)
        .with_prompt(bootstrap_opts.prompt)
        .with_cli_enabled(bootstrap_opts.cli_enabled)
        .with_builtin_tools_enabled(false)
        .with_sdk_directory_enabled(false)
        .build();
    runtime.start();
    let result = runtime
        .cli_channel()
        .expect("cli channel should exist when enabled")
        .run_stdin_loop();
    runtime.stop();
    println!("\nGoodbye!");
    result
}

fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let opts = parse_args(std::env::args().skip(1));
    if opts.help {
        print_help();
        return Ok(());
    }
    if opts.status {
        println!("{}", render_status());
        return Ok(());
    }
    if opts.dispatcher_status {
        println!("{}", render_dispatcher_status());
        return Ok(());
    }
    if opts.route_reports {
        println!("{}", render_route_reports());
        return Ok(());
    }
    if opts.channels {
        println!("{}", render_channels());
        return Ok(());
    }
    if opts.health {
        println!("{}", render_health());
        return Ok(());
    }
    if opts.metrics {
        println!("{}", render_metrics());
        return Ok(());
    }
    start_cli(&opts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_defaults_match_baseline() {
        let opts = parse_args(Vec::<String>::new());
        assert_eq!(opts.url, DEFAULT_URL);
        assert_eq!(opts.prompt, DEFAULT_PROMPT);
        assert!(!opts.help);
        assert!(!opts.status);
        assert!(!opts.dispatcher_status);
        assert!(!opts.route_reports);
        assert!(!opts.channels);
        assert!(!opts.health);
        assert!(!opts.metrics);
    }

    #[test]
    fn parse_args_accepts_url_prompt_and_help() {
        let opts = parse_args(vec![
            "--url".to_string(),
            "http://localhost:9999".to_string(),
            "--prompt".to_string(),
            "custom> ".to_string(),
        ]);
        assert_eq!(opts.url, "http://localhost:9999");
        assert_eq!(opts.prompt, "custom> ");

        let opts = parse_args(vec!["--help".to_string()]);
        assert!(opts.help);

        let opts = parse_args(vec!["-h".to_string()]);
        assert!(opts.help);
    }

    #[test]
    fn parse_args_accepts_status_flags() {
        let status = parse_args(vec!["--status".to_string()]);
        assert!(status.status);

        let dispatcher = parse_args(vec!["--dispatcher-status".to_string()]);
        assert!(dispatcher.dispatcher_status);

        let routes = parse_args(vec!["--route-reports".to_string()]);
        assert!(routes.route_reports);

        let channels = parse_args(vec!["--channels".to_string()]);
        assert!(channels.channels);

        let health = parse_args(vec!["--health".to_string()]);
        assert!(health.health);

        let metrics = parse_args(vec!["--metrics".to_string()]);
        assert!(metrics.metrics);
    }

    #[test]
    fn render_status_outputs_stable_sections() {
        let output = render_status();
        assert!(output.contains("Runtime Status"));
        assert!(output.contains("dispatcher_state"));
        assert!(output.contains("queue_depth"));
    }

    #[test]
    fn render_dispatcher_and_route_reports_are_stable() {
        let dispatcher = render_dispatcher_status();
        assert!(dispatcher.contains("Dispatcher Status"));
        assert!(dispatcher.contains("state:"));

        let routes = render_route_reports();
        assert!(routes.contains("Route Reports"));
    }

    #[test]
    fn render_channels_health_and_metrics_are_stable() {
        let channels = render_channels();
        assert!(channels.contains("Channel Entries"));

        let health = render_health();
        assert!(health.contains("Health Report"));

        let metrics = render_metrics();
        assert!(metrics.contains("Metrics Snapshot"));
        assert!(metrics.contains("prefix:"));
    }
}
