// EPS telemetry generator
// Rust port of orbitSim.py + telemetryGen.py
// Simulates one LEO orbit, logs every frame to a CSV, and sends them to COSMOS over UDP.
// Built for LEO; change the constants for other orbits.

use rand::Rng;                          // lets us pass the random number generator around
use rand_distr::{Distribution, Normal}; // bell-curve (Gaussian) random numbers
use std::fs::File;                      // creating files
use std::io::{BufWriter, Write};        // fast, buffered writing to files
use std::net::UdpSocket;                // sending UDP packets
use std::thread::sleep;                 // pausing the program
use std::time::Duration;                // how long to pause

// ---------- Orbit and power constants ----------
const ORBIT_PERIOD_S: u32 = 5400;      // ~90 min, typical LEO
const ECLIPSE_FRACTION: f64 = 0.35;    // ~35% of each orbit in Earth's shadow
const SOLAR_PEAK_MA: f64 = 1200.0;     // array output at best sun angle
const LOAD_CURRENT_MA: i32 = 450;      // steady housekeeping draw
const BATT_CAPACITY_MAH: f64 = 2600.0; // pack capacity
const V_EMPTY_MV: f64 = 6800.0;        // pack voltage at 0% state of charge
const V_FULL_MV: f64 = 8400.0;         // pack voltage at 100%
const BATT_TEMP_CC: i32 = 2350;        // placeholder - thermal comes later
const NOISE_SIGMA_FRAC: f64 = 0.01;    // jitter as a fraction of each field's span

// ---------- Downlink ----------
const DEST: &str = "127.0.0.1:6900"; // where packets go: COSMOS IP and port (127.0.0.1 = this computer)
const PKT_ID_EPS: u8 = 2;            // ID that tells COSMOS "this is an EPS packet" (must match tlm.txt)

// ---------- Schema (stand-in for telemetrySchema.py) ----------

/// Describes one telemetry field.
struct Field {
    name: &'static str, // field name, used for the CSV header
    nom_min: f64,       // lowest healthy value
    nom_max: f64,       // highest healthy value
}

// TODO: replace these envelopes with the real ones from telemetrySchema.py
const EPS_FIELDS: [Field; 4] = [
    Field { name: "batt_voltage_mV",  nom_min: 6800.0, nom_max: 8400.0 }, // battery voltage
    Field { name: "batt_current_mA",  nom_min: -500.0, nom_max: 800.0 },  // battery current
    Field { name: "batt_temp_cC",     nom_min: 0.0,    nom_max: 4000.0 }, // battery temperature
    Field { name: "solar_current_mA", nom_min: 0.0,    nom_max: 1200.0 }, // solar panel current
];

/// One EPS frame, values in EPS_FIELDS column order.
type Frame = [i32; 4]; // a frame is just 4 whole numbers

// ---------- Orbit model ----------

/// Position within the current orbit, 0.0 -> 1.0.
fn orbit_phase(t: u32) -> f64 {
    (t % ORBIT_PERIOD_S) as f64 / ORBIT_PERIOD_S as f64 // time into this orbit / orbit length
}

/// True if the satellite is sunlit at t seconds since epoch.
fn in_sunlight(t: u32) -> bool {
    orbit_phase(t) >= ECLIPSE_FRACTION // the first 35% of each orbit is eclipse
}

/// Array output in mA at time t.
fn solar_current(t: u32) -> i32 {
    if !in_sunlight(t) { // in Earth's shadow?
        return 0;        // then the panels make nothing
    }
    // how far through the sunlit part we are: 0.0 at sunrise -> 1.0 at sunset
    let sun_phase = (orbit_phase(t) - ECLIPSE_FRACTION) / (1.0 - ECLIPSE_FRACTION);
    // sine curve: 0 at sunrise, peak at solar noon, back to 0 at sunset
    (SOLAR_PEAK_MA * (std::f64::consts::PI * sun_phase).sin()) as i32
}

/// Battery current in mA. Positive = charging, negative = discharging.
fn battery_current(t: u32) -> i32 {
    solar_current(t) - LOAD_CURRENT_MA // what the panels make minus what the satellite uses
}

/// Advance stored charge by dt seconds. Returns new charge in mAh.
fn step_charge(charge_mah: f64, t: u32, dt: u32) -> f64 {
    let delta_mah = battery_current(t) as f64 * (dt as f64 / 3600.0); // mA x hours = mAh gained or lost
    (charge_mah + delta_mah).clamp(0.0, BATT_CAPACITY_MAH)            // keep it between empty and full
}

/// Pack voltage in mV for a given stored charge.
fn battery_voltage(charge_mah: f64) -> i32 {
    let soc = charge_mah / BATT_CAPACITY_MAH;            // state of charge: 0.0 = empty, 1.0 = full
    (V_EMPTY_MV + soc * (V_FULL_MV - V_EMPTY_MV)) as i32 // straight line from empty voltage to full voltage
}

/// Gaussian jitter scaled to the field's nominal envelope.
fn add_noise(value: i32, field: &Field, rng: &mut impl Rng) -> i32 {
    let sigma = (field.nom_max - field.nom_min) * NOISE_SIGMA_FRAC; // noise size: 1% of the field's range
    let noise = Normal::new(0.0, sigma).unwrap();                   // bell curve centered on 0
    (value as f64 + noise.sample(rng)).round() as i32               // add a random wiggle, round to a whole number
}

/// n consecutive EPS frames, dt seconds apart, with battery charge
/// carried from one frame to the next.
fn orbit_sequence(n: usize, dt: u32, start_soc: f64, t0: u32, rng: &mut impl Rng) -> Vec<Frame> {
    let mut charge = BATT_CAPACITY_MAH * start_soc; // starting battery charge in mAh
    let mut frames = Vec::with_capacity(n);         // empty list with room for n frames

    for i in 0..n {                 // one pass per frame
        let t = t0 + i as u32 * dt; // simulated time of this frame, in seconds

        let raw: Frame = [           // the exact values, before noise
            battery_voltage(charge), // batt_voltage_mV
            battery_current(t),      // batt_current_mA
            BATT_TEMP_CC,            // batt_temp_cC
            solar_current(t),        // solar_current_mA
        ];

        let mut frame: Frame = [0; 4]; // start with four zeros
        for j in 0..4 {                // for each field...
            frame[j] = add_noise(raw[j], &EPS_FIELDS[j], rng); // ...add noise to it
        }
        frames.push(frame); // save the finished frame

        charge = step_charge(charge, t, dt); // update the battery for the next frame
    }
    frames // return the list of frames
}

// ---------- Random healthy frames (from telemetryGen.py) ----------

/// One healthy EPS frame drawn from inside each field's envelope.
fn nominal_frame(rng: &mut impl Rng) -> Frame {
    let mut frame: Frame = [0; 4]; // start with four zeros
    for j in 0..4 {                // for each field...
        let f = &EPS_FIELDS[j];    // look up its healthy range
        let center = (f.nom_min + f.nom_max) / 2.0;                   // middle of the range
        let spread = (f.nom_max - f.nom_min) / 6.0;                   // ~3 sigma spans the envelope
        let value = Normal::new(center, spread).unwrap().sample(rng); // random value near the middle
        frame[j] = value.clamp(f.nom_min, f.nom_max).round() as i32;  // keep it in range, round it
    }
    frame // return the frame
}

/// n healthy frames.
fn nominal_batch(n: usize, rng: &mut impl Rng) -> Vec<Frame> {
    let mut frames = Vec::with_capacity(n); // empty list with room for n frames
    for _ in 0..n {                         // repeat n times
        frames.push(nominal_frame(rng));    // make a frame and save it
    }
    frames // return the list
}

// ---------- Logging ----------

/// Write every frame to a CSV file: a header row, then one row per frame.
fn write_log(path: &str, orbit: &[Frame], dt: u32, t0: u32, nominal: &[Frame]) -> std::io::Result<()> {
    let mut out = BufWriter::new(File::create(path)?); // create (or overwrite) the file

    // header row built from the schema, so it always matches the column order
    write!(out, "kind,index,t_s")?;  // first three column names
    for f in EPS_FIELDS.iter() {     // then one column per field
        write!(out, ",{}", f.name)?; // comma, then the field name
    }
    writeln!(out)?; // end the header line

    // orbit frames, with simulated time
    for (i, frame) in orbit.iter().enumerate() { // i = frame number
        let t = t0 + i as u32 * dt;              // simulated time of this frame
        write!(out, "orbit,{},{}", i, t)?;       // kind, index, time
        for v in frame {                         // each of the 4 values...
            write!(out, ",{}", v)?;              // ...after a comma
        }
        writeln!(out)?; // end the row
    }

    // nominal frames have no orbit time, so t_s is left blank
    for (i, frame) in nominal.iter().enumerate() { // i = frame number
        write!(out, "nominal,{},", i)?;            // kind, index, empty time
        for v in frame {                           // each of the 4 values...
            write!(out, ",{}", v)?;                // ...after a comma
        }
        writeln!(out)?; // end the row
    }

    out.flush()?; // make sure everything is actually written to disk
    Ok(())        // report success
}

// ---------- Packing ----------

/// Pack one EPS frame into the 15-byte EPS packet, big-endian.
/// The layout must match the EPS packet in RASCAL's tlm.txt.
fn pack_eps(counter: u16, sim_time: u32, frame: &Frame) -> [u8; 15] {
    let mut pkt = [0u8; 15];                                       // 15 empty bytes
    pkt[0] = PKT_ID_EPS;                                           // byte 0: packet ID
    pkt[1..3].copy_from_slice(&counter.to_be_bytes());             // bytes 1-2: counter
    pkt[3..7].copy_from_slice(&sim_time.to_be_bytes());            // bytes 3-6: simulated time
    pkt[7..9].copy_from_slice(&(frame[0] as u16).to_be_bytes());   // bytes 7-8: batt voltage
    pkt[9..11].copy_from_slice(&(frame[1] as i16).to_be_bytes());  // bytes 9-10: batt current
    pkt[11..13].copy_from_slice(&(frame[2] as i16).to_be_bytes()); // bytes 11-12: batt temp
    pkt[13..15].copy_from_slice(&(frame[3] as i16).to_be_bytes()); // bytes 13-14: solar current
    pkt // return the finished packet
}

fn main() {
    let mut rng = rand::rng(); // random number generator for the noise

    // format guard: column order must match the schema
    let names: Vec<&str> = EPS_FIELDS.iter().map(|f| f.name).collect(); // list of field names
    assert_eq!(names, ["batt_voltage_mV", "batt_current_mA", "batt_temp_cC", "solar_current_mA"]); // stop if the order changed

    // one full orbit: 90 frames x 60 s
    let dt = 60;                                          // seconds of orbit between frames
    let t0 = 0;                                           // simulated start time
    let seq = orbit_sequence(90, dt, 0.5, t0, &mut rng);  // 90 frames, battery starts half full
    let batch = nominal_batch(3, &mut rng);               // 3 random healthy frames for the log

    write_log("eps_log.csv", &seq, dt, t0, &batch).expect("could not write eps_log.csv"); // save everything
    println!("wrote {} frames to eps_log.csv", seq.len() + batch.len());                  // confirm it

    // ---------- downlink: 1 packet per real second, each = dt seconds of orbit ----------
    let sock = UdpSocket::bind("0.0.0.0:0").expect("could not open UDP socket"); // port 0 = let Windows pick one
    let mut counter: u16 = 0;                                                     // packet counter, starts at 0
    println!("sending {} packets to {}", seq.len(), DEST);                        // say where they're going

    for (i, frame) in seq.iter().enumerate() {                   // one pass per frame
        let sim_time = t0 + i as u32 * dt;                       // simulated time of this frame
        let pkt = pack_eps(counter, sim_time, frame);            // turn the frame into bytes
        sock.send_to(&pkt, DEST).expect("could not send packet"); // send it to COSMOS
        println!("sent #{counter} t={sim_time}s {:?}", frame);    // show what went out
        counter = counter.wrapping_add(1);                       // +1, rolls back to 0 after 65535
        sleep(Duration::from_secs(1));                           // wait 1 second
    }
}

// ---------- Tests (only run with `cargo test`) ----------
#[cfg(test)]
mod tests {
    use super::*; // use everything from the main file

    #[test]
    fn sunlight() { // eclipse vs sunlight timing
        assert!(!in_sunlight(0));    // eclipse
        assert!(in_sunlight(2700));  // halfway, sunlit
        assert!(!in_sunlight(5400)); // orbit two begins, eclipse again
    }

    #[test]
    fn solar() { // solar panel output across the orbit
        assert_eq!(solar_current(0), 0);       // eclipse
        assert_eq!(solar_current(2000), 117);  // climbing after sunrise
        assert_eq!(solar_current(3510), 1191); // solar noon
        assert_eq!(solar_current(5390), 10);   // sunset
    }

    #[test]
    fn battery() { // battery current and voltage
        assert_eq!(battery_current(0), -450);      // eclipse, discharging
        assert_eq!(battery_current(3645), 750);    // solar noon, charging
        assert_eq!(battery_current(2100), -226);   // array not yet covering load
        assert_eq!(battery_voltage(0.0), 6800);    // empty
        assert_eq!(battery_voltage(1300.0), 7600); // half
        assert_eq!(battery_voltage(2600.0), 8400); // full
    }

    #[test]
    fn charge_clamps() { // battery can't go past full or below empty
        assert_eq!(step_charge(2600.0, 3645, 3600), 2600.0); // can't overcharge
        assert_eq!(step_charge(0.0, 0, 3600), 0.0);          // can't go below empty
    }

    #[test]
    fn sequence_length() { // right number of frames
        let mut rng = rand::rng();                                       // random generator for the noise
        assert_eq!(orbit_sequence(90, 60, 0.5, 0, &mut rng).len(), 90); // asked for 90, got 90
    }

    #[test]
    fn eps_packet_layout() { // bytes match the tlm.txt layout exactly
        let pkt = pack_eps(1, 60, &[7600, -450, 2350, 0]); // pack a known frame
        assert_eq!(pkt, [
            2,                      // PKT_ID
            0x00, 0x01,             // counter = 1
            0x00, 0x00, 0x00, 0x3C, // sim time = 60
            0x1D, 0xB0,             // 7600 mV
            0xFE, 0x3E,             // -450 mA
            0x09, 0x2E,             // 2350 cC
            0x00, 0x00,             // 0 mA
        ]);
    }
}