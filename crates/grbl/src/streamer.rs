//! Character counting (GRBL 1.1 Interface, "Streaming Protocol: Character-Counting"): keep the
//! bytes sent but not yet answered at or under the machine's receive buffer, so it never
//! overflows. Each line counts its length plus the newline; each `ok` or `error` answers the oldest.

use std::collections::VecDeque;

/// Bytes in flight in the machine's receive buffer.
#[derive(Clone, Debug)]
pub struct Streamer {
    capacity: usize,
    used: usize,
    sent: VecDeque<usize>,
}

impl Streamer {
    /// A streamer for a receive buffer of `capacity` bytes (at least 2).
    pub fn new(capacity: usize) -> Streamer {
        Streamer { capacity: capacity.max(2), used: 0, sent: VecDeque::new() }
    }

    /// The receive buffer size counted against.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Bytes sent and not yet answered.
    pub fn used(&self) -> usize {
        self.used
    }

    /// Lines sent and not yet answered.
    pub fn in_flight(&self) -> usize {
        self.sent.len()
    }

    /// True when nothing is waiting for an answer.
    pub fn is_empty(&self) -> bool {
        self.sent.is_empty()
    }

    /// Whether a line of `len` bytes (without its newline) fits now.
    pub fn fits(&self, len: usize) -> bool {
        len.saturating_add(1).saturating_add(self.used) <= self.capacity
    }

    /// Count a line of `len` bytes (plus its newline) as sent. False (nothing counted) when it
    /// doesn't fit: the caller must not send it.
    pub fn push(&mut self, len: usize) -> bool {
        if !self.fits(len) {
            return false;
        }
        self.used += len + 1;
        self.sent.push_back(len + 1);
        true
    }

    /// An `ok` or `error` arrived: the oldest line is answered. Its byte count, or `None` when
    /// nothing was in flight.
    pub fn ack(&mut self) -> Option<usize> {
        let n = self.sent.pop_front()?;
        self.used = self.used.saturating_sub(n);
        Some(n)
    }

    /// Forget everything in flight (after a reset, the machine purged its buffer).
    pub fn clear(&mut self) {
        self.used = 0;
        self.sent.clear();
    }

    /// Change the buffer size (from the machine's `[OPT:…]`), keeping what is in flight.
    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity.max(2);
    }
}
