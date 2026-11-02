//REAL
use std::net::UdpSocket; 
use std::path::Path;
use secure_link::{encrypt_packet, decrypt_packet,NonceManager}; //outside function defined in secure_link folder 

const CMD_PORT: &str = "0.0.0.0:6901";    // UDP port used to receive commands on satellite   0.0.0.0 means the program accepts packets arriving on any of its local IPv4 interfaces.
const ACK_DEST: &str = "127.0.0.1:6900";   // UDP port used to receive commands on ground station
  
const NOOP_CMD_ID: u8 = 1; //noop id is 1
const ACK_ID: u8 = 3;  // ack id is 3


// Must match the Ruby secure link protocol.
const VERSION: u8 = 1;
const COMMAND_DIRECTION: u8 = 1; //Ground station to Sattellite
const ACK_DIRECTION: u8 = 2;
const ACK_MARKER: u8 = 0xA5;



// Temporary development key. Do not use in production.
const COMMAND_KEY: [u8; 32] = [
    0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
    0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
    0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
    0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
];



// Separate temporary AES-256 key for satellite-to-ground ACKs.
const ACK_KEY: [u8; 32] = [
    0x3C, 0xA7, 0xBD, 0x16, 0xE1, 0x45, 0xD1, 0x41, 0xCC, 0xCC, 0x28, 0xBC, 0x2C, 0x01, 0x2B, 0x18, 0x04, 0x7D, 0x7F, 0xC1, 0xC0, 0x2F, 0xE4, 0x3E, 0xEB, 0x17, 0xE6, 0x72, 0x77, 0x56, 0xA6, 0xD8
];




fn main() -> std::io::Result<()> //  return can mean the function can finish successfully or return an I/O error.
{  
    let socket = UdpSocket::bind(CMD_PORT)?; //Creates a UDP socket and binds it to the address stored in CMD_PORT.The ? operator handles errors.If binding fails, Rust immediately returns the error from main().

    println!("CubeSat command receiver started");
    println!("Listening for UDP commands on port 6901...");

    let mut buffer = [0u8; 1024]; //The result is an array containing 256 bytes, all initially set to zero.


    let mut ack_nonce_manager = NonceManager::new( [0, 0, 0, 1], Path::new("../nonce_counters/ack_counter.bin"), // calling NonceManager constuctor 
    ).expect("Failed to initialize ACK nonce manager");




    loop // infite loop equivalent to while(1)
    {
        let (bytes_received, sender) = socket.recv_from(&mut buffer)?; //This line waits for an incoming UDP packet.write the incoming bytes into  buffer.

        println!("Received {} bytes from {}", bytes_received, sender);

        let packet = &buffer[..bytes_received]; // store bytes from index 0 and stop before index bytes_received. We only want the used bytes in our packet

        if bytes_received == 0 {
            println!("Empty packet received");
            continue;
        }


         // Authenticate the expected command header and decrypt the payload.
        let plaintext = match decrypt_packet(packet, &COMMAND_KEY, &[VERSION, COMMAND_DIRECTION])
        {
            Ok(data) => {
                println!("AES-GCM decryption successful!");
                data
            }
            Err(error) => {
                println!("Encrypted command rejected: {:?}", error);
                continue;
            }
        };

        // Our current NOOP command must contain exactly 2 bytes.
        if plaintext.len() != 2
        {
            println!("Invalid decrypted command length");
            continue;
        }

        println!("Decrypted plaintext bytes: {:02X?}", plaintext);
      
        let command_id = plaintext[0]; //Extract the command id
        let sequence_number = plaintext[1]; // Extract sequence_number


        match command_id   // It is similar to a switch statement in C.
        {
            NOOP_CMD_ID => {
                println!("NOOP received with sequence number {}", sequence_number);

                // Original ACK: [packet ID, sequence number]
                let ack = [ACK_ID, sequence_number];

                // The shared library generates the nonce and authenticates this header.
                let ack_header = [ACK_MARKER, VERSION, ACK_DIRECTION];


                let ack_nonce = ack_nonce_manager.next_nonce().expect("Failed to reserve ACK nonce"); // retreive nonce 


                let ack_packet = match encrypt_packet(&ack, &ACK_KEY, &ack_header,&ack_nonce) 
                {
                    Ok(packet) => packet, // ack_packet = packet if successful
                    Err(error) => 
                    {
                        println!("ACK encryption failed: {:?}", error);
                        continue;
                    }
                };

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
