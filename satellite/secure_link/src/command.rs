//! Shared command handling: the one place that knows how a RASCAL command is
//! authenticated, replay-checked and acknowledged. Used by the std uplink listener
//! and by the no_std ESP32 firmware.
//!
//! Command on the wire: [version=1][direction=1] + nonce(12) + ciphertext + tag(16)
//! plaintext = [CMD_ID][SEQ_NUM]; nonce = [0,0,0,4] + counter (u64 big-endian).
//! Order of checks: authenticate -> length -> source prefix -> known command -> replay -> ACK.
//! (Only authenticated, known commands consume replay counters.)

use alloc::vec::Vec;

use crate::{decrypt_packet, encrypt_packet, ReplayWindow, SecureLinkError};

pub const NOOP_CMD_ID: u8 = 1;
pub const ACK_ID: u8 = 3;

pub const VERSION: u8 = 1;
pub const COMMAND_DIRECTION: u8 = 1;
pub const ACK_DIRECTION: u8 = 2;
pub const ACK_MARKER: u8 = 0xA5;

pub const COMMAND_HEADER: [u8; 2] = [VERSION, COMMAND_DIRECTION];
pub const ACK_HEADER: [u8; 3] = [ACK_MARKER, VERSION, ACK_DIRECTION];
/// First four nonce bytes of every command OpenC3 sends.
pub const COMMAND_SOURCE_PREFIX: [u8; 4] = [0, 0, 0, 4];

#[derive(Debug)]
pub enum CommandError {
    Link(SecureLinkError),
    BadLength,
    BadSourcePrefix,
    UnknownCommand(u8),
    /// Duplicate or too-old counter.
    Replay(u64),
    /// Replay state could not be loaded/saved. The command is NOT executed (fail closed).
    Store,
}

impl From<SecureLinkError> for CommandError {
    fn from(e: SecureLinkError) -> Self {
        CommandError::Link(e)
    }
}

/// Where the replay window lives (file on the satellite sim, flash on the ESP32).
pub trait ReplayStore {
    fn load(&mut self) -> Result<ReplayWindow, CommandError>;
    fn save(&mut self, window: &ReplayWindow) -> Result<(), CommandError>;

    /// Check `counter` and persist the new window *before* returning Ok(true).
    /// Stores that need extra atomicity (file lock) override this.
    fn check_and_persist(&mut self, counter: u64) -> Result<bool, CommandError> {
        let mut window = self.load()?;
        if !window.check_and_update(counter) {
            return Ok(false);
        }
        self.save(&window)?;
        Ok(true)
    }
}

/// Source of fresh ACK nonces.
pub trait NonceSource {
    fn next_nonce(&mut self) -> Result<[u8; 12], SecureLinkError>;
}

impl NonceSource for crate::counter::NonceCounter {
    fn next_nonce(&mut self) -> Result<[u8; 12], SecureLinkError> {
        crate::counter::NonceCounter::next_nonce(self)
    }
}

#[cfg(feature = "std")]
impl NonceSource for crate::NonceManager {
    fn next_nonce(&mut self) -> Result<[u8; 12], SecureLinkError> {
        crate::NonceManager::next_nonce(self)
    }
}

/// Result of a successful authentication (before the replay check).
#[derive(Debug, PartialEq, Eq)]
pub struct AuthenticatedCommand {
    pub id: u8,
    pub seq: u8,
    pub counter: u64,
}

pub fn authenticate_command(
    packet: &[u8],
    command_key: &[u8; 32],
) -> Result<AuthenticatedCommand, CommandError> {
    let plaintext = decrypt_packet(packet, command_key, &COMMAND_HEADER)?;
    if plaintext.len() != 2 {
        return Err(CommandError::BadLength);
    }
    // decrypt_packet already proved packet.len() >= header + nonce + tag
    let nonce = &packet[COMMAND_HEADER.len()..COMMAND_HEADER.len() + crate::NONCE_LEN];
    if nonce[..4] != COMMAND_SOURCE_PREFIX {
        return Err(CommandError::BadSourcePrefix);
    }
    let mut c = [0u8; 8];
    c.copy_from_slice(&nonce[4..12]);
    Ok(AuthenticatedCommand { id: plaintext[0], seq: plaintext[1], counter: u64::from_be_bytes(c) })
}

pub fn build_ack(seq: u8, ack_key: &[u8; 32], nonce: &[u8; 12]) -> Result<Vec<u8>, CommandError> {
    Ok(encrypt_packet(&[ACK_ID, seq], ack_key, &ACK_HEADER, nonce)?)
}

/// Full pipeline. Returns (sequence number, encrypted ACK datagram to send).
pub fn process_command<S: ReplayStore, N: NonceSource>(
    packet: &[u8],
    command_key: &[u8; 32],
    ack_key: &[u8; 32],
    store: &mut S,
    ack_nonces: &mut N,
) -> Result<(u8, Vec<u8>), CommandError> {
    let cmd = authenticate_command(packet, command_key)?;

    if cmd.id != NOOP_CMD_ID {
        return Err(CommandError::UnknownCommand(cmd.id));
    }

    if !store.check_and_persist(cmd.counter)? {
        return Err(CommandError::Replay(cmd.counter));
    }

    let nonce = ack_nonces.next_nonce()?;
    Ok((cmd.seq, build_ack(cmd.seq, ack_key, &nonce)?))
}

/// RAM-only store, for tests and for bring-up. NOT safe across reboots.
pub struct MemoryReplayStore(pub ReplayWindow);

impl ReplayStore for MemoryReplayStore {
    fn load(&mut self) -> Result<ReplayWindow, CommandError> {
        ReplayWindow::from_bytes(&self.0.to_bytes()).map_err(|_| CommandError::Store)
    }
    fn save(&mut self, w: &ReplayWindow) -> Result<(), CommandError> {
        self.0 = ReplayWindow::from_bytes(&w.to_bytes()).map_err(|_| CommandError::Store)?;
        Ok(())
    }
}

/// File-backed store (satellite simulators): same lock + atomic-rename logic as before.
#[cfg(feature = "std")]
pub struct FileReplayStore {
    pub state_path: std::path::PathBuf,
    pub lock_path: std::path::PathBuf,
}

#[cfg(feature = "std")]
impl FileReplayStore {
    pub fn new(state: impl Into<std::path::PathBuf>, lock: impl Into<std::path::PathBuf>) -> Self {
        Self { state_path: state.into(), lock_path: lock.into() }
    }
}

#[cfg(feature = "std")]
impl ReplayStore for FileReplayStore {
    fn load(&mut self) -> Result<ReplayWindow, CommandError> {
        ReplayWindow::load_from_file(&self.state_path).map_err(|_| CommandError::Store)
    }
    fn save(&mut self, w: &ReplayWindow) -> Result<(), CommandError> {
        w.save_to_file(&self.state_path).map_err(|_| CommandError::Store)
    }
    fn check_and_persist(&mut self, counter: u64) -> Result<bool, CommandError> {
        crate::check_and_persist_replay(&self.state_path, &self.lock_path, counter)
            .map_err(|_| CommandError::Store)
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::counter::NonceCounter;

    const CK: [u8; 32] = [0x11; 32];
    const AK: [u8; 32] = [0x22; 32];

    fn cmd(id: u8, seq: u8, counter: u64) -> Vec<u8> {
        let mut nonce = [0u8; 12];
        nonce[..4].copy_from_slice(&COMMAND_SOURCE_PREFIX);
        nonce[4..].copy_from_slice(&counter.to_be_bytes());
        encrypt_packet(&[id, seq], &CK, &COMMAND_HEADER, &nonce).unwrap()
    }

    fn run(p: &[u8], s: &mut MemoryReplayStore, n: &mut NonceCounter) -> Result<(u8, Vec<u8>), CommandError> {
        process_command(p, &CK, &AK, s, n)
    }

    #[test]
    fn accepts_then_rejects_replay_and_acks_correctly() {
        let mut s = MemoryReplayStore(ReplayWindow::new());
        let mut n = NonceCounter::with_start([0, 0, 0, 6], 100);
        let p = cmd(1, 42, 7);
        let (seq, ack) = run(&p, &mut s, &mut n).unwrap();
        assert_eq!(seq, 42);
        assert_eq!(&ack[..3], &ACK_HEADER);
        assert_eq!(&ack[3..7], &[0, 0, 0, 6]);
        assert_eq!(&ack[7..15], &100u64.to_be_bytes());
        assert_eq!(crate::decrypt_packet(&ack, &AK, &ACK_HEADER).unwrap(), [ACK_ID, 42]);
        assert!(matches!(run(&p, &mut s, &mut n), Err(CommandError::Replay(7))));
    }

    #[test]
    fn out_of_order_within_window_ok_old_rejected() {
        let mut s = MemoryReplayStore(ReplayWindow::new());
        let mut n = NonceCounter::with_start([0, 0, 0, 6], 0);
        run(&cmd(1, 1, 100), &mut s, &mut n).unwrap();
        run(&cmd(1, 2, 98), &mut s, &mut n).unwrap();
        assert!(run(&cmd(1, 3, 98), &mut s, &mut n).is_err());
        assert!(matches!(run(&cmd(1, 4, 30), &mut s, &mut n), Err(CommandError::Replay(30))));
    }

    #[test]
    fn rejects_bad_inputs_without_consuming_counter() {
        let mut s = MemoryReplayStore(ReplayWindow::new());
        let mut n = NonceCounter::with_start([0, 0, 0, 6], 0);
        let mut bad = cmd(1, 1, 5);
        *bad.last_mut().unwrap() ^= 1;
        assert!(matches!(run(&bad, &mut s, &mut n), Err(CommandError::Link(SecureLinkError::AuthenticationFailed))));
        assert!(matches!(run(&cmd(9, 1, 5), &mut s, &mut n), Err(CommandError::UnknownCommand(9))));
        // counter 5 was never consumed by the failures above
        assert!(run(&cmd(1, 1, 5), &mut s, &mut n).is_ok());
        // wrong source prefix
        let mut nonce = [0u8; 12];
        nonce[..4].copy_from_slice(&[0, 0, 0, 9]);
        let p = encrypt_packet(&[1, 1], &CK, &COMMAND_HEADER, &nonce).unwrap();
        assert!(matches!(run(&p, &mut s, &mut n), Err(CommandError::BadSourcePrefix)));
        assert!(matches!(run(&[1, 2, 3], &mut s, &mut n), Err(CommandError::Link(SecureLinkError::InvalidPacket))));
    }

    #[test]
    fn state_survives_reboot_via_bytes() {
        let mut s = MemoryReplayStore(ReplayWindow::new());
        let mut n = NonceCounter::with_start([0, 0, 0, 6], 0);
        run(&cmd(1, 1, 50), &mut s, &mut n).unwrap();
        // "reboot": rebuild the store from the saved 17 bytes
        let saved = s.0.to_bytes();
        let mut s2 = MemoryReplayStore(ReplayWindow::from_bytes(&saved).unwrap());
        assert!(matches!(run(&cmd(1, 1, 50), &mut s2, &mut n), Err(CommandError::Replay(50))));
        assert!(run(&cmd(1, 2, 51), &mut s2, &mut n).is_ok());
    }

    #[test]
    fn file_store_matches_memory_store() {
        let dir = std::env::temp_dir().join(format!("sl_cmd_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (sp, lp) = (dir.join("r.bin"), dir.join("r.lock"));
        let _ = std::fs::remove_file(&sp);
        crate::initialize_replay_state(&sp, &lp).unwrap();
        let mut s = FileReplayStore::new(&sp, &lp);
        let mut n = NonceCounter::with_start([0, 0, 0, 1], 0);
        assert!(process_command(&cmd(1, 1, 3), &CK, &AK, &mut s, &mut n).is_ok());
        assert!(matches!(process_command(&cmd(1, 1, 3), &CK, &AK, &mut s, &mut n), Err(CommandError::Replay(3))));
        std::fs::remove_dir_all(&dir).ok();
    }
}