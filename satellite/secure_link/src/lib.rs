use aes_gcm::{aead::{Aead, KeyInit, Payload}, Aes256Gcm, Nonce};
//use rand::{rngs::OsRng, RngCore};

use std::fs::{self,File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use fs2::FileExt;


// 12-byte nonce + 16-byte tag
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

#[derive(Debug)]
pub enum SecureLinkError 
{
    InvalidPacket,
    InvalidHeader,
    AuthenticationFailed,
    EncryptionFailed,
    Io(std::io::Error),
    CounterExhausted,
}


pub struct NonceManager
{
    source_prefix: [u8; 4],
    counter_file: File //This field stores a Rust File object representing an open counter file on your computer.The important advantage of storing the counter in a file is that the counter can survive when the Rust program stops and restarts.
}

impl NonceManager // functions for the NonceManager structure 
{
    pub fn new( source_prefix: [u8; 4], counter_path: &Path ) -> Result<Self, SecureLinkError> //constructor-like function in Rust.
    {
        // Open an existing counter file.
        // Never silently reset a missing counter to zero.
        let counter_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(counter_path)
            .map_err(SecureLinkError::Io)?;

        // A valid counter file contains exactly 8 bytes.
        let size = counter_file
            .metadata()
            .map_err(SecureLinkError::Io)?
            .len();

        if size != 8 {
            return Err(SecureLinkError::InvalidPacket);
        }

        Ok(Self {    //return statement 
            source_prefix,
            counter_file,
        })
    }

    pub fn next_nonce(&mut self) -> Result<[u8; 12], SecureLinkError>
    {
        // Lock the counter file against other processes.
        self.counter_file
            .lock_exclusive()
            .map_err(SecureLinkError::Io)?;

        let result = self.reserve_nonce(); //That method does the actual work of reading the counter, updating the file, and constructing the nonce.

        // Release the lock, even if reservation failed.
        let unlock_result = FileExt::unlock(&self.counter_file);

        match (result, unlock_result) 
        {
            (Ok(nonce), Ok(())) => Ok(nonce), // if Reservation and unlock succeeded , Return the Nonce 
            (Err(error), _) => Err(error),  // if  Reservation failed , Return error
            (Ok(_), Err(error)) => Err(SecureLinkError::Io(error)), // if Reservation succeeded & unlock failed, Return I/O error  
        }
    }

    fn reserve_nonce(&mut self) -> Result<[u8; 12], SecureLinkError> //this method is private to its module by default.
    {
        // Read the next available counter.
        self.counter_file
            .seek(SeekFrom::Start(0))
            .map_err(SecureLinkError::Io)?;

        let mut counter_bytes = [0u8; 8];

        self.counter_file
            .read_exact(&mut counter_bytes)
            .map_err(SecureLinkError::Io)?;

        let counter = u64::from_be_bytes(counter_bytes);

        // Increment counter 
        let next_counter = counter
            .checked_add(1) //If the counter reaches that maximum, we must not wrap around and reuse earlier nonce values.
            .ok_or(SecureLinkError::CounterExhausted)?;

        // move file cursor back to zero before wrting new value
        self.counter_file
            .seek(SeekFrom::Start(0))
            .map_err(SecureLinkError::Io)?;

        //write new counter value 
        self.counter_file 
            .write_all(&next_counter.to_be_bytes())
            .map_err(SecureLinkError::Io)?;

         //This asks the operating system to synchronize the file's data and metadata to storage.
        self.counter_file 
            .sync_all()
            .map_err(SecureLinkError::Io)?;

        // Construct the 12-byte nonce.
        let mut nonce = [0u8; 12];

        nonce[..4].copy_from_slice(&self.source_prefix);
        nonce[4..].copy_from_slice(&counter_bytes);

        Ok(nonce)
    }
}





// Packet format: [authenticated header][12-byte nonce][ciphertext][16-byte tag].
pub fn encrypt_packet( plaintext: &[u8], key: &[u8; 32], header: &[u8], nonce_bytes: &[u8; 12]) -> Result<Vec<u8>, SecureLinkError>
{
    if header.is_empty() 
    { 
        return Err(SecureLinkError::InvalidHeader);
    }


    let cipher = Aes256Gcm::new_from_slice(key).expect("32-byte AES key"); //Initialize the AES-256-GCM cipher

    // Encrypt the ACK and authenticate the header.
    let encrypted = cipher.encrypt(
        Nonce::from_slice(nonce_bytes),
        Payload 
        { 
            msg: plaintext, // encyrpted
            aad: header
        },
    ).map_err(|_| SecureLinkError::EncryptionFailed)?;

    // [marker][version][direction][nonce][ciphertext][tag]
    //-------------HEADER-------//
    let mut packet = Vec::with_capacity(header.len() + NONCE_LEN + encrypted.len());
    packet.extend_from_slice(header);
    packet.extend_from_slice(nonce_bytes);
    packet.extend_from_slice(&encrypted);
    Ok(packet)
}

pub fn decrypt_packet( packet: &[u8], key: &[u8; 32], expected_header: &[u8] ) -> Result<Vec<u8>, SecureLinkError>
{
    if expected_header.is_empty()
    { 
        return Err(SecureLinkError::InvalidHeader);
    }

    let header_len = expected_header.len();


    if packet.len() < header_len + NONCE_LEN + TAG_LEN
    {
        return Err(SecureLinkError::InvalidPacket);
    }

    let (header, rest) = packet.split_at(header_len);
    if header != expected_header 
    { 
        return Err(SecureLinkError::InvalidHeader);
    }

    let (nonce, ciphertext_and_tag) = rest.split_at(NONCE_LEN);

    let cipher = Aes256Gcm::new_from_slice(key).expect("32-byte AES key");  //Initialize the AES-256-GCM cipher

    return cipher.decrypt // return plain text if succesful 
    (
        Nonce::from_slice(nonce),
        Payload 
        { 
            msg: ciphertext_and_tag,
            aad: header
        },
    ).map_err(|_| SecureLinkError::AuthenticationFailed)
}


pub struct ReplayWindow 
{
    highest_counter: u64, //Stores the largest command nonce counter that has been accepted.
    seen_bitmap: u64, //A 64-bit number that tracks which counters within the sliding window have already been accepted.
    initialized: bool, //This tells us whether the replay window has accepted its first packet.
}

impl ReplayWindow 
{
    pub fn new() -> Self { //constructor , initializes an empty window.
        Self {
            highest_counter: 0,
            seen_bitmap: 0,
            initialized: false,
        }
    }



    pub fn check_and_update(&mut self, counter: u64) -> bool //A return value of true means the counter was accepted and recorded. false means it was rejected.
    {
        // First authenticated packet.
        if !self.initialized //If no counter has been accepted yet, Rust enters this block
        {
            self.highest_counter = counter;
            self.seen_bitmap = 1;
            self.initialized = true;
            return true;
        }

        // Case 1: Incoming counter is newer than our highest.
        if counter > self.highest_counter
        {
            let shift = counter - self.highest_counter; //Calculates how far forward the window must move.

            if shift >= 64 
            {
                self.seen_bitmap = 1;
            } 
            else
            {
                self.seen_bitmap = (self.seen_bitmap << shift) | 1; // shifts bitmap to accpet new seen number
            }

            self.highest_counter = counter;
            return true;
        }

        // Case 2: Incoming counter is equal to or below our highest.
        let difference = self.highest_counter - counter; //This tells us which bit represents counter

        // Outside the 64-packet window.Checks whether the incoming counter is too old.
        if difference >= 64 
        {
            return false;
        }

        let mask = 1u64 << difference; //shifts that bit left by the number of positions calculated earlier.

        // Already received this counter.Detect duplicate counters
        if self.seen_bitmap & mask != 0 {
            return false;
        }

        // New counter received out of order.
        self.seen_bitmap |= mask;
        true
    }


    pub fn to_bytes(&self) -> [u8; 17]
    {
        let mut bytes = [0u8; 17]; //Creates 17 bytes, initially set to zero.

        // Byte 0: Whether the window has been initialized.
        bytes[0] = if self.initialized { 1 } else { 0 };

        // Bytes 1-8: Highest accepted counter.
        bytes[1..9].copy_from_slice(&self.highest_counter.to_be_bytes());

        // Bytes 9-16: Bitmap of accepted counters.
        bytes[9..17].copy_from_slice(&self.seen_bitmap.to_be_bytes());

        return bytes
    } 


    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SecureLinkError> 
    {
        // Replay state must contain exactly 17 bytes.
        if bytes.len() != 17 {
            return Err(SecureLinkError::InvalidPacket);
        }

        // Byte 0 contains the initialized flag.
        let initialized = match bytes[0] {
            0 => false,
            1 => true,
            _ => return Err(SecureLinkError::InvalidPacket),
        };

        // Extract bytes 1-8 for the highest counter.
        let mut counter_bytes = [0u8; 8];
        counter_bytes.copy_from_slice(&bytes[1..9]);

        let highest_counter = u64::from_be_bytes(counter_bytes);

        // Extract bytes 9-16 for the bitmap.
        let mut bitmap_bytes = [0u8; 8];
        bitmap_bytes.copy_from_slice(&bytes[9..17]);

        let seen_bitmap = u64::from_be_bytes(bitmap_bytes);

        // Validate that the stored replay state is consistent.
        if initialized 
        {
            // Bit 0 must be set for the highest accepted counter.
            if seen_bitmap & 1 == 0 {
                return Err(SecureLinkError::InvalidPacket);
            }
        } 
        else 
        {
            // An uninitialized window must contain no accepted counters.
            if highest_counter != 0 || seen_bitmap != 0 {
                return Err(SecureLinkError::InvalidPacket);
            }
        }

        Ok(Self { //This code creates a new ReplayWindow struct using the values we extracted from the 17 bytes and returns it as a successful result.
            highest_counter,
            seen_bitmap,
            initialized,
        })
    }



    pub fn save_to_file(&self, path: &Path) -> std::io::Result<()>
    {
        // Convert the replay window into 17 bytes.
        let bytes = self.to_bytes();

        // Create a temporary file in the same directory.
        let temp_path = path.with_extension("tmp");


        // A leftover temporary file may indicate an interrupted save.
        // Do not silently delete it or overwrite it.
        if temp_path.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "Replay-state temporary file exists; recovery required",
            ));
}




        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;

        // Write all 17 bytes to the temporary file.
        file.write_all(&bytes)?;

        // Flush the file contents to disk.
        file.sync_all()?;

        // Close the temporary file before replacing the original.
        drop(file);

        // Replace the old replay state with the new state.
        fs::rename(&temp_path, path)?;

        Ok(())
    }

    pub fn load_from_file(path: &Path) -> std::io::Result<Self> 
    {
        // Open the existing replay-state file.
        let mut file = File::open(path)?;

        // Read the entire file into a byte vector.
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;

        // Reconstruct the replay window from the saved bytes.
        let window = Self::from_bytes(&bytes).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid replay state file",
            )
        })?;

        Ok(window)
    }
}

pub fn lock_replay_state(lock_path: &Path) -> std::io::Result<File> 
{
    // Open or create the dedicated lock file.
    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(lock_path)?;

    // Wait until exclusive access is available.
    lock_file.lock_exclusive()?;

    // Return the locked file handle.
    Ok(lock_file)
}




pub fn check_and_persist_replay( state_path: &Path, lock_path: &Path,counter: u64 ) -> std::io::Result<bool> 
{
    // Step 1: Acquire exclusive access.
    let _lock_file = lock_replay_state(lock_path)?;

    // Step 2: Load the previous replay state.
    let mut window = ReplayWindow::load_from_file(state_path)?;

    // Step 3: Check whether this counter is acceptable.
    if !window.check_and_update(counter) {
        return Ok(false);
    }

    // Step 4: Save the updated state before accepting the command.
    window.save_to_file(state_path)?;

    // Step 5: Return success.
    Ok(true)
}






//Add an explicit initialization function// Important: This function must only be called during an explicitly authorized first-time setup, such as provisioning a new command key or key epoch. It must not be called automatically whenever the satellite starts.//
pub fn initialize_replay_state(
    state_path: &Path,
    lock_path: &Path,
) -> std::io::Result<()> {
    let _lock_file = lock_replay_state(lock_path)?;

    // Never overwrite an existing replay-state file.
    if state_path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "Replay state already exists",
        ));
    }

    // Never initialize while a temporary state file exists.
    let temp_path = state_path.with_extension("tmp");

    if temp_path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "Replay-state temporary file exists",
        ));
    }

    // Create an empty replay window.
    let window = ReplayWindow::new();

    window.save_to_file(state_path)?;

    Ok(())
}














#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_counter() {
        let mut window = ReplayWindow::new();

        assert!(window.check_and_update(10));
        assert!(window.check_and_update(11));
        assert!(window.check_and_update(12));
    }

    #[test]
    fn test_duplicate_counter() {
        let mut window = ReplayWindow::new();

        assert!(window.check_and_update(10));
        assert!(!window.check_and_update(10));
    }

    #[test]
    fn test_out_of_order_counter() {
        let mut window = ReplayWindow::new();

        assert!(window.check_and_update(10));
        assert!(window.check_and_update(12));

        // Counter 11 was skipped, so accept it.
        assert!(window.check_and_update(11));

        // But don't accept counter 11 twice.
        assert!(!window.check_and_update(11));
    }

    #[test]
    fn test_counter_too_old() {
        let mut window = ReplayWindow::new();

        assert!(window.check_and_update(10));
        assert!(window.check_and_update(100));

        // Counter 10 is now outside the window.
        assert!(!window.check_and_update(10));
    }


    #[test]
    fn test_replay_window_serialization() 
    {
        // Step 1: Create a new replay window.
        let mut window = ReplayWindow::new();

        // Step 2: Accept counters 10, 11, and 12.
        assert!(window.check_and_update(10));
        assert!(window.check_and_update(11));
        assert!(window.check_and_update(12));

        // Step 3: Convert the replay window into 17 bytes.
        let saved_bytes = window.to_bytes();

        // Step 4: Reconstruct the replay window from those bytes.
        let mut restored_window = ReplayWindow::from_bytes(&saved_bytes).unwrap();

        // Step 5: Previously accepted counters must be rejected.
        assert!(!restored_window.check_and_update(10));
        assert!(!restored_window.check_and_update(11));
        assert!(!restored_window.check_and_update(12));

        // Step 6: A new counter should still be accepted.
        assert!(restored_window.check_and_update(13));
    }


    #[test]
    fn test_replay_window_file_persistence()
    {
        use std::time::{SystemTime, UNIX_EPOCH};

        // Create a unique temporary directory for this test.
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        let test_dir = std::env::temp_dir()
            .join(format!("replay_test_{}_{}", std::process::id(), timestamp));

        std::fs::create_dir(&test_dir).unwrap();

        let path = test_dir.join("command_replay.bin");

        // Step 1: Create a replay window.
        let mut window = ReplayWindow::new();

        // Step 2: Accept counters 10, 11, and 12.
        assert!(window.check_and_update(10));
        assert!(window.check_and_update(11));
        assert!(window.check_and_update(12));

        // Step 3: Save the replay window to disk.
        window.save_to_file(&path).unwrap();

        // Step 4: Load the replay window from disk.
        let mut restored_window = ReplayWindow::load_from_file(&path).unwrap();

        // Step 5: Previously accepted counters must be rejected.
        assert!(!restored_window.check_and_update(10));
        assert!(!restored_window.check_and_update(11));
        assert!(!restored_window.check_and_update(12));

        // Step 6: A new counter should be accepted.
        assert!(restored_window.check_and_update(13));

        // Step 7: Clean up the test directory.
        std::fs::remove_dir_all(&test_dir).unwrap();
    }

    #[test]
    fn test_replay_window_repeated_saves() 
    {
        use std::time::{SystemTime, UNIX_EPOCH};

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        let test_dir = std::env::temp_dir()
            .join(format!("replay_update_{}_{}", std::process::id(), timestamp));

        std::fs::create_dir(&test_dir).unwrap();

        let path = test_dir.join("command_replay.bin");

        // Create a replay window and accept counter 10.
        let mut window = ReplayWindow::new();

        assert!(window.check_and_update(10));

        // First save: command_replay.bin does not exist yet.
        window.save_to_file(&path).unwrap();

        // Accept two more counters.
        assert!(window.check_and_update(11));
        assert!(window.check_and_update(12));

        // Second save: command_replay.bin already exists.
        window.save_to_file(&path).unwrap();

        // Load the most recently saved state.
        let mut restored = ReplayWindow::load_from_file(&path).unwrap();

        // All three counters should be remembered.
        assert!(!restored.check_and_update(10));
        assert!(!restored.check_and_update(11));
        assert!(!restored.check_and_update(12));

        // A new counter should be accepted.
        assert!(restored.check_and_update(13));

        std::fs::remove_dir_all(&test_dir).unwrap();
    }


    #[test]
    fn test_missing_replay_state_file() 
    {
        use std::time::{SystemTime, UNIX_EPOCH};

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        let path = std::env::temp_dir().join(format!(
            "missing_replay_{}_{}.bin",
            std::process::id(),
            timestamp
        ));

        // A missing replay-state file must return an error.
        let result = ReplayWindow::load_from_file(&path);

        assert!(result.is_err());
    }



    #[test]
    fn test_corrupted_replay_state_file()
    {
        use std::time::{SystemTime, UNIX_EPOCH};

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        let path = std::env::temp_dir().join(format!(
            "corrupted_replay_{}_{}.bin",
            std::process::id(),
            timestamp
        ));

        // Create an invalid replay-state file.
        std::fs::write(&path, [1u8, 2, 3, 4, 5]).unwrap();

        // Loading must fail because the file is not 17 bytes.
        let result = ReplayWindow::load_from_file(&path);

        assert!(result.is_err());

        // Remove the test file.
        std::fs::remove_file(&path).unwrap();
    }


    #[test]
    fn test_check_and_persist_replay() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        let test_dir = std::env::temp_dir().join(format!(
            "replay_check_{}_{}",
            std::process::id(),
            timestamp
        ));

        std::fs::create_dir(&test_dir).unwrap();

        let state_path = test_dir.join("command_replay.bin");
        let lock_path = test_dir.join("command_replay.lock");

        // Initialize a replay window and save it.
        let window = ReplayWindow::new();
        window.save_to_file(&state_path).unwrap();

        // Counter 100 should be accepted.
        assert!(
            check_and_persist_replay(&state_path, &lock_path, 100).unwrap()
        );

        // Counter 100 should now be rejected as a duplicate.
        assert!(
            !check_and_persist_replay(&state_path, &lock_path, 100).unwrap()
        );

        // Counter 102 should be accepted.
        assert!(
            check_and_persist_replay(&state_path, &lock_path, 102).unwrap()
        );

        // Counter 101 should be accepted out of order.
        assert!(
            check_and_persist_replay(&state_path, &lock_path, 101).unwrap()
        );

        // Counter 101 should now be rejected as a duplicate.
        assert!(
            !check_and_persist_replay(&state_path, &lock_path, 101).unwrap()
        );

        // Load the state again to simulate a program restart.
        let mut restored = ReplayWindow::load_from_file(&state_path).unwrap();

        // Previously accepted counters must remain rejected.
        assert!(!restored.check_and_update(100));
        assert!(!restored.check_and_update(101));
        assert!(!restored.check_and_update(102));

        std::fs::remove_dir_all(&test_dir).unwrap();

    }

    #[test]
    fn test_interrupted_replay_state_write() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        let test_dir = std::env::temp_dir().join(format!(
            "replay_interrupted_{}_{}",
            std::process::id(),
            timestamp
        ));

        std::fs::create_dir(&test_dir).unwrap();

        let state_path = test_dir.join("command_replay.bin");
        let temp_path = state_path.with_extension("tmp");
        let lock_path = test_dir.join("command_replay.lock");

        // Create and save an initial replay window.
        let mut window = ReplayWindow::new();

        assert!(window.check_and_update(100));
        window.save_to_file(&state_path).unwrap();

        // Simulate an interrupted write by leaving a .tmp file.
        std::fs::write(&temp_path, [1u8, 2, 3, 4, 5]).unwrap();

        // Attempt to accept and persist counter 101.
        let result = check_and_persist_replay(
            &state_path,
            &lock_path,
            101,
        );

        // The operation must fail.
        assert!(result.is_err());

        // Verify that the original state was not overwritten.
        let mut restored = ReplayWindow::load_from_file(&state_path).unwrap();

        assert!(!restored.check_and_update(100));

        // Counter 101 was not persisted.
        assert!(restored.check_and_update(101));

        // Clean up.
        std::fs::remove_dir_all(&test_dir).unwrap();
    }



    #[test]
    fn test_concurrent_replay_checks() {
        use std::sync::{Arc, Barrier};
        use std::thread;
        use std::time::{SystemTime, UNIX_EPOCH};

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        let test_dir = std::env::temp_dir().join(format!(
            "replay_concurrent_{}_{}",
            std::process::id(),
            timestamp
        ));

        std::fs::create_dir(&test_dir).unwrap();

        let state_path = test_dir.join("command_replay.bin");
        let lock_path = test_dir.join("command_replay.lock");

        // Initialize the replay state.
        ReplayWindow::new()
            .save_to_file(&state_path)
            .unwrap();

        // Synchronize the start of both threads.
        let barrier = Arc::new(Barrier::new(2));

        let mut handles = Vec::new();

        for _ in 0..2 {
            let state = state_path.clone();
            let lock = lock_path.clone();
            let start = Arc::clone(&barrier);

            handles.push(thread::spawn(move || {
                start.wait();

                check_and_persist_replay(&state, &lock, 100).unwrap()
            }));
        }

        let results: Vec<bool> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();

        // Exactly one thread should accept counter 100.
        assert_eq!(results.iter().filter(|&&accepted| accepted).count(), 1);

        // The accepted counter must also be saved.
        let mut restored = ReplayWindow::load_from_file(&state_path).unwrap();

        assert!(!restored.check_and_update(100));

        std::fs::remove_dir_all(&test_dir).unwrap();
    }



        #[test]
    fn test_initialize_replay_state() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        let test_dir = std::env::temp_dir().join(format!(
            "replay_init_{}_{}",
            std::process::id(),
            timestamp
        ));

        std::fs::create_dir(&test_dir).unwrap();

        let state_path = test_dir.join("command_replay.bin");
        let lock_path = test_dir.join("command_replay.lock");
        let temp_path = state_path.with_extension("tmp");

        // Test 1: First-time initialization should succeed.
        assert!(
            initialize_replay_state(&state_path, &lock_path).is_ok()
        );

        // Verify the initialized state is empty.
        let mut window = ReplayWindow::load_from_file(&state_path).unwrap();

        assert!(window.check_and_update(100));

        // Test 2: Initialization must not overwrite existing state.
        assert!(
            initialize_replay_state(&state_path, &lock_path).is_err()
        );

        // Test 3: A leftover temporary file must prevent initialization.
        std::fs::remove_file(&state_path).unwrap();

        std::fs::write(&temp_path, [1u8, 2, 3, 4, 5]).unwrap();

        assert!(
            initialize_replay_state(&state_path, &lock_path).is_err()
        );

        // Clean up test files.
        std::fs::remove_dir_all(&test_dir).unwrap();
    }

}







