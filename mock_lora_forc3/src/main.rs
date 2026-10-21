// very simple telemetry sender to view on OpenC3
// sends fake `HEALTH` packet over UDP (NOTE: UDP since OpenC3 already has this interface)
// tlm.txt will tell OpenC3 how to interpret what we send

// will be sent as [offset][size][field][type]
// [0][1][PKT_ID][u8] 
// PKT_ID set to 1 for now since we are only sending `HEALTH` data, add more telemetry? increase #PKT_ID's 
// [1][2][COUNTER][u16] big-endian, +1 each packet

use std::net::UdpSocket;
use std::thread::sleep;
use std::time::Duration;

// NOTE: 127.0.0.1 is local host
// 6900 is a random UDP port (UDP RANGE [1-65535])
// though 7799-7814 are used as bridges by OpenC3 (from compose.override.yaml)
const DEST: &str = "127.0.0.1:6900";
const PKT_ID: u8 = 1;

fn main() {
    // port 0 = let the OS pick a free local port to send from
    let sock = UdpSocket::bind("0.0.0.0:0").expect("could not open UDP socket");

    let mut counter: u16 = 0;

    loop {
        // 3 byte array starting values 0,0,0
        let mut packet = [0u8; 3];
        packet[0] = PKT_ID;
        let counter_bytes = counter.to_be_bytes();
        packet[1] = counter_bytes[0]; // high byte
        packet[2] = counter_bytes[1]; // low byte
        //send bytes over UDP, .expect stops the sending if it fails
        sock.send_to(&packet, DEST).expect("could not send packet");
        // verify it sent
        println!("sent counter={counter}");
        // + 1 for next packet
        counter = counter.wrapping_add(1);
        // pause for 1s, result: 1 packet per second
        sleep(Duration::from_secs(1));
    }
}