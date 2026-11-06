//! Permanent CLI/HTTP compatibility coverage for the shared-library extraction.
//! No Excel, PowerShell or external processor is needed for these contracts.

use std::{io::{Read, Write}, net::{TcpListener, TcpStream}, process::{Child, Command, Stdio}, thread, time::{Duration, Instant}};

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn both_binary_adapters_keep_their_distinct_command_contracts() {
    let shrink = Command::new(env!("CARGO_BIN_EXE_excel-shrink-cli")).arg("--help").output().unwrap();
    assert!(shrink.status.success());
    let help = String::from_utf8(shrink.stdout).unwrap();
    assert!(help.contains("Clean and shrink XLSX files"));
    assert!(help.contains("--force-overwrite"));
    let scope = Command::new(env!("CARGO_BIN_EXE_sheet-scope")).arg("--help").output().unwrap();
    assert!(scope.status.success());
    let help = String::from_utf8(scope.stdout).unwrap();
    for command in ["compare", "observatory", "open", "export"] { assert!(help.contains(command)); }
}

#[test]
fn observatory_starts_without_sources_and_authenticates_configuration_requests() {
    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let mut server = Server(Command::new(env!("CARGO_BIN_EXE_sheet-scope"))
        .args(["observatory", "--no-open", "--port", &port.to_string()])
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() { break; }
        assert!(server.0.try_wait().unwrap().is_none(), "Observatory exited during startup");
        assert!(Instant::now() < deadline, "Observatory did not bind");
        thread::sleep(Duration::from_millis(25));
    }
    let session = request(port, "GET", "/api/session", "", "");
    assert!(session.starts_with("HTTP/1.1 200"));
    let body = session.split("\r\n\r\n").nth(1).unwrap();
    let session: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(session["config"]["source_a"], "");
    let token = session["token"].as_str().unwrap();
    let rejected = request(port, "POST", "/api/run", "X-Sheet-Scope-Token: wrong\r\n", "{}");
    assert!(rejected.starts_with("HTTP/1.1 403"));
    let accepted = request(port, "POST", "/api/run", &format!("X-Sheet-Scope-Token: {token}\r\n"), "{}");
    // Authentication succeeds; incomplete configuration remains a validation error,
    // and no external process or workbook operation is started.
    assert!(accepted.starts_with("HTTP/1.1 400"));
    assert!(accepted.contains("Select source A"));
}

fn request(port: u16, method: &str, path: &str, headers: &str, body: &str) -> String {
    let mut connection = TcpStream::connect(("127.0.0.1", port)).unwrap();
    connection.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(connection, "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nContent-Type: application/json\r\n{headers}Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    let mut response = String::new();
    connection.read_to_string(&mut response).unwrap();
    response
}
