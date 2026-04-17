//! 消息总线核心模块
//!
//! 提供统一的多通道消息总线（Bus），是运行时各组件间通信的中枢。
//! 总线包含四条独立通道：
//! - inbound（入站）：外部消息进入系统，使用 Dropping 策略防止背压传播
//! - outbound（出站）：系统向外发送消息，使用 Sliding 策略保证最新消息优先
//! - internal（内部）：组件间通信，使用 Sliding 策略
//! - control（控制）：生命周期管理信号，使用 Sliding 策略

use crate::bus::bounded::{
    bounded_channel_with_counter, BoundedReceiver, BoundedSender, BufferStrategy,
};
use crate::bus::message::{ControlMessage, InboundMessage, InternalMessage, OutboundMessage};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// 总线各通道的缓冲区容量配置
pub struct BusConfig {
    pub inbound_capacity: usize,
    pub outbound_capacity: usize,
    pub internal_capacity: usize,
    pub control_capacity: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusChannelKind {
    Inbound,
    Outbound,
    Internal,
    Control,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BusTelemetrySnapshot {
    pub inbound_dropped_total: u64,
    pub outbound_dropped_total: u64,
    pub internal_dropped_total: u64,
    pub control_dropped_total: u64,
    pub inbound_queue_depth: usize,
    pub outbound_queue_depth: usize,
    pub internal_queue_depth: usize,
    pub control_queue_depth: usize,
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

/// 消息总线，运行时的核心通信基础设施
///
/// 通过四条有界通道实现组件间解耦通信，支持关闭状态检测。
/// 入站通道使用 Dropping 策略（满时丢弃新消息），其余通道使用 Sliding 策略（满时丢弃旧消息）。
pub struct Bus {
    inbound_tx: BoundedSender<InboundMessage>,
    inbound_rx: BoundedReceiver<InboundMessage>,
    outbound_tx: BoundedSender<OutboundMessage>,
    outbound_rx: BoundedReceiver<OutboundMessage>,
    internal_tx: BoundedSender<InternalMessage>,
    internal_rx: BoundedReceiver<InternalMessage>,
    control_tx: BoundedSender<ControlMessage>,
    control_rx: BoundedReceiver<ControlMessage>,
    inbound_dropped_total: Arc<AtomicU64>,
    outbound_dropped_total: Arc<AtomicU64>,
    internal_dropped_total: Arc<AtomicU64>,
    control_dropped_total: Arc<AtomicU64>,
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
    /// 使用默认配置创建总线
    pub fn create() -> Self {
        Self::create_with_config(BusConfig::default())
    }

    /// 使用自定义缓冲区容量创建总线
    ///
    /// 入站通道采用 Dropping 策略：满时丢弃新消息，避免外部流量压垮系统
    /// 其余通道采用 Sliding 策略：满时丢弃最旧消息，保证最新数据优先送达
    pub fn create_with_config(config: BusConfig) -> Self {
        let inbound_dropped_total = Arc::new(AtomicU64::new(0));
        let outbound_dropped_total = Arc::new(AtomicU64::new(0));
        let internal_dropped_total = Arc::new(AtomicU64::new(0));
        let control_dropped_total = Arc::new(AtomicU64::new(0));
        let (inbound_tx, inbound_rx) = bounded_channel_with_counter(
            BufferStrategy::Dropping(config.inbound_capacity),
            Some(inbound_dropped_total.clone()),
        );
        let (outbound_tx, outbound_rx) = bounded_channel_with_counter(
            BufferStrategy::Sliding(config.outbound_capacity),
            Some(outbound_dropped_total.clone()),
        );
        let (internal_tx, internal_rx) = bounded_channel_with_counter(
            BufferStrategy::Sliding(config.internal_capacity),
            Some(internal_dropped_total.clone()),
        );
        let (control_tx, control_rx) = bounded_channel_with_counter(
            BufferStrategy::Sliding(config.control_capacity),
            Some(control_dropped_total.clone()),
        );
        Self {
            inbound_tx,
            inbound_rx,
            outbound_tx,
            outbound_rx,
            internal_tx,
            internal_rx,
            control_tx,
            control_rx,
            inbound_dropped_total,
            outbound_dropped_total,
            internal_dropped_total,
            control_dropped_total,
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

    pub fn telemetry_snapshot(&self) -> BusTelemetrySnapshot {
        BusTelemetrySnapshot {
            inbound_dropped_total: self.inbound_dropped_total.load(Ordering::SeqCst),
            outbound_dropped_total: self.outbound_dropped_total.load(Ordering::SeqCst),
            internal_dropped_total: self.internal_dropped_total.load(Ordering::SeqCst),
            control_dropped_total: self.control_dropped_total.load(Ordering::SeqCst),
            inbound_queue_depth: self.inbound_tx.len(),
            outbound_queue_depth: self.outbound_tx.len(),
            internal_queue_depth: self.internal_tx.len(),
            control_queue_depth: self.control_tx.len(),
        }
    }

    pub fn dropped_total(&self, channel: BusChannelKind) -> u64 {
        match channel {
            BusChannelKind::Inbound => self.inbound_dropped_total.load(Ordering::SeqCst),
            BusChannelKind::Outbound => self.outbound_dropped_total.load(Ordering::SeqCst),
            BusChannelKind::Internal => self.internal_dropped_total.load(Ordering::SeqCst),
            BusChannelKind::Control => self.control_dropped_total.load(Ordering::SeqCst),
        }
    }
}
