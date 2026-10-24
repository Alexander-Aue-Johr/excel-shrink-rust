//! Measured startup stages, separate from workbook comparison totals and ETA.
use super::*;

pub(super) struct Startup {
    root: Option<PathBuf>,
    stage: String,
    corpus: String,
    started: u64,
    last: Option<Instant>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_progress_counts_all_entries_without_changing_workbook_selection() -> Result<()> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for name in ["notes.txt", "Book.xlsx", "Legacy.XLS"] {
            writer.start_file(name, zip::write::FileOptions::default())?;
        }
        let mut archive = ZipArchive::new(writer.finish()?)?;
        let mut counts = Vec::new();
        let members = workbook_members_with_progress(&mut archive, |done, total| {
            counts.push((done, total));
            Ok(())
        })?;
        assert_eq!(members, vec![(1, "Book.xlsx".into()), (2, "Legacy.XLS".into())]);
        assert_eq!(counts.first(), Some(&(0, 3)));
        assert_eq!(counts.last(), Some(&(3, 3)));
        assert!(counts.windows(2).all(|pair| pair[0].0 <= pair[1].0));
        Ok(())
    }

    #[test]
    fn startup_keeps_ownership_and_reports_stage_boundaries() -> Result<()> {
        let output = artifact_cache::unique_path(&std::env::temp_dir(), "startup-progress")?;
        let root = output.join("control-room");
        fs::create_dir_all(&root)?;
        control_room::write(&root.join("queue.json"), &json!({"run_id":"current", "groups":[]}))?;
        let mut startup = Startup::new(&output, true);
        startup.update("opening_archive", "Enron", 0, None)?;
        let queue = control_room::read_json(&root.join("queue.json")).unwrap();
        assert!(queue["startup"]["total"].is_null());
        startup.update("scanning_archive", "Enron", 0, Some(100))?;
        startup.update("scanning_archive", "Enron", 100, Some(100))?;
        let queue = control_room::read_json(&root.join("queue.json")).unwrap();
        assert_eq!(queue["startup"]["completed"], 100);
        assert_eq!(queue["run_id"], "current");
        startup.update("checking_catalog", "Enron", 0, Some(50))?;
        let queue = control_room::read_json(&root.join("queue.json")).unwrap();
        assert_eq!(queue["startup"]["completed"], 0);
        assert_eq!(queue["startup"]["total"], 50);
        startup.finish()?;
        let queue = control_room::read_json(&root.join("queue.json")).unwrap();
        assert!(queue.get("startup").is_none());
        assert_eq!(queue["run_id"], "current");
        fs::remove_dir_all(output)?;
        Ok(())
    }
}

impl Startup {
    pub fn new(output: &Path, enabled: bool) -> Self {
        Self { root: enabled.then(|| output.join("control-room")), stage: String::new(),
            corpus: String::new(), started: provenance::now_ms(), last: None }
    }

    pub fn update(&mut self, stage: &str, corpus: &str, completed: usize, total: Option<usize>) -> Result<()> {
        let Some(root) = &self.root else { return Ok(()) };
        let changed = self.stage != stage || self.corpus != corpus;
        if !changed && total != Some(completed)
            && self.last.is_some_and(|last| last.elapsed() < Duration::from_millis(250)) {
            return Ok(());
        }
        let now = provenance::now_ms();
        if changed { self.started = now; }
        self.stage = stage.into();
        self.corpus = corpus.into();
        self.last = Some(Instant::now());
        let path = root.join("queue.json");
        let mut queue = control_room::read_json(&path).context("missing startup queue")?;
        queue["startup"] = json!({"stage":stage,"corpus":corpus,"completed":completed,
            "total":total,"started_unix_ms":self.started,"updated_unix_ms":now});
        queue["phase"] = json!("loading archives");
        queue["updated_unix_ms"] = json!(now);
        control_room::write(&path, &queue)
    }

    pub fn finish(&mut self) -> Result<()> {
        if let Some(root) = &self.root {
            let path = root.join("queue.json");
            let mut queue = control_room::read_json(&path).context("missing startup queue")?;
            queue.as_object_mut().context("invalid startup queue")?.remove("startup");
            control_room::write(&path, &queue)?;
        }
        Ok(())
    }
}
