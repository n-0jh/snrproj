// ADCS telemetry generator
// Simulates a single-axis slew of the suspended CubeSat using one reaction wheel,
// logs every frame to a CSV, and sends them to COSMOS over UDP.
// Values are typical for a 1U CubeSat until we pick real hardware.

use rand::Rng;                          // lets us pass the random number generator around
use rand_distr::{Distribution, Normal}; // bell-curve (Gaussian) random numbers
use std::f64::consts::PI;               // pi, for converting RPM <-> rad/s
use std::fs::File;                      // creating files
use std::io::{BufWriter, Write};        // fast, buffered writing to files
use std::net::UdpSocket;                // sending UDP packets
use std::thread::sleep;                 // pausing the program
use std::time::Duration;                // how long to pause

// ---------- Physical constants (typical 1U CubeSat) ----------
const I_BODY: f64 = 0.002;           // how hard the body is to spin, kg*m^2
const I_WHEEL: f64 = 2.0e-6;         // how hard the wheel is to spin, kg*m^2
const WHEEL_MAX_RPM: f64 = 6000.0;   // wheel top speed
const WHEEL_MAX_TORQUE: f64 = 0.001; // strongest push the motor can give, N*m
const MOTOR_KT: f64 = 0.002;         // motor torque per amp of current, N*m/A
const WHEEL_IDLE_MA: f64 = 30.0;     // electronics draw with the wheel stopped, mA
const WHEEL_MA_PER_RPM: f64 = 0.02;  // extra draw from friction as the wheel spins faster

// ---------- Sensor noise ----------
const GYRO_BIAS_DPS: f64 = 0.05;  // constant gyro offset, deg/s (real gyros drift)
const GYRO_NOISE_DPS: f64 = 0.02; // random gyro jitter, deg/s
const ACCEL_NOISE_MG: f64 = 5.0;  // random accelerometer jitter, milli-g

// ---------- Controller ----------
const KP: f64 = 4.5e-5; // how hard to push toward the target angle
const KD: f64 = 5.4e-4; // how hard to resist spinning (damping)

// ---------- Modes ----------
const MODE_IDLE: u8 = 0;     // wheel off, satellite drifts
const MODE_DETUMBLE: u8 = 1; // stop the spinning
const MODE_POINT: u8 = 2;    // turn to the target and hold

// ---------- Scenario ----------
const PHYS_DT: f64 = 0.05;       // physics step, seconds
const STEPS_PER_FRAME: u32 = 10; // 10 x 0.05 s = one frame every 0.5 s
const DURATION_S: f64 = 120.0;   // total simulated time, seconds
const START_RATE_DPS: f64 = 2.0; // initial slow tumble, deg/s
const DETUMBLE_AT_S: f64 = 10.0; // when to start detumbling
const POINT_AT_S: f64 = 40.0;    // when to start pointing
const TARGET_DEG: f64 = 90.0;    // where we want to point, degrees

// ---------- Downlink ----------
const DEST: &str = "127.0.0.1:6900"; // COSMOS IP and port (127.0.0.1 = this computer)
const PKT_ID_ADCS: u8 = 3;           // ID that tells COSMOS "this is an ADCS packet" (must match tlm.txt)

/// One ADCS frame, already in the units that go in the packet.
struct AdcsFrame {
    t_ms: u32,         // simulated time, milliseconds
    gyro: [i16; 3],    // X, Y, Z rotation rate, 0.01 deg/s
    accel: [i16; 3],   // X, Y, Z acceleration, milli-g
    angle: i16,        // current rotation, 0.01 deg
    target: i16,       // commanded rotation, 0.01 deg
    wheel_rpm: i16,    // wheel speed, RPM
    wheel_torque: i16, // motor torque, micro-N*m
    wheel_ma: u16,     // wheel current, mA
    mode: u8,          // 0 idle, 1 detumble, 2 pointing
}

// ---------- Helpers ----------

/// Wrap an angle into the range -180..180 degrees.
fn wrap_deg(a: f64) -> f64 {
    (a + 180.0).rem_euclid(360.0) - 180.0 // shift, wrap to 0..360, shift back
}

/// Convert a value to hundredths, e.g. 12.34 -> 1234.
fn to_centi(x: f64) -> i16 {
    (x * 100.0).round() as i16 // scale up and round to a whole number
}

/// Which mode the controller is in at time t (seconds).
fn mode_at(t: f64) -> u8 {
    if t < DETUMBLE_AT_S {
        MODE_IDLE // first 10 s: do nothing
    } else if t < POINT_AT_S {
        MODE_DETUMBLE // next 30 s: stop the spin
    } else {
        MODE_POINT // after that: point at the target
    }
}

/// Torque the controller wants on the body, N*m.
fn control_torque(mode: u8, angle_deg: f64, target_deg: f64, rate_meas_dps: f64) -> f64 {
    let rate = rate_meas_dps.to_radians(); // measured spin, rad/s
    match mode {
        MODE_DETUMBLE => -KD * rate, // just resist the spinning
        MODE_POINT => {
            let err = wrap_deg(target_deg - angle_deg).to_radians(); // shortest way to the target
            KP * err - KD * rate                                     // push toward target, resist spinning
        }
        _ => 0.0, // idle: do nothing
    }
}

// ---------- Simulation ----------

/// Run the whole scenario and return one frame every STEPS_PER_FRAME physics steps.
fn simulate(rng: &mut impl Rng) -> Vec<AdcsFrame> {
    let gyro_noise = Normal::new(0.0, GYRO_NOISE_DPS).unwrap();  // gyro jitter generator
    let accel_noise = Normal::new(0.0, ACCEL_NOISE_MG).unwrap(); // accel jitter generator
    let max_wheel_rate = WHEEL_MAX_RPM * 2.0 * PI / 60.0;        // wheel top speed in rad/s

    let mut angle_deg = 0.0;                         // where the body is pointing, degrees
    let mut body_rate = START_RATE_DPS.to_radians(); // body spin, rad/s
    let mut wheel_rate = 0.0;                        // wheel spin, rad/s
    let mut frames = Vec::new();                     // collected frames

    let total_steps = (DURATION_S / PHYS_DT).round() as u32; // 120 s / 0.05 s = 2400 steps
    for step in 0..total_steps {     // one pass per physics step
        let t = step as f64 * PHYS_DT; // current simulated time, seconds
        let mode = mode_at(t);         // idle, detumble or pointing

        // what the gyro reads: true spin + drift + noise
        let gyro_z = body_rate.to_degrees() + GYRO_BIAS_DPS + gyro_noise.sample(rng);

        // controller picks a body torque; the motor makes it by pushing the wheel the opposite way
        let wanted = control_torque(mode, angle_deg, TARGET_DEG, gyro_z);
        let mut motor = (-wanted).clamp(-WHEEL_MAX_TORQUE, WHEEL_MAX_TORQUE); // torque on the wheel, limited

        // saturation: a wheel already at top speed can't be pushed any faster
        if (wheel_rate >= max_wheel_rate && motor > 0.0) || (wheel_rate <= -max_wheel_rate && motor < 0.0) {
            motor = 0.0; // no more push available in that direction
        }

        // physics: the wheel speeds up one way and the body turns the other way
        wheel_rate += motor / I_WHEEL * PHYS_DT; // wheel speeds up
        body_rate -= motor / I_BODY * PHYS_DT;   // body reacts in the opposite direction
        angle_deg = wrap_deg(angle_deg + body_rate.to_degrees() * PHYS_DT); // body turns

        // save a frame every STEPS_PER_FRAME steps
        if step % STEPS_PER_FRAME == 0 {
            let wheel_rpm = wheel_rate * 60.0 / (2.0 * PI); // rad/s -> RPM
            let current = WHEEL_IDLE_MA                     // electronics
                + motor.abs() / MOTOR_KT * 1000.0           // current to make the torque, mA
                + wheel_rpm.abs() * WHEEL_MA_PER_RPM;       // friction

            frames.push(AdcsFrame {
                t_ms: (t * 1000.0).round() as u32, // seconds -> milliseconds
                gyro: [
                    to_centi(GYRO_BIAS_DPS + gyro_noise.sample(rng)), // X: not rotating, just drift + noise
                    to_centi(GYRO_BIAS_DPS + gyro_noise.sample(rng)), // Y: not rotating, just drift + noise
                    to_centi(gyro_z),                                 // Z: the axis we rotate around
                ],
                accel: [
                    accel_noise.sample(rng).round() as i16,            // X: just noise
                    accel_noise.sample(rng).round() as i16,            // Y: just noise
                    (1000.0 + accel_noise.sample(rng)).round() as i16, // Z: gravity (1 g = 1000 mg)
                ],
                angle: to_centi(angle_deg),                  // current angle
                target: to_centi(TARGET_DEG),                // commanded angle
                wheel_rpm: wheel_rpm.round() as i16,         // wheel speed
                wheel_torque: (motor * 1.0e6).round() as i16, // N*m -> micro-N*m
                wheel_ma: current.round() as u16,            // wheel current
                mode,                                        // current mode
            });
        }
    }
    frames // return all frames
}

// ---------- Logging ----------

/// Write every frame to a CSV file: a header row, then one row per frame.
fn write_log(path: &str, frames: &[AdcsFrame]) -> std::io::Result<()> {
    let mut out = BufWriter::new(File::create(path)?); // create (or overwrite) the file
    writeln!(out, "t_ms,gyro_x_cdps,gyro_y_cdps,gyro_z_cdps,accel_x_mg,accel_y_mg,accel_z_mg,angle_cdeg,target_cdeg,wheel_rpm,wheel_torque_uNm,wheel_current_mA,mode")?; // header row
    for f in frames { // one row per frame
        writeln!(out, "{},{},{},{},{},{},{},{},{},{},{},{},{}",
            f.t_ms, f.gyro[0], f.gyro[1], f.gyro[2],   // time and gyro
            f.accel[0], f.accel[1], f.accel[2],        // accelerometer
            f.angle, f.target,                         // angles
            f.wheel_rpm, f.wheel_torque, f.wheel_ma,   // wheel
            f.mode)?;                                  // mode
    }
    out.flush()?; // make sure everything is written to disk
    Ok(())        // report success
}

// ---------- Packing ----------

/// Pack one ADCS frame into the 30-byte ADCS packet, big-endian.
/// The layout must match the ADCS packet in RASCAL's tlm.txt.
fn pack_adcs(counter: u16, f: &AdcsFrame) -> [u8; 30] {
    let mut p = [0u8; 30];                                     // 30 empty bytes
    p[0] = PKT_ID_ADCS;                                        // byte 0: packet ID
    p[1..3].copy_from_slice(&counter.to_be_bytes());           // bytes 1-2: counter
    p[3..7].copy_from_slice(&f.t_ms.to_be_bytes());            // bytes 3-6: time
    p[7..9].copy_from_slice(&f.gyro[0].to_be_bytes());         // bytes 7-8: gyro X
    p[9..11].copy_from_slice(&f.gyro[1].to_be_bytes());        // bytes 9-10: gyro Y
    p[11..13].copy_from_slice(&f.gyro[2].to_be_bytes());       // bytes 11-12: gyro Z
    p[13..15].copy_from_slice(&f.accel[0].to_be_bytes());      // bytes 13-14: accel X
    p[15..17].copy_from_slice(&f.accel[1].to_be_bytes());      // bytes 15-16: accel Y
    p[17..19].copy_from_slice(&f.accel[2].to_be_bytes());      // bytes 17-18: accel Z
    p[19..21].copy_from_slice(&f.angle.to_be_bytes());         // bytes 19-20: angle
    p[21..23].copy_from_slice(&f.target.to_be_bytes());        // bytes 21-22: target angle
    p[23..25].copy_from_slice(&f.wheel_rpm.to_be_bytes());     // bytes 23-24: wheel speed
    p[25..27].copy_from_slice(&f.wheel_torque.to_be_bytes());  // bytes 25-26: wheel torque
    p[27..29].copy_from_slice(&f.wheel_ma.to_be_bytes());      // bytes 27-28: wheel current
    p[29] = f.mode;                                            // byte 29: mode
    p // return the finished packet
}

fn main() {
    let mut rng = rand::rng(); // random number generator for the noise

    let frames = simulate(&mut rng); // run the whole scenario
    write_log("adcs_log.csv", &frames).expect("could not write adcs_log.csv"); // save every frame
    println!("wrote {} frames to adcs_log.csv", frames.len());                 // confirm it

    // ---------- downlink: real time, one packet every 0.5 s ----------
    let sock = UdpSocket::bind("0.0.0.0:0").expect("could not open UDP socket"); // port 0 = let Windows pick one
    let frame_period = Duration::from_secs_f64(STEPS_PER_FRAME as f64 * PHYS_DT); // 0.5 s between packets
    let mut counter: u16 = 0;                                                     // packet counter
    println!("sending {} packets to {}", frames.len(), DEST);                     // say where they're going

    for f in &frames {                                           // one pass per frame
        let pkt = pack_adcs(counter, f);                         // turn the frame into bytes
        sock.send_to(&pkt, DEST).expect("could not send packet"); // send it to COSMOS
        println!(
            "sent #{counter} t={:.1}s mode={} angle={:.1}deg wheel={}rpm", // show what went out
            f.t_ms as f64 / 1000.0, f.mode, f.angle as f64 / 100.0, f.wheel_rpm
        );
        counter = counter.wrapping_add(1); // +1, rolls back to 0 after 65535
        sleep(frame_period);               // wait until the next frame is due
    }
}

// ---------- Tests (only run with `cargo test`) ----------
#[cfg(test)]
mod tests {
    use super::*; // use everything from the main file

    #[test]
    fn angle_wrapping() { // angles stay between -180 and 180
        assert_eq!(wrap_deg(90.0), 90.0);    // already in range
        assert_eq!(wrap_deg(190.0), -170.0); // past 180 wraps negative
        assert_eq!(wrap_deg(-190.0), 170.0); // past -180 wraps positive
    }

    #[test]
    fn controller_directions() { // controller pushes the right way
        assert_eq!(control_torque(MODE_IDLE, 0.0, 90.0, 5.0), 0.0);  // idle does nothing
        assert!(control_torque(MODE_DETUMBLE, 0.0, 0.0, 10.0) < 0.0); // spinning + => push -
        assert!(control_torque(MODE_POINT, 0.0, 90.0, 0.0) > 0.0);    // target ahead => push toward it
    }

    #[test]
    fn reaches_target() { // requirement E12: end within 10 degrees of the target
        let mut rng = rand::rng();                       // random generator for the noise
        let frames = simulate(&mut rng);                 // run the scenario
        assert_eq!(frames.len(), 240);                   // 120 s / 0.5 s = 240 frames
        let last = frames.last().unwrap();               // final frame
        let error = last.angle as f64 / 100.0 - TARGET_DEG; // how far off we ended up
        assert!(error.abs() < 10.0, "ended {error:.2} deg off target");
    }

    #[test]
    fn adcs_packet_layout() { // bytes match the tlm.txt layout exactly
        let f = AdcsFrame {
            t_ms: 500,
            gyro: [1, -1, 200],
            accel: [0, 0, 1000],
            angle: 9000,
            target: 9000,
            wheel_rpm: -333,
            wheel_torque: 19,
            wheel_ma: 45,
            mode: 2,
        };
        assert_eq!(pack_adcs(1, &f), [
            3,                      // PKT_ID
            0x00, 0x01,             // counter = 1
            0x00, 0x00, 0x01, 0xF4, // time = 500 ms
            0x00, 0x01,             // gyro X = 1
            0xFF, 0xFF,             // gyro Y = -1
            0x00, 0xC8,             // gyro Z = 200
            0x00, 0x00,             // accel X = 0
            0x00, 0x00,             // accel Y = 0
            0x03, 0xE8,             // accel Z = 1000
            0x23, 0x28,             // angle = 9000 (90.00 deg)
            0x23, 0x28,             // target = 9000
            0xFE, 0xB3,             // wheel = -333 rpm
            0x00, 0x13,             // torque = 19
            0x00, 0x2D,             // current = 45 mA
            2,                      // mode = pointing
        ]);
    }
}