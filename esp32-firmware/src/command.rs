// Command receive + encrypted ACK logic for the ESP32.
//
// Ported from satellite/uplink_listener/src/main.rs. This file has no hardware
// dependencies, so it can be unit tested.
//
// Command on the wire: [version=1][direction=1] + 12 byte nonce + ciphertext + 16 byte tag
// plaintext = [CMD_ID][SEQ_NUM]
// ACK on the wire: [0xA5][version=1][direction=2] + 12 byte nonce + ciphertext + 16 byte tag
// plaintext = [3][SEQ_NUM]
//
// Must match satellite/uplink_listener and targets/RASCAL/lib/secure_link_protocol.rb.

#![forbid(unsafe_code)]

extern crate alloc;

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use alloc::vec::Vec;

pub const NOOP_CMD_ID: u8 = 1;
pub const ACK_ID: u8 = 3;

const VERSION: u8 = 1;
const COMMAND_DIRECTION: u8 = 1;
const ACK_DIRECTION: u8 = 2;
const ACK_MARKER: u8 = 0xA5;

const HEADER_LEN: usize = 2;
pub const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
const MIN_PACKET_LEN: usize = HEADER_LEN + NONCE_LEN + TAG_LEN;

// Same values as the Ruby protocol and uplink_listener.
// WARNING: TEMP KEYS
// Do not use in production.
const COMMAND_KEY: [u8; 32] = [
    0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
    0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
];
const ACK_KEY: [u8; 32] = [
    0x10, 0x21, 0x32, 0x43, 0x54, 0x65, 0x76, 0x87, 0x98, 0xa9, 0xba, 0xcb, 0xdc, 0xed, 0xfe, 0x0f,
    0x10, 0x21, 0x32, 0x43, 0x54, 0x65, 0x76, 0x87, 0x98, 0xa9, 0xba, 0xcb, 0xdc, 0xed, 0xfe, 0x0f,
];

#[derive(Debug, PartialEq, Eq)]
pub enum CommandError {
    TooShort,
    BadHeader,
    AuthFailed,
    BadLength,
    UnknownCommand(u8),
    EncryptFailed,
}

pub struct SecureLink {
    command_cipher: Aes256Gcm,
    ack_cipher: Aes256Gcm,
}

impl SecureLink {
    pub fn new() -> Self {
        Self {
            command_cipher: Aes256Gcm::new_from_slice(&COMMAND_KEY).expect("32-byte key"),
            ack_cipher: Aes256Gcm::new_from_slice(&ACK_KEY).expect("32-byte key"),
        }
    }

    // Authenticate + decrypt one received datagram. On success returns the
    // encrypted ACK datagram to send back, plus the command's sequence number.
    // `ack_nonce` must be 12 fresh random bytes for every call.
    pub fn handle(
        &self,
        packet: &[u8],
        ack_nonce: [u8; NONCE_LEN],
    ) -> Result<(u8, Vec<u8>), CommandError> {
        if packet.len() < MIN_PACKET_LEN {
            return Err(CommandError::TooShort);
        }

        let header = &packet[..HEADER_LEN];
        if header[0] != VERSION || header[1] != COMMAND_DIRECTION {
            return Err(CommandError::BadHeader);
        }

        let nonce = Nonce::from_slice(&packet[HEADER_LEN..HEADER_LEN + NONCE_LEN]);
        let plaintext = self
            .command_cipher
            .decrypt(
                nonce,
                Payload {
                    msg: &packet[HEADER_LEN + NONCE_LEN..],
                    aad: header,
                },
            )
            .map_err(|_| CommandError::AuthFailed)?;

        if plaintext.len() != 2 {
            return Err(CommandError::BadLength);
        }
        let command_id = plaintext[0];
        let seq = plaintext[1];

        match command_id {
            NOOP_CMD_ID => Ok((seq, self.build_ack(seq, ack_nonce)?)),
            other => Err(CommandError::UnknownCommand(other)),
        }
    }

    fn build_ack(&self, seq: u8, nonce_bytes: [u8; NONCE_LEN]) -> Result<Vec<u8>, CommandError> {
        let ack_header = [ACK_MARKER, VERSION, ACK_DIRECTION];
        let nonce = Nonce::from_slice(&nonce_bytes);
        let encrypted = self
            .ack_cipher
            .encrypt(
                nonce,
                Payload {
                    msg: &[ACK_ID, seq],
                    aad: &ack_header,
                },
            )
            .map_err(|_| CommandError::EncryptFailed)?;

        let mut out = Vec::with_capacity(ack_header.len() + NONCE_LEN + encrypted.len());
        out.extend_from_slice(&ack_header);
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&encrypted);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_command(cmd: u8, seq: u8, nonce: [u8; 12]) -> Vec<u8> {
        let c = Aes256Gcm::new_from_slice(&COMMAND_KEY).unwrap();
        let header = [VERSION, COMMAND_DIRECTION];
        let ct = c
            .encrypt(Nonce::from_slice(&nonce), Payload { msg: &[cmd, seq], aad: &header })
            .unwrap();
        let mut p = Vec::new();
        p.extend_from_slice(&header);
        p.extend_from_slice(&nonce);
        p.extend_from_slice(&ct);
        p
    }

    #[test]
    fn noop_gets_valid_ack() {
        let link = SecureLink::new();
        let (seq, ack) = link.handle(&make_command(1, 42, [7; 12]), [9; 12]).unwrap();
        assert_eq!(seq, 42);
        assert_eq!(ack.len(), 3 + 12 + 2 + 16);
        assert_eq!(&ack[..3], &[0xA5, 1, 2]);
        // decrypt the ACK the way the Ruby side does
        let a = Aes256Gcm::new_from_slice(&ACK_KEY).unwrap();
        let pt = a
            .decrypt(Nonce::from_slice(&ack[3..15]), Payload { msg: &ack[15..], aad: &ack[..3] })
            .unwrap();
        assert_eq!(pt, [3, 42]);
    }

    #[test]
    fn rejects_tampering_and_garbage() {
        let link = SecureLink::new();
        let mut p = make_command(1, 5, [1; 12]);
        *p.last_mut().unwrap() ^= 1;
        assert_eq!(link.handle(&p, [0; 12]).unwrap_err(), CommandError::AuthFailed);
        assert_eq!(link.handle(&[1, 2, 3], [0; 12]).unwrap_err(), CommandError::TooShort);
        let mut h = make_command(1, 5, [1; 12]);
        h[1] = 2;
        assert_eq!(link.handle(&h, [0; 12]).unwrap_err(), CommandError::BadHeader);
        assert_eq!(
            link.handle(&make_command(9, 5, [1; 12]), [0; 12]).unwrap_err(),
            CommandError::UnknownCommand(9)
        );
    }
}