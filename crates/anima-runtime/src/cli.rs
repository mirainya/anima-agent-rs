use serde_json::json;
use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::bus::{make_inbound, Bus, MakeInbound};
use crate::channel::{
    ok, Channel, SendOptions, SendResult, Session, SessionCreateOptions, SessionStore,
};

pub const DEFAULT_PROMPT: &str = "anima> ";
pub const DEFAULT_URL: &str = "http://127.0.0.1:9711";

pub fn is_quit_command(line: &str) -> bool {
    matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "exit" | "quit" | ":q" | "/quit" | "/exit" | "bye"
    )
}

#[derive(Debug)]
pub struct CliChannel {
    running: AtomicBool,
    session_store: Arc<SessionStore>,
    default_session: Mutex<Option<Session>>,
    bus: Option<Arc<Bus>>,
    prompt: String,
    output: Mutex<Vec<String>>,
}

impl CliChannel {
    pub fn new(
        session_store: Option<Arc<SessionStore>>,
        bus: Option<Arc<Bus>>,
        prompt: Option<&str>,
    ) -> Self {
        Self {
            running: AtomicBool::new(false),
            session_store: session_store.unwrap_or_else(|| Arc::new(SessionStore::new())),
            default_session: Mutex::new(None),
            bus,
            prompt: prompt.unwrap_or(DEFAULT_PROMPT).to_string(),
            output: Mutex::new(Vec::new()),
        }
    }

    pub fn with_session(session: Session, bus: Option<Arc<Bus>>, prompt: Option<&str>) -> Self {
        Self {
            running: AtomicBool::new(false),
            session_store: Arc::new(SessionStore::new()),
            default_session: Mutex::new(Some(session)),
            bus,
            prompt: prompt.unwrap_or(DEFAULT_PROMPT).to_string(),
            output: Mutex::new(Vec::new()),
        }
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn bus(&self) -> Option<Arc<Bus>> {
        self.bus.clone()
    }

    pub fn default_session(&self) -> Option<Session> {
        self.default_session.lock().unwrap().clone()
    }

    pub fn outputs(&self) -> Vec<String> {
        self.output.lock().unwrap().clone()
    }

    pub fn status_text(&self) -> String {
        match self.default_session() {
            Some(session) => format!(
                "Session Status:\n  ID: {}\n  Channel: {}\n  Routing Key: {}\n  History Length: {}",
                session.id,
                session.channel,
                session.routing_key,
                self.session_store.get_history(&session.id).len()
            ),
            None => "No active session".to_string(),
        }
    }

    pub fn history_text(&self) -> String {
        match self.default_session() {
            Some(session) => {
                let history = self.session_store.get_history(&session.id);
                if history.is_empty() {
                    "No conversation history".to_string()
                } else {
                    let mut lines = vec!["Conversation History:".to_string()];
                    for message in history {
                        let role = message
                            .get("role")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        let content = message
                            .get("content")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        lines.push(format!("  [{role}]  {content}"));
                    }
                    lines.join("\n")
                }
            }
            None => "No active session".to_string(),
        }
    }

    pub fn clear_history(&self) {
        if let Some(session) = self.default_session() {
            let _ = self
                .session_store
                .set_session_context(&session.id, json!({"history": []}));
            self.output
                .lock()
                .unwrap()
                .push("Conversation history cleared.".to_string());
        }
    }

    pub fn help_text(&self) -> &'static str {
        "Available commands:\n  help              - Show this help\n  status            - Show session status\n  history           - Show conversation history\n  clear             - Clear conversation history\n  exit/quit/:q      - Exit the CLI\n\nJust type your message to chat with the AI."
    }

    pub fn handle_line(&self, line: &str) -> Option<String> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Some(self.prompt.clone());
        }
        let lowered = trimmed.to_ascii_lowercase();
        match lowered.as_str() {
            "help" => Some(self.help_text().to_string()),
            "status" => Some(self.status_text()),
            "history" => Some(self.history_text()),
            "clear" => {
                self.clear_history();
                Some("Conversation history cleared.".to_string())
            }
            _ if is_quit_command(trimmed) => {
                self.running.store(false, Ordering::SeqCst);
                if let Some(session) = self.default_session() {
                    let _ = self.session_store.close_session(&session.id);
                }
                Some("Goodbye!".to_string())
            }
            _ => {
                if let Some(session) = self.default_session() {
                    let _ = self
                        .session_store
                        .add_to_history(&session.id, json!({"role": "user", "content": trimmed}));
                    let _ = self.session_store.touch_session(&session.id);
                    if let Some(bus) = &self.bus {
                        let _ = bus.publish_inbound(make_inbound(MakeInbound {
                            channel: "cli".into(),
                            sender_id: Some("user".into()),
                            chat_id: Some(session.id.clone()),
                            content: trimmed.to_string(),
                            session_key: Some(session.routing_key.clone()),
                            metadata: Some(json!({"source": "stdin"})),
                            ..Default::default()
                        }));
                    }
                    None
                } else {
                    Some("No active session".to_string())
                }
            }
        }
    }

    pub fn run_stdin_loop(&self) -> io::Result<()> {
        let stdin = io::stdin();
        let mut stdout = io::stdout();
        writeln!(
            stdout,
            "\n{}Ready. Type 'help' for commands, 'exit' to quit.",
            self.prompt
        )?;
        write!(stdout, "{}", self.prompt)?;
        stdout.flush()?;

        for line in stdin.lock().lines() {
            let line = line?;
            if !self.is_running() {
                break;
            }
            if let Some(output) = self.handle_line(&line) {
                writeln!(stdout, "{}", output)?;
                if self.is_running() {
                    write!(stdout, "{}", self.prompt)?;
                    stdout.flush()?;
                }
            }
        }
        Ok(())
    }
}

impl Channel for CliChannel {
    fn start(&self) {
        if !self.running.swap(true, Ordering::SeqCst) {
            let mut default_session = self.default_session.lock().unwrap();
            if default_session.is_none() {
                *default_session = Some(
                    self.session_store
                        .create_session("cli", SessionCreateOptions::default()),
                );
            }
        }
    }

    fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    fn send_message(&self, _target: &str, message: &str, opts: SendOptions) -> SendResult {
        self.output.lock().unwrap().push(message.to_string());
        if opts.stage.as_deref() == Some("final") || opts.stage.is_none() {
            self.output.lock().unwrap().push(self.prompt.clone());
        }
        ok(None)
    }

    fn channel_name(&self) -> &str {
        "cli"
    }

    fn health_check(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}
