use crate::dispatcher::core::Dispatcher;
use crate::dispatcher::diagnostics::{DispatcherState, DispatcherStatus};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DispatcherControlCommand {
    Start,
    Pause,
    Resume,
    Drain,
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatcherControlResponse {
    pub state: DispatcherState,
    pub queue_depth: usize,
}

pub fn apply_dispatcher_control(
    dispatcher: &Dispatcher,
    command: DispatcherControlCommand,
) -> DispatcherControlResponse {
    match command {
        DispatcherControlCommand::Start => dispatcher.start(),
        DispatcherControlCommand::Pause => dispatcher.pause(),
        DispatcherControlCommand::Resume => dispatcher.resume(),
        DispatcherControlCommand::Drain => dispatcher.drain(),
        DispatcherControlCommand::Stop => dispatcher.stop(),
    }

    let status = dispatcher.status();
    DispatcherControlResponse {
        state: status.state,
        queue_depth: status.queue_depth,
    }
}

pub fn dispatcher_status(dispatcher: &Dispatcher) -> DispatcherStatus {
    dispatcher.status()
}
