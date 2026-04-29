//! 有界通道模块
//!
//! 基于 crossbeam 的有界通道封装，提供两种缓冲区溢出策略：
//! - Dropping：通道满时丢弃新消息（适合入站流量，防止背压）
//! - Sliding：通道满时丢弃最旧消息（适合出站/内部通信，保证时效性）
//!
//! 设计灵感来自 Clojure core.async 的 dropping-buffer 和 sliding-buffer。

use crate::agent::runtime_error::AgentError;
use crossbeam_channel::{bounded, Receiver, RecvTimeoutError, Sender};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// 缓冲区溢出策略
#[derive(Debug, Clone, Copy)]
pub enum BufferStrategy {
    /// 通道满时丢弃新消息（类似 core.async dropping-buffer）
    Dropping(usize),
    /// 通道满时丢弃最旧消息（类似 core.async sliding-buffer）
    Sliding(usize),
}

impl BufferStrategy {
    fn capacity(&self) -> usize {
        match self {
            BufferStrategy::Dropping(cap) => *cap,
            BufferStrategy::Sliding(cap) => *cap,
        }
    }
}

/// 丢弃事件的回调钩子
pub type DropHook = Arc<dyn Fn() + Send + Sync>;

/// 有界发送端
///
/// 持有发送端和接收端的引用，因为 Sliding 策略需要从接收端弹出旧消息来腾出空间。
pub struct BoundedSender<T> {
    tx: Sender<T>,
    rx: Receiver<T>,
    strategy: BufferStrategy,
    dropped_total: Option<Arc<AtomicU64>>,
    on_drop: Option<DropHook>,
}

impl<T> BoundedSender<T> {
    fn record_drop(&self) {
        if let Some(counter) = &self.dropped_total {
            counter.fetch_add(1, Ordering::SeqCst);
        }
        if let Some(hook) = &self.on_drop {
            hook();
        }
    }

    /// 根据策略发送消息
    /// - Dropping：满时静默丢弃新消息，返回 Ok
    /// - Sliding：满时先弹出旧消息腾出空间，再发送
    pub fn send(&self, msg: T) -> Result<(), AgentError> {
        match self.strategy {
            BufferStrategy::Dropping(_cap) => {
                if self.tx.len() >= self.strategy.capacity() {
                    self.record_drop();
                    return Ok(());
                }
                self.tx.send(msg).map_err(|e| AgentError::BusSendFailed(e.to_string()))
            }
            BufferStrategy::Sliding(_cap) => {
                while self.tx.len() >= self.strategy.capacity() {
                    if self.rx.try_recv().is_ok() {
                        self.record_drop();
                    } else {
                        break;
                    }
                }
                self.tx.send(msg).map_err(|e| AgentError::BusSendFailed(e.to_string()))
            }
        }
    }

    pub fn len(&self) -> usize {
        self.tx.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn is_full(&self) -> bool {
        self.tx.len() >= self.strategy.capacity()
    }
}

/// 有界接收端
pub struct BoundedReceiver<T> {
    rx: Receiver<T>,
}

impl<T> BoundedReceiver<T> {
    pub fn recv(&self) -> Option<T> {
        self.rx.recv().ok()
    }

    pub fn try_recv(&self) -> Option<T> {
        self.rx.try_recv().ok()
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<T, RecvTimeoutError> {
        self.rx.recv_timeout(timeout)
    }

    pub fn is_empty(&self) -> bool {
        self.rx.is_empty()
    }

    pub fn len(&self) -> usize {
        self.rx.len()
    }
}

impl<T> Clone for BoundedReceiver<T> {
    fn clone(&self) -> Self {
        Self {
            rx: self.rx.clone(),
        }
    }
}

/// 创建一对有界通道（发送端 + 接收端）
///
/// 实际容量为 strategy.capacity() + 1，多出的 1 个位置用于 Sliding 策略
/// 在弹出旧消息和写入新消息之间的短暂窗口期避免阻塞。
pub fn bounded_channel<T>(strategy: BufferStrategy) -> (BoundedSender<T>, BoundedReceiver<T>) {
    bounded_channel_with_counter(strategy, None)
}

pub fn bounded_channel_with_counter<T>(
    strategy: BufferStrategy,
    dropped_total: Option<Arc<AtomicU64>>,
) -> (BoundedSender<T>, BoundedReceiver<T>) {
    bounded_channel_full(strategy, dropped_total, None)
}

pub fn bounded_channel_full<T>(
    strategy: BufferStrategy,
    dropped_total: Option<Arc<AtomicU64>>,
    on_drop: Option<DropHook>,
) -> (BoundedSender<T>, BoundedReceiver<T>) {
    let cap = strategy.capacity();
    let (tx, rx) = bounded(cap + 1);
    let sender = BoundedSender {
        tx,
        rx: rx.clone(),
        strategy,
        dropped_total,
        on_drop,
    };
    let receiver = BoundedReceiver { rx };
    (sender, receiver)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    #[test]
    fn drop_counter_increments_on_dropping() {
        let counter = Arc::new(AtomicU64::new(0));
        let (tx, _rx) = bounded_channel_with_counter(BufferStrategy::Dropping(2), Some(counter.clone()));
        tx.send(1).unwrap();
        tx.send(2).unwrap();
        tx.send(3).unwrap(); // dropped
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn drop_counter_increments_on_sliding() {
        let counter = Arc::new(AtomicU64::new(0));
        let (tx, _rx) = bounded_channel_with_counter(BufferStrategy::Sliding(2), Some(counter.clone()));
        tx.send(1).unwrap();
        tx.send(2).unwrap();
        tx.send(3).unwrap(); // evicts oldest
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn on_drop_hook_fires() {
        let fired = Arc::new(AtomicU64::new(0));
        let fired_clone = fired.clone();
        let hook: DropHook = Arc::new(move || { fired_clone.fetch_add(1, Ordering::SeqCst); });
        let (tx, _rx) = bounded_channel_full(BufferStrategy::Dropping(1), None, Some(hook));
        tx.send(1).unwrap();
        tx.send(2).unwrap(); // dropped, hook fires
        assert_eq!(fired.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn is_full_and_is_empty() {
        let (tx, rx) = bounded_channel::<i32>(BufferStrategy::Dropping(1));
        assert!(tx.is_empty());
        assert!(!tx.is_full());
        tx.send(1).unwrap();
        assert!(tx.is_full());
        assert!(!tx.is_empty());
        rx.recv();
        assert!(tx.is_empty());
    }

    #[test]
    fn len_tracks_queue_depth() {
        let (tx, rx) = bounded_channel::<i32>(BufferStrategy::Dropping(3));
        assert_eq!(tx.len(), 0);
        tx.send(1).unwrap();
        tx.send(2).unwrap();
        assert_eq!(tx.len(), 2);
        rx.recv();
        assert_eq!(tx.len(), 1);
    }

    #[test]
    fn recv_timeout_returns_err() {
        let (_tx, rx) = bounded_channel::<i32>(BufferStrategy::Dropping(1));
        let result = rx.recv_timeout(Duration::from_millis(10));
        assert!(result.is_err());
    }

    #[test]
    fn receiver_clone_shares_channel() {
        let (tx, rx) = bounded_channel::<i32>(BufferStrategy::Dropping(2));
        let rx2 = rx.clone();
        tx.send(42).unwrap();
        assert_eq!(rx2.recv(), Some(42));
        assert!(rx.try_recv().is_none());
    }
}
