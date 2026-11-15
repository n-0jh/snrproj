// ported to ESP32 in the same downlink format as EPS and ADCS:
//   [0xA5][version=1][direction=2] + 12-byte nonce + ciphertext + 16-byte tag
// The plaintext is the original telemetry packet, starting with its packet ID.

#![forbid(unsafe_code)]

extern crate alloc;

use alloc::vec::Vec;

use crate::keys::DOWNLINK_KEY;
use secure_link::counter::NonceCounter;
use secure_link::{SecureLinkError, encrypt_packet};

pub const DOWNLINK_MARKER: u8 = 0xA5;
pub const VERSION: u8 = 1;
pub const DOWNLINK_DIRECTION: u8 = 2;
pub const DOWNLINK_HEADER: [u8; 3] = [DOWNLINK_MARKER, VERSION, DOWNLINK_DIRECTION];

// Source prefixes already used: 1 = uplink_listener ACK, 2 = EPS, 3 = ADCS.
pub const ESP32_TLM_PREFIX: [u8; 4] = [0, 0, 0, 5];
pub const ESP32_ACK_PREFIX: [u8; 4] = [0, 0, 0, 6];

pub struct TelemetryEncryptor {
    nonces: NonceCounter,
}

impl TelemetryEncryptor {
    pub fn new(counter_start: u64) -> Self {
        Self { nonces: NonceCounter::with_start(ESP32_TLM_PREFIX, counter_start) }
    }

    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, SecureLinkError> {
        let nonce = self.nonces.next_nonce()?;
        encrypt_packet(plaintext, &DOWNLINK_KEY, &DOWNLINK_HEADER, &nonce)
    }
}