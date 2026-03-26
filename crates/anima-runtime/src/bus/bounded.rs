use crossbeam_channel::{bounded, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub enum BufferStrategy {
    /// Channel is full → drop the new message (like core.async dropping-buffer)
    Dropping(usize),
    /// Channel is full → drop the oldest message (like core.async sliding-buffer)
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

pub struct BoundedSender<T> {
    tx: Sender<T>,
    rx: Receiver<T>,
    strategy: BufferStrategy,
}

impl<T> BoundedSender<T> {
    pub fn send(&self, msg: T) -> Result<(), String> {
        match self.strategy {
            BufferStrategy::Dropping(_cap) => {
                if self.tx.len() >= self.strategy.capacity() {
                    return Ok(());
                }
                self.tx.send(msg).map_err(|e| e.to_string())
            }
            BufferStrategy::Sliding(_cap) => {
                while self.tx.len() >= self.strategy.capacity() {
                    let _ = self.rx.try_recv();
                }
                self.tx.send(msg).map_err(|e| e.to_string())
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

pub fn bounded_channel<T>(strategy: BufferStrategy) -> (BoundedSender<T>, BoundedReceiver<T>) {
    let cap = strategy.capacity();
    let (tx, rx) = bounded(cap + 1);
    let sender = BoundedSender {
        tx,
        rx: rx.clone(),
        strategy,
    };
    let receiver = BoundedReceiver { rx };
    (sender, receiver)
}
