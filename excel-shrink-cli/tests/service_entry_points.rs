//! Both application names must dispatch the services their native children use.

use std::process::Command;

#[test]
fn application_names_share_native_service_commands() {
    for binary in [
        env!("CARGO_BIN_EXE_excel-shrink-cli"),
        env!("CARGO_BIN_EXE_deltasieve"),
    ] {
        let host = Command::new(binary).arg("capture-host").output().unwrap();
        assert!(!host.status.success());
        assert!(
            String::from_utf8_lossy(&host.stderr)
                .contains("Expected capture-host resident, control-room, or classic"),
            "Wrong service dispatcher in {binary}: {}",
            String::from_utf8_lossy(&host.stderr)
        );
        let metadata = Command::new(binary)
            .args(["metadata-store", "--help"])
            .output()
            .unwrap();
        assert!(metadata.status.success(), "Metadata help failed in {binary}");
        assert!(String::from_utf8_lossy(&metadata.stdout).contains("verify"));
    }
}
