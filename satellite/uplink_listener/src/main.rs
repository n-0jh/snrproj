
use std::net::UdpSocket; 

use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit,Payload},
    Aes256Gcm, Nonce,
};

use rand::RngCore;
use rand::rngs::OsRng;

const CMD_PORT: &str = "0.0.0.0:6901";    // UDP port used to receive commands on satellite   0.0.0.0 means the program accepts packets arriving on any of its local IPv4 interfaces.
const ACK_DEST: &str = "127.0.0.1:6900";   // UDP port used to receive commands on ground station
  
const NOOP_CMD_ID: u8 = 1; //noop id is 1
const ACK_ID: u8 = 3;  // ack id is 3


// Must match the Ruby secure link protocol.
const VERSION: u8 = 1;
const COMMAND_DIRECTION: u8 = 1;
const ACK_DIRECTION: u8 = 2;
const ACK_MARKER: u8 = 0xA5;

// 2-byte header + 12-byte nonce + 16-byte tag.
const HEADER_LEN: usize = 2;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
const MIN_PACKET_LEN: usize = HEADER_LEN + NONCE_LEN + TAG_LEN;


// Temporary development key. Do not use in production.
const COMMAND_KEY: [u8; 32] = [
    0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
    0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
    0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
    0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
];



// Separate temporary AES-256 key for satellite-to-ground ACKs.
const ACK_KEY: [u8; 32] = [
    0x10, 0x21, 0x32, 0x43, 0x54, 0x65, 0x76, 0x87,
    0x98, 0xa9, 0xba, 0xcb, 0xdc, 0xed, 0xfe, 0x0f,
    0x10, 0x21, 0x32, 0x43, 0x54, 0x65, 0x76, 0x87,
    0x98, 0xa9, 0xba, 0xcb, 0xdc, 0xed, 0xfe, 0x0f,
];




fn main() -> std::io::Result<()> //  return can mean the function can finish successfully or return an I/O error.
{  
    let socket = UdpSocket::bind(CMD_PORT)?; //Creates a UDP socket and binds it to the address stored in CMD_PORT.The ? operator handles errors.If binding fails, Rust immediately returns the error from main().

    let cipher = Aes256Gcm::new_from_slice(&COMMAND_KEY).expect("AES-256-GCM requires a 32-byte key"); //Initialize the AES-256-GCM cipher to receive commands 
   
    let ack_cipher = Aes256Gcm::new_from_slice(&ACK_KEY).expect("AES-256-GCM requires a 32-byte ACK key"); //Initialize ACK cipher


    println!("CubeSat command receiver started");
    println!("Listening for UDP commands on port 6901...");

    let mut buffer = [0u8; 1024]; //The result is an array containing 256 bytes, all initially set to zero.

    loop // infite loop equivalent to while(1)
    {
        let (bytes_received, sender) = socket.recv_from(&mut buffer)?; //This line waits for an incoming UDP packet.write the incoming bytes into  buffer.

        println!("Received {} bytes from {}", bytes_received, sender);

        let packet = &buffer[..bytes_received]; // store bytes from index 0 and stop before index bytes_received. We only want the used bytes in our packet

        if bytes_received == 0 {
            println!("Empty packet received");
            continue;
        }

        // Reject packets too short to contain the required fields.
        if packet.len() < MIN_PACKET_LEN {
            println!("Invalid encrypted packet: too short");
            continue;
        }


         // Extract the 2-byte header.
        let header = &packet[..HEADER_LEN];

        let version = header[0];
        let direction = header[1];

        if version != VERSION || direction != COMMAND_DIRECTION {
            println!("Unsupported version or packet direction");
            continue;
        }

        // Extract the 12-byte nonce. (index 2-14) 
        let nonce_bytes = &packet[HEADER_LEN..HEADER_LEN + NONCE_LEN];

        let nonce = Nonce::from_slice(nonce_bytes); //This takes the 12-byte slice we extracted and represents it as the nonce type expected by the aes-gcm library.

        // Remaining bytes contain ciphertext followed by the GCM tag. Extract the ciphertext and authentication tag
        let encrypted_payload = &packet[HEADER_LEN + NONCE_LEN..];

        // Authenticate the header and decrypt the command.  decrypt() method expects the ciphertext and authentication tag together.
        let plaintext = match cipher.decrypt(
            nonce,
            Payload 
            {
                msg: encrypted_payload,
                aad: header,
            },
        ) 
        {
            Ok(data) => {
                println!("AES-GCM authentication successful!");
                data // plaintext = data   (if successful) 
            }
            Err(_) => {
                println!("AES-GCM authentication FAILED! Command rejected.");
                continue;
            }
        };

        // Our current NOOP command must contain exactly 2 bytes.
        if plaintext.len() != 2 {
            println!("Invalid decrypted command length");
            continue;
        }

        println!("Decrypted plaintext bytes: {:02X?}", plaintext);

        let command_id = plaintext[0]; //Extract the command identifier
        let sequence_number = plaintext[1]; // Extract sequence_number


        match command_id   // It is similar to a switch statement in C.
        {
            NOOP_CMD_ID => {
                println!("NOOP received with sequence number {}", sequence_number);

 
                //socket.send_to(&ack, ACK_DEST)?; //Sends a UDP datagram.

                // Original ACK: [packet ID, sequence number]
                let ack = [ACK_ID, sequence_number];

                // Create a fresh 12-byte nonce.
                let mut nonce_bytes = [0u8; 12];
                OsRng.try_fill_bytes(&mut nonce_bytes).expect("Failed to generate secure ACK nonce");
                let ack_nonce = Nonce::from_slice(&nonce_bytes);

                // Authenticated outer header:
                // [ACK marker, version, satellite-to-ground direction]
                let ack_header = [ACK_MARKER, VERSION, ACK_DIRECTION];

                // Encrypt the ACK and authenticate the header.
                let encrypted_ack = match ack_cipher.encrypt(
                    &ack_nonce,
                    Payload {
                        msg: &ack, // encyrpted
                        aad: &ack_header,
                    },
                ) {
                    Ok(data) => data,  //encypted_ack = data  ( if successful)
                    Err(_) => {
                        println!("ACK encryption failed");
                        continue;
                    }
                };

                // Assemble:
                // [marker][version][direction][nonce][ciphertext][tag]
                let mut ack_packet = Vec::new();

                ack_packet.extend_from_slice(&ack_header);
                ack_packet.extend_from_slice(&ack_nonce);
                ack_packet.extend_from_slice(&encrypted_ack);

                // Send the encrypted ACK to OpenC3.
                socket.send_to(&ack_packet, ACK_DEST)?;

                println!(
                    "Encrypted ACK sent with sequence number {} ({} bytes)",
                    sequence_number,
                    ack_packet.len()
                );




                println!( "ACK sent with sequence number {}",sequence_number);
            }
            _ => {
                println!("Unknown command ID: {}", command_id);
            }
        }
    }
}
