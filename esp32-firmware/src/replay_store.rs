// Flash-backed replay state for the ESP32

// implementation of secure_link::command::ReplayStore, same process_command()
// used by the std uplink listener decides whether a command is a replay. 
// The only change is where the 17-byte ReplayWindow is saved: here, in flash.


// Layout:
// Each sector is a log of 32-byte slots:
//   [0] magic 0xA7
//   [1..5] sequence number u32 big-endian (newest record = highest sequence)
//   [5..22] ReplayWindow::to_bytes()  (17 bytes)
//   [22] XOR checksum of bytes 0..22
//   [23..32] 0xFF


// Each accepted command appends one slot (no erase). A sector is erased only when the OTHER sector is
// full, and the new record is written to the freshly erased sector, so the newest valid
// record always survives a power loss at any point.

// the first authenticated command starts the window.
// anything else that is unreadable fails closed

#![forbid(unsafe_code)]

use embedded_storage::nor_flash::NorFlash;
use secure_link::ReplayWindow;
use secure_link::command::{CommandError, ReplayStore};

const SLOT: usize = 32;
const MAGIC: u8 = 0xA7;
const SECTORS: usize = 2;

pub struct FlashReplayStore<F: NorFlash> {
    flash: F,
    base: u32,
}

struct Best {
    seq: u32,
    sector: usize,
    window: ReplayWindow,
}

struct Scan {
    best: Option<Best>,
    /// index of the last non-blank slot per sector (None = sector completely blank)
    last_used: [Option<usize>; SECTORS],
    /// a non-blank slot exists that is not a valid record
    garbage: bool,
}

fn checksum(b: &[u8]) -> u8 {
    b.iter().fold(0, |a, x| a ^ x)
}

impl<F: NorFlash> FlashReplayStore<F> {
    /// `base` must be erase-sector aligned and reserve 2 sectors that nothing else uses.
    pub fn new(flash: F, base: u32) -> Self {
        assert!(F::ERASE_SIZE % SLOT == 0 && F::WRITE_SIZE <= SLOT && SLOT % F::WRITE_SIZE == 0);
        assert!(base as usize % F::ERASE_SIZE == 0);
        Self { flash, base }
    }

    pub fn into_inner(self) -> F {
        self.flash
    }

    fn slots(&self) -> usize {
        F::ERASE_SIZE / SLOT
    }

    fn addr(&self, sector: usize, slot: usize) -> u32 {
        self.base + (sector * F::ERASE_SIZE + slot * SLOT) as u32
    }

    fn scan(&mut self) -> Result<Scan, CommandError> {
        let mut scan = Scan { best: None, last_used: [None; SECTORS], garbage: false };
        for sector in 0..SECTORS {
            for slot in 0..self.slots() {
                let mut buf = [0u8; SLOT];
                self.flash
                    .read(self.addr(sector, slot), &mut buf)
                    .map_err(|_| CommandError::Store)?;
                if buf.iter().all(|b| *b == 0xFF) {
                    continue;
                }
                scan.last_used[sector] = Some(slot);
                let valid = buf[0] == MAGIC && checksum(&buf[..22]) == buf[22];
                match valid.then(|| ReplayWindow::from_bytes(&buf[5..22])) {
                    Some(Ok(window)) => {
                        let seq = u32::from_be_bytes([buf[1], buf[2], buf[3], buf[4]]);
                        if scan.best.as_ref().map_or(true, |b| seq > b.seq) {
                            scan.best = Some(Best { seq, sector, window });
                        }
                    }
                    // torn / corrupt slot: ignore it (older valid records still count)
                    _ => scan.garbage = true,
                }
            }
        }
        Ok(scan)
    }

    fn write_slot(&mut self, sector: usize, slot: usize, seq: u32, w: &ReplayWindow) -> Result<(), CommandError> {
        let mut rec = [0xFFu8; SLOT];
        rec[0] = MAGIC;
        rec[1..5].copy_from_slice(&seq.to_be_bytes());
        rec[5..22].copy_from_slice(&w.to_bytes());
        rec[22] = checksum(&rec[..22]);
        self.flash.write(self.addr(sector, slot), &rec).map_err(|_| CommandError::Store)
    }
}

impl<F: NorFlash> ReplayStore for FlashReplayStore<F> {
    fn load(&mut self) -> Result<ReplayWindow, CommandError> {
        let scan = self.scan()?;
        match scan.best {
            Some(b) => Ok(b.window),
            // completely blank flash: first boot
            None if scan.last_used.iter().all(|l| l.is_none()) => Ok(ReplayWindow::new()),
            // data present but nothing valid: fail closed
            None => Err(CommandError::Store),
        }
    }

    fn save(&mut self, w: &ReplayWindow) -> Result<(), CommandError> {
        let scan = self.scan()?;
        let (sector, slot, seq) = match &scan.best {
            None => {
                if scan.garbage {
                    return Err(CommandError::Store);
                }
                (0, 0, 1)
            }
            Some(b) => {
                let next = scan.last_used[b.sector].map_or(0, |l| l + 1);
                let seq = b.seq.checked_add(1).ok_or(CommandError::Store)?;
                if next < self.slots() {
                    (b.sector, next, seq)
                } else {
                    // sector full: erase the other one and continue there.
                    let other = (b.sector + 1) % SECTORS;
                    let from = self.addr(other, 0);
                    self.flash
                        .erase(from, from + F::ERASE_SIZE as u32)
                        .map_err(|_| CommandError::Store)?;
                    (other, 0, seq)
                }
            }
        };
        self.write_slot(sector, slot, seq, w)
    }
}