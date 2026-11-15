//! In-memory nonce counter (no file system needed).
//!
//! Used by the ESP32. The satellite programs keep using the file-backed `NonceManager`.
//! Nonce layout is identical: 4-byte source prefix + 8-byte big-endian counter.

use crate::SecureLinkError;

pub struct NonceCounter {
    prefix: [u8; 4],
    next: u64,
    exhausted: bool,
}

impl NonceCounter {
    pub fn with_start(prefix: [u8; 4], start: u64) -> Self {
        Self { prefix, next: start, exhausted: false }
    }

    /// Never wraps: once the counter is used up, every call returns CounterExhausted.
    pub fn next_nonce(&mut self) -> Result<[u8; 12], SecureLinkError> {
        if self.exhausted {
            return Err(SecureLinkError::CounterExhausted);
        }
        let counter = self.next;
        match counter.checked_add(1) {
            Some(n) => self.next = n,
            None => self.exhausted = true,
        }
        let mut nonce = [0u8; 12];
        nonce[..4].copy_from_slice(&self.prefix);
        nonce[4..].copy_from_slice(&counter.to_be_bytes());
        Ok(nonce)
    }
}