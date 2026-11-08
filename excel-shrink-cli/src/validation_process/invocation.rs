//! Bounded subprocess scheduling, durable receipts and accepted-output checks.
use super::*;
use anyhow::bail;
use std::{
    collections::VecDeque,
    fs::OpenOptions,
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

const POLL_INTERVAL: Duration = Duration::from_millis(50);
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) fn execute(
    stage: &Stage,
    plan: Vec<PlannedInvocation>,
    output_root: &Path,
    logs_root: &Path,
    parallelism: usize,
    continue_on_error: bool,
    mut observer: Option<&mut ProcessObserver<'_>>,
) -> Result<Vec<ProcessOutcome>> {
    let program = fs::canonicalize(&stage.program)?;
    let working_directory = stage
        .working_directory
        .as_ref()
        .map(fs::canonicalize)
        .transpose()?;
    let mut pending = VecDeque::from(plan);
    let mut running: Vec<RunningInvocation> = Vec::new();
    let mut outcomes = Vec::new();
    let mut stop_dispatch = false;

    let execution = (|| -> Result<()> {
        while !pending.is_empty() || !running.is_empty() {
            while !stop_dispatch && running.len() < parallelism {
                let Some(plan) = pending.pop_front() else {
                    break;
                };
                match start(
                    stage,
                    plan,
                    &program,
                    working_directory.as_deref(),
                    output_root,
                    logs_root,
                )? {
                    Started::Running(invocation) => {
                        // Transfer ownership before notifying; an observer error
                        // must still close this child through explicit cleanup.
                        running.push(invocation);
                        if let Some(observer) = observer.as_mut() {
                            let invocation = running.last().unwrap();
                            observer(&invocation.outcome, Some(invocation.child.id()))?;
                        }
                    }
                    Started::Finished(outcome) => {
                        if let Some(observer) = observer.as_mut() {
                            observer(&outcome, None)?;
                        }
                        stop_dispatch = !continue_on_error;
                        outcomes.push(outcome);
                    }
                }
            }
            for index in (0..running.len()).rev() {
                if let Some(outcome) = running[index].poll(stage, output_root)? {
                    if let Some(observer) = observer.as_mut() {
                        observer(&outcome, Some(running[index].child.id()))?;
                    }
                    if outcome.status != ProcessStatus::Succeeded && !continue_on_error {
                        stop_dispatch = true;
                    }
                    running.swap_remove(index);
                    outcomes.push(outcome);
                }
            }
            if stop_dispatch {
                for plan in pending.drain(..) {
                    let outcome = record_skipped(
                        stage,
                        plan,
                        &program,
                        working_directory.as_deref(),
                        logs_root,
                    )?;
                    if let Some(observer) = observer.as_mut() {
                        observer(&outcome, None)?;
                    }
                    outcomes.push(outcome);
                }
            }
            if !running.is_empty() {
                thread::sleep(POLL_INTERVAL);
            }
        }
        Ok(())
    })();
    if let Err(error) = execution {
        let mut unconfirmed = Vec::new();
        for invocation in &mut running {
            if invocation.finished {
                continue;
            }
            if let Err(cleanup) = invocation.interrupt() {
                if cleanup.is::<UnconfirmedTermination>() {
                    unconfirmed.push(format!("{cleanup:#}"));
                } else {
                    tracing::error!("Could not persist interrupted invocation: {cleanup:#}");
                }
            }
            if let Some(observer) = observer.as_mut() {
                if let Err(publication) = observer(&invocation.outcome, Some(invocation.child.id()))
                {
                    tracing::error!(
                        "Could not publish interrupted process status: {publication:#}"
                    );
                }
            }
        }
        if !unconfirmed.is_empty() {
            return Err(UnconfirmedTermination {
                detail: format!("{error:#}; {}", unconfirmed.join("; ")),
            }
            .into());
        }
        return Err(error);
    }
    outcomes.sort_by_key(|outcome| outcome.invocation);
    Ok(outcomes)
}

enum Started {
    Running(RunningInvocation),
    Finished(ProcessOutcome),
}

struct RunningInvocation {
    plan: PlannedInvocation,
    child: Child,
    receipt: PathBuf,
    outcome: ProcessOutcome,
    started: Instant,
    finished: bool,
}

fn start(
    stage: &Stage,
    plan: PlannedInvocation,
    program: &Path,
    working_directory: Option<&Path>,
    output_root: &Path,
    logs_root: &Path,
) -> Result<Started> {
    let mut outcome = pending_outcome(stage, &plan, program, working_directory)?;
    let directory = logs_root.join(format!("invocations/{:06}", plan.id));
    fs::create_dir_all(&directory)?;
    let receipt = directory.join("outcome.json");
    persist_outcome(&receipt, &outcome)?;
    let preparation = prepare_command(&plan, &outcome, output_root, logs_root);
    let mut command = match preparation {
        Ok(command) => command,
        Err(error) => return record_start_failure(receipt, outcome, error),
    };
    let started = Instant::now();
    outcome.status = ProcessStatus::Running;
    outcome.started_at = Some(timestamp());
    persist_outcome(&receipt, &outcome)?;
    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return record_start_failure(receipt, outcome, error.into()),
    };
    Ok(Started::Running(RunningInvocation {
        plan,
        child,
        receipt,
        outcome,
        started,
        finished: false,
    }))
}

fn prepare_command(
    plan: &PlannedInvocation,
    outcome: &ProcessOutcome,
    output_root: &Path,
    logs_root: &Path,
) -> Result<Command> {
    for target in plan.expected.values() {
        prepare_output_parent(target, output_root)?;
        ensure!(
            !target.try_exists()?,
            "Output appeared before process start: {}",
            target.display()
        );
    }
    let stdout = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(logs_root.join(&outcome.stdout_log))?;
    let stderr = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(logs_root.join(&outcome.stderr_log))?;
    let mut command = Command::new(&outcome.program);
    command
        .args(&outcome.arguments)
        .envs(&outcome.environment)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    if let Some(directory) = &outcome.working_directory {
        command.current_dir(directory);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW for console tools.
    }
    Ok(command)
}

fn prepare_output_parent(target: &Path, output_root: &Path) -> Result<()> {
    let relative = target
        .strip_prefix(output_root)
        .context("Output escaped its destination directory")?;
    let parent = relative.parent().context("Output workbook has no parent")?;
    let mut directory = output_root.to_owned();
    for component in parent.components() {
        directory.push(component.as_os_str());
        if !directory.try_exists()? {
            fs::create_dir(&directory)?;
        }
        let metadata = fs::symlink_metadata(&directory)?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "Output parent must be a real directory: {}",
            directory.display()
        );
        ensure!(
            fs::canonicalize(&directory)?.starts_with(output_root),
            "Output parent resolves outside its destination: {}",
            directory.display()
        );
    }
    Ok(())
}

fn pending_outcome(
    stage: &Stage,
    plan: &PlannedInvocation,
    program: &Path,
    working_directory: Option<&Path>,
) -> Result<ProcessOutcome> {
    let paths = Substitutions::for_invocation(plan)?;
    let arguments = stage
        .arguments
        .iter()
        .map(|argument| paths.expand(argument))
        .collect::<Result<_>>()?;
    let environment = stage
        .environment
        .iter()
        .map(|(name, value)| Ok((name.clone(), paths.expand(value)?)))
        .collect::<Result<_>>()?;
    Ok(ProcessOutcome {
        invocation: plan.id,
        stage: stage.name.clone(),
        inputs: plan.inputs.clone(),
        expected_outputs: plan.expected.keys().cloned().collect(),
        produced_outputs: Vec::new(),
        accepted_outputs: Vec::new(),
        missing_outputs: plan.expected.keys().cloned().collect(),
        status: ProcessStatus::Pending,
        program: program.to_owned(),
        arguments,
        environment,
        working_directory: working_directory.map(Path::to_owned),
        stdout_log: PathBuf::from(format!("invocations/{:06}/stdout.log", plan.id)),
        stderr_log: PathBuf::from(format!("invocations/{:06}/stderr.log", plan.id)),
        started_at: None,
        finished_at: None,
        elapsed_ms: 0,
        exit_code: None,
        exit_description: None,
        error: None,
    })
}

fn record_start_failure(
    receipt: PathBuf,
    mut outcome: ProcessOutcome,
    error: anyhow::Error,
) -> Result<Started> {
    outcome.status = ProcessStatus::Failed;
    outcome.error = Some(format!("Start failed: {error:#}"));
    outcome.finished_at = Some(timestamp());
    persist_outcome(&receipt, &outcome)?;
    Ok(Started::Finished(outcome))
}

fn record_skipped(
    stage: &Stage,
    plan: PlannedInvocation,
    program: &Path,
    working_directory: Option<&Path>,
    logs_root: &Path,
) -> Result<ProcessOutcome> {
    let mut outcome = pending_outcome(stage, &plan, program, working_directory)?;
    outcome.status = ProcessStatus::Skipped;
    outcome.error = Some("Not started after an earlier failed invocation".to_owned());
    outcome.finished_at = Some(timestamp());
    let directory = logs_root.join(format!("invocations/{:06}", plan.id));
    fs::create_dir_all(&directory)?;
    persist_outcome(&directory.join("outcome.json"), &outcome)?;
    Ok(outcome)
}

impl RunningInvocation {
    fn poll(&mut self, stage: &Stage, output_root: &Path) -> Result<Option<ProcessOutcome>> {
        if let Some(status) = self.child.try_wait().context("Observe processor exit")? {
            self.finished = true;
            self.record_exit(status);
            self.collect_outputs(stage, output_root);
            return self.publish_finished().map(Some);
        }
        if self.started.elapsed() >= Duration::from_secs(stage.timeout_seconds) {
            match terminate_owned(&mut self.child) {
                Ok(status) => {
                    self.finished = true;
                    self.record_exit(status);
                    self.outcome.status = ProcessStatus::TimedOut;
                    self.outcome.error = Some(format!(
                        "Exceeded {} seconds; the owned process was stopped",
                        stage.timeout_seconds,
                    ));
                    return self.publish_finished().map(Some);
                }
                Err(error) => {
                    self.outcome.status = ProcessStatus::TimedOut;
                    self.outcome.error =
                        Some(format!("Timeout cleanup was not confirmed: {error:#}"));
                    let error: anyhow::Error = UnconfirmedTermination {
                        detail: format!("{error:#}"),
                    }
                    .into();
                    if let Err(publication) = persist_outcome(&self.receipt, &self.outcome) {
                        return Err(
                            error.context(format!("Timeout receipt also failed: {publication:#}"))
                        );
                    }
                    return Err(error);
                }
            }
        }
        Ok(None)
    }

    fn record_exit(&mut self, status: ExitStatus) {
        self.outcome.exit_code = status.code();
        self.outcome.exit_description = Some(status.to_string());
        self.outcome.status = if status.success() {
            ProcessStatus::Succeeded
        } else {
            ProcessStatus::Failed
        };
        if !status.success() {
            self.outcome.error = Some(format!("Processor exited unsuccessfully: {status}"));
        }
    }

    fn collect_outputs(&mut self, stage: &Stage, output_root: &Path) {
        if self.outcome.status != ProcessStatus::Succeeded {
            self.describe_existing_outputs();
            return;
        }
        let result = if stage.output == OutputKind::Archive {
            self.unpack_output_archive(stage.input, output_root)
        } else {
            Ok(())
        }
        .and_then(|()| self.accept_existing_outputs(output_root));
        if let Err(error) = result {
            self.outcome.status = ProcessStatus::Failed;
            self.outcome.error = Some(format!("Output verification failed: {error:#}"));
            self.outcome.accepted_outputs.clear();
        }
    }

    fn describe_existing_outputs(&mut self) {
        self.outcome.produced_outputs = self
            .plan
            .expected
            .iter()
            .filter(|(_, path)| path.exists())
            .map(|(name, _)| name.clone())
            .collect();
        self.outcome.missing_outputs = self
            .plan
            .expected
            .iter()
            .filter(|(_, path)| !path.exists())
            .map(|(name, _)| name.clone())
            .collect();
    }

    fn accept_existing_outputs(&mut self, output_root: &Path) -> Result<()> {
        self.describe_existing_outputs();
        for (name, path) in &self.plan.expected {
            if !path.try_exists()? {
                continue;
            }
            sources::require_regular_file(path)?;
            ensure!(
                fs::canonicalize(path)?.starts_with(output_root),
                "Produced workbook resolves outside the output directory: {name}"
            );
            self.outcome.accepted_outputs.push(name.clone());
        }
        if !self.outcome.missing_outputs.is_empty() {
            self.outcome.status = ProcessStatus::Incomplete;
            self.outcome.error = Some(format!(
                "Processor exited successfully but {} expected output(s) are missing",
                self.outcome.missing_outputs.len()
            ));
        }
        Ok(())
    }

    fn unpack_output_archive(&mut self, kind: InputKind, output_root: &Path) -> Result<()> {
        sources::require_regular_file(&self.plan.output_path)
            .context("Expected output ZIP was not produced")?;
        let unpacked =
            sources::extract_archive(&self.plan.output_path, &self.plan.archive_directory)?;
        let mut transfers = Vec::new();
        for (name, target) in &self.plan.expected {
            let member = if kind == InputKind::File {
                Path::new(name)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .context("Output has no filename")?
            } else {
                name.as_str()
            };
            if let Some(source) = unpacked.files.get(member) {
                ensure!(
                    !target.try_exists()?,
                    "Output ZIP would replace an existing workbook: {name}"
                );
                prepare_output_parent(target, output_root)?;
                transfers.push((source, target));
            }
        }
        for (source, target) in transfers {
            fs::rename(source, target)
                .with_context(|| format!("Publish archived workbook {}", target.display()))?;
        }
        Ok(())
    }

    fn flush_logs(&self) -> Result<()> {
        let directory = self
            .receipt
            .parent()
            .context("Invocation receipt has no directory")?;
        for name in ["stdout.log", "stderr.log"] {
            OpenOptions::new()
                .write(true)
                .open(directory.join(name))?
                .sync_all()?;
        }
        Ok(())
    }

    fn publish_finished(&mut self) -> Result<ProcessOutcome> {
        self.outcome.finished_at = Some(timestamp());
        self.outcome.elapsed_ms =
            self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        self.flush_logs()?;
        persist_outcome(&self.receipt, &self.outcome)?;
        Ok(self.outcome.clone())
    }
}

impl RunningInvocation {
    /// Explicit cleanup lets the scheduler propagate uncertainty instead of
    /// relying on a destructor that cannot return an error to the coordinator.
    fn interrupt(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        let cleanup = terminate_owned(&mut self.child);
        self.outcome.status = ProcessStatus::Failed;
        self.outcome.finished_at = Some(timestamp());
        self.outcome.elapsed_ms =
            self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        self.outcome.error = Some(match &cleanup {
            Ok(status) => {
                self.finished = true;
                self.outcome.exit_code = status.code();
                self.outcome.exit_description = Some(status.to_string());
                format!("Scheduler interrupted this invocation; owned process stopped ({status})")
            }
            Err(error) => {
                format!("Scheduler interrupted this invocation; cleanup not confirmed: {error:#}")
            }
        });
        let publication = persist_outcome(&self.receipt, &self.outcome);
        if let Err(error) = cleanup {
            let error: anyhow::Error = UnconfirmedTermination {
                detail: format!("{error:#}"),
            }
            .into();
            return Err(match publication {
                Ok(()) => error,
                Err(publication) => {
                    error.context(format!("Interruption receipt also failed: {publication:#}"))
                }
            });
        }
        publication
    }
}

impl Drop for RunningInvocation {
    fn drop(&mut self) {
        if let Err(error) = self.interrupt() {
            tracing::error!(
                "Processor cleanup failed for {}: {error:#}",
                self.receipt.display()
            );
        }
    }
}

/// Terminate only the tree started for this invocation. The caller retains the
/// child handle throughout; no process-name or global Excel termination is used.
fn terminate_owned(child: &mut Child) -> Result<ExitStatus> {
    if let Some(status) = child.try_wait()? {
        return Ok(status);
    }
    #[cfg(unix)]
    {
        let process_group =
            i32::try_from(child.id()).context("Processor PID exceeds platform range")?;
        let result = unsafe { libc::kill(-process_group, libc::SIGKILL) };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error).context("Terminate owned processor group");
            }
        }
    }
    #[cfg(windows)]
    {
        let system = std::env::var_os("SystemRoot")
            .context("SystemRoot is unavailable for processor cleanup")?;
        let mut cleanup = Command::new(PathBuf::from(system).join("System32/taskkill.exe"))
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Start owned processor-tree cleanup")?;
        match wait_bounded(&mut cleanup) {
            Ok(status) if status.success() => {}
            Ok(status) => {
                if let Some(exited) = child.try_wait()? {
                    return Ok(exited);
                }
                bail!("taskkill did not terminate the owned processor tree ({status})");
            }
            Err(error) => {
                let _ = cleanup.kill();
                let _ = wait_bounded(&mut cleanup);
                return Err(error).context("Processor-tree cleanup exceeded its deadline");
            }
        }
    }
    #[cfg(not(any(unix, windows)))]
    child.kill().context("Terminate owned processor")?;
    wait_bounded(child)
}

fn wait_bounded(child: &mut Child) -> Result<ExitStatus> {
    let deadline = Instant::now() + STOP_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        ensure!(
            Instant::now() < deadline,
            "Process exit was not confirmed within five seconds"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn persist_outcome(path: &Path, outcome: &ProcessOutcome) -> Result<()> {
    crate::regression::write_atomic_artifact(path, &serde_json::to_vec_pretty(outcome)?)
}

fn timestamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub(super) fn validate_templates(stage: &Stage) -> Result<()> {
    let paths = Substitutions {
        input: "input".into(),
        output: "output".into(),
        output_directory: "output-directory".into(),
        output_file: (stage.input == InputKind::File).then(|| "output-file".into()),
    };
    for template in stage.arguments.iter().chain(stage.environment.values()) {
        paths.expand(template)?;
    }
    Ok(())
}

struct Substitutions {
    input: String,
    output: String,
    output_directory: String,
    output_file: Option<String>,
}

impl Substitutions {
    fn for_invocation(plan: &PlannedInvocation) -> Result<Self> {
        fn text(path: &Path) -> Result<String> {
            Ok(path
                .to_str()
                .context("Processor path is not valid UTF-8")?
                .to_owned())
        }
        Ok(Self {
            input: text(&plan.input_path)?,
            output: text(&plan.output_path)?,
            output_directory: text(&plan.output_directory)?,
            output_file: plan.output_file.as_deref().map(text).transpose()?,
        })
    }

    /// Double braces escape literal braces. Inserted paths are never reparsed,
    /// split at spaces, or interpreted as command/shell syntax.
    fn expand(&self, template: &str) -> Result<String> {
        let mut text = String::with_capacity(template.len());
        let mut characters = template.chars().peekable();
        while let Some(character) = characters.next() {
            match character {
                '{' if characters.peek() == Some(&'{') => {
                    characters.next();
                    text.push('{');
                }
                '}' if characters.peek() == Some(&'}') => {
                    characters.next();
                    text.push('}');
                }
                '{' => {
                    let mut key = String::new();
                    loop {
                        match characters.next() {
                            Some('}') => break,
                            Some(character) => key.push(character),
                            None => bail!("Unclosed processor argument placeholder"),
                        }
                    }
                    let value = match key.as_str() {
                        "input" => self.input.as_str(),
                        "output" => self.output.as_str(),
                        "output_dir" => self.output_directory.as_str(),
                        "output_file" => self
                            .output_file
                            .as_deref()
                            .context("{output_file} requires file invocation granularity")?,
                        _ => bail!("Unknown processor argument placeholder: {{{key}}}"),
                    };
                    text.push_str(value);
                }
                '}' => bail!(
                    "Unmatched closing brace in processor argument; use doubled literal braces"
                ),
                character => text.push(character),
            }
        }
        ensure!(
            !text.contains('\0'),
            "Processor argument contains a NUL byte"
        );
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_preserve_spaces_and_do_not_reinterpret_inserted_paths() {
        let paths = Substitutions {
            input: "C:\\Data files\\{output}.xlsx".into(),
            output: "C:\\Results\\sample.xlsx".into(),
            output_directory: "C:\\Results".into(),
            output_file: None,
        };
        assert_eq!(
            paths.expand("--source={input}").unwrap(),
            "--source=C:\\Data files\\{output}.xlsx"
        );
        assert_eq!(
            paths.expand("{{literal}} $() ; &").unwrap(),
            "{literal} $() ; &"
        );
        assert!(paths.expand("{unknown}").is_err());
        assert!(paths.expand("{output_file}").is_err());
        assert!(paths.expand("{input").is_err());
    }
}
