
use std::net::UdpSocket;

const CMD_PORT: &str = "0.0.0.0:6901";
const NOOP_CMD_ID: u8 = 1;

fn main() -> std::io::Result<()> {
    let socket = UdpSocket::bind(CMD_PORT)?;

    println!("CubeSat command receiver started");
    println!("Listening for UDP commands on port 6901...");

    let mut buffer = [0u8; 256];

    loop {
        let (bytes_received, sender) =
            socket.recv_from(&mut buffer)?;

        println!(
            "Received {} bytes from {}",
            bytes_received, sender
        );

        if bytes_received == 0 {
            println!("Empty packet received");
            continue;
        }

        let command_id = buffer[0];

        match command_id {
            NOOP_CMD_ID => {
                println!("NOOP command received!");
            }
            _ => {
                println!("Unknown command ID: {}", command_id);
            }
        }
    }
}
