use aes_gcm::{aead::{Aead, KeyInit, Payload}, Aes256Gcm, Nonce};
//use rand::{rngs::OsRng, RngCore};

use std::fs::{File, OpenOptions};
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
