use std::path::Path;
use secure_link::initialize_replay_state;

fn main() -> std::io::Result<()> {
    let state_path = Path::new("replay_state/command_replay.bin");
    let lock_path = Path::new("replay_state/command_replay.lock");

    initialize_replay_state(state_path, lock_path)?;

    println!("Replay state initialized successfully!");

    Ok(())
}