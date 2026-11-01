//! Transport and ownership of the persistent Windows Excel host.
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

pub(super) struct ExcelHost {
    child: Child,
    input: BufWriter<ChildStdin>,
    output: BufReader<ChildStdout>,
}

impl ExcelHost {
    /// Starts one command host; that host owns the persistent Excel instances.
    pub(super) fn start(host_script: Option<&Path>) -> Result<Self> {
        let workspace_script =
            Path::new("excel-shrink-cli/office-capture/targeted-comparison-host.ps1");
        let package_script = Path::new("office-capture/targeted-comparison-host.ps1");
        let default_script = if workspace_script.exists() {
            workspace_script.to_path_buf()
        } else if package_script.exists() {
            package_script.to_path_buf()
        } else {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("office-capture/targeted-comparison-host.ps1")
        };
        let script = host_script.unwrap_or(&default_script);
        let mut child = Command::new("powershell.exe")
            .args(["-STA", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("start persistent Excel host; Windows PowerShell is required")?;
        let input = BufWriter::new(child.stdin.take().context("Excel host stdin")?);
        let output = BufReader::new(child.stdout.take().context("Excel host stdout")?);
        Ok(Self {
            child,
            input,
            output,
        })
    }

    /// Sends one command and checks the corresponding host response.
    pub(super) fn call(&mut self, command: Value) -> Result<Value> {
        serde_json::to_writer(&mut self.input, &command)?;
        self.input.write_all(b"\n")?;
        self.input.flush()?;
        let mut line = String::new();
        ensure!(
            self.output.read_line(&mut line)? > 0,
            "Excel host exited without a response"
        );
        let response: Value = serde_json::from_str(&line).context("invalid Excel host response")?;
        ensure!(response["ok"] == true, "Excel host: {}", response["error"]);
        Ok(response["result"].clone())
    }
}

impl Drop for ExcelHost {
    fn drop(&mut self) {
        let _ = self.call(json!({"kind":"close", "lane":"SD"}));
        let _ = self.call(json!({"kind":"close", "lane":"O"}));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
