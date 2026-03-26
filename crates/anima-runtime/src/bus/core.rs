use crate::bus::bounded::{bounded_channel, BoundedReceiver, BoundedSender, BufferStrategy};
use crate::bus::message::{ControlMessage, InboundMessage, InternalMessage, OutboundMessage};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Configuration for bus buffer capacities.
pub struct BusConfig {
    pub inbound_capacity: usize,
    pub outbound_capacity: usize,
    pub internal_capacity: usize,
    pub control_capacity: usize,
}

impl Default for BusConfig {
    fn default() -> Self {
        Self {
            inbound_capacity: 1000,
            outbound_capacity: 1000,
            internal_capacity: 100,
            control_capacity: 10,
        }
    }
}

pub struct Bus {
    inbound_tx: BoundedSender<InboundMessage>,
    inbound_rx: BoundedReceiver<InboundMessage>,
    outbound_tx: BoundedSender<OutboundMessage>,
    outbound_rx: BoundedReceiver<OutboundMessage>,
    internal_tx: BoundedSender<InternalMessage>,
    internal_rx: BoundedReceiver<InternalMessage>,
    control_tx: BoundedSender<ControlMessage>,
    control_rx: BoundedReceiver<ControlMessage>,
    closed: Arc<AtomicBool>,
}

impl std::fmt::Debug for Bus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bus")
            .field("closed", &self.is_closed())
            .finish()
    }
}

impl Bus {
    /// Create a bus with default config (matches Clojure buffer semantics).
    pub fn create() -> Self {
        Self::create_with_config(BusConfig::default())
    }

    /// Create a bus with custom buffer capacities.
    pub fn create_with_config(config: BusConfig) -> Self {
        let (inbound_tx, inbound_rx) =
            bounded_channel(BufferStrategy::Dropping(config.inbound_capacity));
        let (outbound_tx, outbound_rx) =
            bounded_channel(BufferStrategy::Sliding(config.outbound_capacity));
        let (internal_tx, internal_rx) =
            bounded_channel(BufferStrategy::Sliding(config.internal_capacity));
        let (control_tx, control_rx) =
            bounded_channel(BufferStrategy::Sliding(config.control_capacity));
        Self {
            inbound_tx,
            inbound_rx,
            outbound_tx,
            outbound_rx,
            internal_tx,
            internal_rx,
            control_tx,
            control_rx,
            closed: Arc::new(AtomicBool::new(false)),
        }
    }

    // ── Inbound (Dropping) ─────────────────────────────────────────

    pub fn publish_inbound(&self, msg: InboundMessage) -> Result<(), String> {
        if self.is_closed() {
            return Err("bus closed".into());
        }
        self.inbound_tx.send(msg)
    }

    pub fn consume_inbound(&self) -> Option<InboundMessage> {
        self.inbound_rx.recv()
    }

    pub fn inbound_receiver(&self) -> BoundedReceiver<InboundMessage> {
        self.inbound_rx.clone()
    }

    // ── Outbound (Sliding) ─────────────────────────────────────────

    pub fn publish_outbound(&self, msg: OutboundMessage) -> Result<(), String> {
        if self.is_closed() {
            return Err("bus closed".into());
        }
        self.outbound_tx.send(msg)
    }

    pub fn consume_outbound(&self) -> Option<OutboundMessage> {
        self.outbound_rx.recv()
    }

    pub fn outbound_receiver(&self) -> BoundedReceiver<OutboundMessage> {
        self.outbound_rx.clone()
    }

    // ── Internal (Sliding) ─────────────────────────────────────────

    pub fn publish_internal(&self, msg: InternalMessage) -> Result<(), String> {
        if self.is_closed() {
            return Err("bus closed".into());
        }
        self.internal_tx.send(msg)
    }

    pub fn consume_internal(&self) -> Option<InternalMessage> {
        self.internal_rx.recv()
    }

    pub fn internal_receiver(&self) -> BoundedReceiver<InternalMessage> {
        self.internal_rx.clone()
    }

    // ── Control (Sliding) ──────────────────────────────────────────

    pub fn publish_control(&self, msg: ControlMessage) -> Result<(), String> {
        if self.is_closed() {
            return Err("bus closed".into());
        }
        self.control_tx.send(msg)
    }

    pub fn consume_control(&self) -> Option<ControlMessage> {
        self.control_rx.recv()
    }

    pub fn control_receiver(&self) -> BoundedReceiver<ControlMessage> {
        self.control_rx.clone()
    }

    // ── Lifecycle ──────────────────────────────────────────────────

    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}
