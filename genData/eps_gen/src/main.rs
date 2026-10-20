// EPS telemetry generator
// Built for LEO

// EPS telemetry generator
// Rust port of orbitSim.py + telemetryGen.py
// Built for LEO; change the constants for other orbits.

use rand::Rng;
use rand_distr::{Distribution, Normal};
use std::fs::File;
use std::io::{BufWriter, Write};

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

// ---------- Schema (stand-in for telemetrySchema.py) ----------
struct Field {
    name: &'static str,
    nom_min: f64,
    nom_max: f64,
}

// TODO: replace these envelopes with the real ones from telemetrySchema.py
const EPS_FIELDS: [Field; 4] = [
    Field { name: "batt_voltage_mV",  nom_min: 6800.0, nom_max: 8400.0 },
    Field { name: "batt_current_mA",  nom_min: -500.0, nom_max: 800.0 },
    Field { name: "batt_temp_cC",     nom_min: 0.0,    nom_max: 4000.0 },
    Field { name: "solar_current_mA", nom_min: 0.0,    nom_max: 1200.0 },
];

/// One EPS frame, values in EPS_FIELDS column order.
type Frame = [i32; 4];

// ---------- Orbit model ----------

/// Position within the current orbit, 0.0 -> 1.0.
fn orbit_phase(t: u32) -> f64 {
    (t % ORBIT_PERIOD_S) as f64 / ORBIT_PERIOD_S as f64
}

/// True if the satellite is sunlit at t seconds since epoch.
fn in_sunlight(t: u32) -> bool {
    orbit_phase(t) >= ECLIPSE_FRACTION
}

/// Array output in mA at time t.
fn solar_current(t: u32) -> i32 {
    if !in_sunlight(t) {
        return 0;
    }
    // 0.0 at sunrise -> 1.0 at sunset
    let sun_phase = (orbit_phase(t) - ECLIPSE_FRACTION) / (1.0 - ECLIPSE_FRACTION);
    (SOLAR_PEAK_MA * (std::f64::consts::PI * sun_phase).sin()) as i32
}

/// Battery current in mA. Positive = charging, negative = discharging.
fn battery_current(t: u32) -> i32 {
    solar_current(t) - LOAD_CURRENT_MA
}

/// Advance stored charge by dt seconds. Returns new charge in mAh.
fn step_charge(charge_mah: f64, t: u32, dt: u32) -> f64 {
    let delta_mah = battery_current(t) as f64 * (dt as f64 / 3600.0);
    (charge_mah + delta_mah).clamp(0.0, BATT_CAPACITY_MAH)
}

/// Pack voltage in mV for a given stored charge.
fn battery_voltage(charge_mah: f64) -> i32 {
    let soc = charge_mah / BATT_CAPACITY_MAH;
    (V_EMPTY_MV + soc * (V_FULL_MV - V_EMPTY_MV)) as i32
}

/// Gaussian jitter scaled to the field's nominal envelope.
fn add_noise(value: i32, field: &Field, rng: &mut impl Rng) -> i32 {
    let sigma = (field.nom_max - field.nom_min) * NOISE_SIGMA_FRAC;
    let noise = Normal::new(0.0, sigma).unwrap();
    (value as f64 + noise.sample(rng)).round() as i32
}

/// n consecutive EPS frames, dt seconds apart, with battery charge
/// carried from one frame to the next.
fn orbit_sequence(n: usize, dt: u32, start_soc: f64, t0: u32, rng: &mut impl Rng) -> Vec<Frame> {
    let mut charge = BATT_CAPACITY_MAH * start_soc;
    let mut frames = Vec::with_capacity(n);

    for i in 0..n {
        let t = t0 + i as u32 * dt;

        let raw: Frame = [
            battery_voltage(charge), // batt_voltage_mV
            battery_current(t),      // batt_current_mA
            BATT_TEMP_CC,            // batt_temp_cC
            solar_current(t),        // solar_current_mA
        ];

        let mut frame: Frame = [0; 4];
        for j in 0..4 {
            frame[j] = add_noise(raw[j], &EPS_FIELDS[j], rng);
        }
        frames.push(frame);

        charge = step_charge(charge, t, dt);
    }
    frames
}

// ---------- Random healthy frames (from telemetryGen.py) ----------

/// One healthy EPS frame drawn from inside each field's envelope.
fn nominal_frame(rng: &mut impl Rng) -> Frame {
    let mut frame: Frame = [0; 4];
    for j in 0..4 {
        let f = &EPS_FIELDS[j];
        let center = (f.nom_min + f.nom_max) / 2.0;
        let spread = (f.nom_max - f.nom_min) / 6.0; // ~3 sigma spans the envelope
        let value = Normal::new(center, spread).unwrap().sample(rng);
        frame[j] = value.clamp(f.nom_min, f.nom_max).round() as i32;
    }
    frame
}

/// n healthy frames.
fn nominal_batch(n: usize, rng: &mut impl Rng) -> Vec<Frame> {
    let mut frames = Vec::with_capacity(n);
    for _ in 0..n {
        frames.push(nominal_frame(rng));
    }
    frames
}

fn write_log(path: &str, orbit: &[Frame], dt: u32, t0: u32, nominal: &[Frame]) -> std::io::Result<()> {
    let mut out = BufWriter::new(File::create(path)?);

    // header row built from the schema, so it always matches the column order
    write!(out, "kind,index,t_s")?;
    for f in EPS_FIELDS.iter() {
        write!(out, ",{}", f.name)?;
    }
    writeln!(out)?;

    // orbit frames, with simulated time
    for (i, frame) in orbit.iter().enumerate() {
        let t = t0 + i as u32 * dt;
        write!(out, "orbit,{},{}", i, t)?;
        for v in frame {
            write!(out, ",{}", v)?;
        }
        writeln!(out)?;
    }

    // nominal frames have no orbit time, so t_s is left blank
    for (i, frame) in nominal.iter().enumerate() {
        write!(out, "nominal,{},", i)?;
        for v in frame {
            write!(out, ",{}", v)?;
        }
        writeln!(out)?;
    }

    out.flush()?;
    Ok(())
}

fn main() {
    let mut rng = rand::rng();

    // format guard: column order must match the schema
    let names: Vec<&str> = EPS_FIELDS.iter().map(|f| f.name).collect();
    assert_eq!(names, ["batt_voltage_mV", "batt_current_mA", "batt_temp_cC", "solar_current_mA"]);

    // one full orbit: 90 frames x 60 s
    let dt = 60;
    let t0 = 0;
    let seq = orbit_sequence(90, dt, 0.5, t0, &mut rng);
    println!("{} frames", seq.len());
    println!("frame 0  (eclipse): {:?}", seq[0]);
    println!("frame 60 (sunlit):  {:?}", seq[60]);

    let batch = nominal_batch(3, &mut rng);
    println!("nominal: {:?}", batch);

    write_log("eps_log.csv", &seq, dt, t0, &batch).expect("could not write eps_log.csv");
    println!("wrote {} frames to eps_log.csv", seq.len() + batch.len());
}