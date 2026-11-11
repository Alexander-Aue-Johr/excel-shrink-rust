use std::ffi::c_void;
use std::fs::File;
use std::io::BufReader;
use std::time::Instant;

#[repr(C)]
#[derive(Default)]
struct IoCounters {
    read_operations: u64,
    write_operations: u64,
    other_operations: u64,
    read_bytes: u64,
    write_bytes: u64,
    other_bytes: u64,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> *mut c_void;
    fn GetProcessIoCounters(process: *mut c_void, counters: *mut IoCounters) -> i32;
}

fn counters() -> IoCounters {
    let mut value = IoCounters::default();
    assert_ne!(unsafe { GetProcessIoCounters(GetCurrentProcess(), &mut value) }, 0);
    value
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args().collect();
    assert_eq!(arguments.len(), 4);
    let before = counters();
    let started = Instant::now();
    let file = File::open(&arguments[2])?;
    assert!(file.metadata()?.len() <= 128 * 1024 * 1024);
    let value: serde_json::Value = if arguments[1] == "buffered" {
        serde_json::from_reader(BufReader::new(file))?
    } else {
        serde_json::from_reader(file)?
    };
    let elapsed = started.elapsed().as_secs_f64();
    let after = counters();
    std::fs::write(&arguments[3], serde_json::to_vec(&value)?)?;
    println!("{}", serde_json::json!({
        "mode": arguments[1],
        "seconds": elapsed,
        "read_operations": after.read_operations - before.read_operations,
        "read_bytes": after.read_bytes - before.read_bytes
    }));
    Ok(())
}
