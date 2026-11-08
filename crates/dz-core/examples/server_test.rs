//! Starts a local session on a built map and stops it after a while.
//! Arguments: map id, seconds, `client` to start the game as well.
use dz_core::*;
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    let id = std::env::args().nth(1).unwrap_or("DZWETest".into());
    let secs: u64 = std::env::args().nth(2).and_then(|v| v.parse().ok()).unwrap_or(120);
    let client = std::env::args().nth(3).as_deref() == Some("client");
    let paths = build::detect();
    let job = Job::default();
    build::prepare_mission(&paths, &id, [3072.0, 3072.0], &job)?;
    let dir = std::env::temp_dir().join(format!("dzwe_session_{id}"));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = build::launch(&paths, &id, &dir, client, "", &job)?;
    for l in job.take_log() {
        println!("{l}");
    }
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(secs) {
        if let Some(c) = s.server.try_wait()? {
            println!("server exited {c:?}");
            break;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    let _ = s.server.kill();
    if let Some(c) = &mut s.client {
        let _ = c.kill();
    }
    println!("profile {}", dir.display());
    Ok(())
}
