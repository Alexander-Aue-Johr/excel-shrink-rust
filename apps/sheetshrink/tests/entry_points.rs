//! The cleanup application retains the native services required by its workers.
use std::process::Command;

#[test]
fn cleanup_help_uses_application_name() {
    let output = Command::new(env!("CARGO_BIN_EXE_sheetshrink"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("sheetshrink"));
}

#[test]
fn workers_can_dispatch_native_services() {
    let binary = env!("CARGO_BIN_EXE_sheetshrink");
    let host = Command::new(binary).arg("capture-host").output().unwrap();
    assert!(!host.status.success());
    assert!(String::from_utf8_lossy(&host.stderr)
        .contains("Expected capture-host resident, control-room, or classic"));
    let metadata = Command::new(binary)
        .args(["metadata-store", "--help"])
        .output()
        .unwrap();
    assert!(metadata.status.success());
    assert!(String::from_utf8_lossy(&metadata.stdout).contains("verify"));
}
