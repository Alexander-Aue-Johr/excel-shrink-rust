//! Versioned archive-study configuration for the web entry point.

use super::Receipt;
use anyhow::{ensure, Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::BTreeMap, fs, net::TcpListener, path::PathBuf};

#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Configuration {
    pub version: u32,
    pub name: String,
    pub archive: PathBuf,
    pub output_root: PathBuf,
    pub comparison_groups: usize,
    pub prepare_processes: usize,
    pub conversion_processes: usize,
    pub shrink_threads: usize,
    pub port: u16,
    pub partitions: u32,
    /// Zero selects the entire corpus; other values are one-based size partitions.
    pub partition: u32,
    pub check_performance: bool,
    pub retain_equal_images: bool,
    pub geometry_batch_ranges: usize,
    pub capture_batch_ranges: usize,
    pub capture_batch_megapixels: usize,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            version: 1,
            name: "Corpus".into(),
            archive: PathBuf::new(),
            output_root: PathBuf::new(),
            comparison_groups: 1,
            prepare_processes: 1,
            conversion_processes: 1,
            shrink_threads: 1,
            port: 18800,
            partitions: 1,
            partition: 0,
            check_performance: false,
            retain_equal_images: false,
            geometry_batch_ranges: 256,
            capture_batch_ranges: 8,
            capture_batch_megapixels: 256,
        }
    }
}

impl Configuration {
    /// Resolve portable configuration paths against the configuration file.
    pub(crate) fn from_file(path: &std::path::Path) -> Result<Self> {
        let path = fs::canonicalize(path)?;
        let mut configuration: Self = serde_json::from_slice(&fs::read(&path)?)?;
        let parent = path.parent().context("Study configuration parent")?;
        for field in [&mut configuration.archive, &mut configuration.output_root] {
            if field.is_relative() && !field.as_os_str().is_empty() {
                *field = parent.join(&*field);
            }
        }
        Ok(configuration)
    }

    /// Validate user settings before creating any run files or starting a process.
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1,
            "Unsupported corpus configuration version"
        );
        ensure!(
            !self.name.trim().is_empty() && !self.name.chars().any(char::is_control),
            "Corpus name must be nonempty and contain no control characters"
        );
        ensure!(
            self.archive.is_absolute() && self.archive.is_file(),
            "Choose an existing absolute archive path"
        );
        ensure!(
            self.output_root.is_absolute() && !self.output_root.exists(),
            "A new study requires a new absolute output directory; use Resume for existing results"
        );
        ensure!(
            self.output_root.parent().is_some_and(|path| path.is_dir()),
            "Choose an existing parent for the new study"
        );
        ensure!(self.port >= 1024, "Study port must be at least 1024");
        ensure!(
            (1..=1000).contains(&self.partitions) && self.partition <= self.partitions,
            "Choose all files or a partition within the partition count"
        );
        let cores = std::thread::available_parallelism().map_or(1, usize::from);
        ensure!(
            (1..=cores).contains(&self.prepare_processes)
                && (1..=cores).contains(&self.shrink_threads),
            "Shrink processes and threads must each be between 1 and {cores}"
        );
        ensure!(
            (1..=i32::MAX as usize).contains(&self.conversion_processes),
            "Conversion workers must be positive"
        );
        ensure!(
            self.comparison_groups <= i32::MAX as usize,
            "Invalid comparison group count"
        );
        crate::capture_settings::batch_size(
            &json!(self.geometry_batch_ranges),
            256,
            crate::capture_settings::MAX_GEOMETRY_RANGES,
        )?;
        crate::capture_settings::batch_size(
            &json!(self.capture_batch_ranges),
            8,
            crate::capture_settings::MAX_CAPTURE_RANGES,
        )?;
        crate::capture_settings::capture_batch_megapixels(&json!(self.capture_batch_megapixels))?;
        Ok(())
    }

    fn arguments(&self, request: &std::path::Path) -> Vec<String> {
        let mut arguments = vec![
            "regression".into(),
            "study".into(),
            "--request-file".into(),
            request.to_string_lossy().into_owned(),
            "--output-root".into(),
            self.output_root.to_string_lossy().into_owned(),
            "--download-root".into(),
            self.output_root
                .join("downloads")
                .to_string_lossy()
                .into_owned(),
            "--resident-capture".into(),
            "--targeted-comparison".into(),
            "--control-room".into(),
            "--control-room-port".into(),
            self.port.to_string(),
            "--comparison-workers".into(),
            self.comparison_groups.to_string(),
            "--order".into(),
            "size-asc".into(),
            "--startup-timeout-seconds".into(),
            "180".into(),
            "--workbook-timeout-seconds".into(),
            "900".into(),
            "--retain-equal-images".into(),
            self.retain_equal_images.to_string(),
        ];
        arguments.push(if self.check_performance {
            "--targeted-check-performance".into()
        } else {
            "--targeted-skip-check-performance".into()
        });
        if self.partition > 0 {
            arguments.extend([
                "--size-part".into(),
                format!("{}/{}", self.partition, self.partitions),
            ]);
        }
        arguments
    }
}

/// Freeze inputs, executable and launch contract for a new run. Existing runs
/// retain their exact receipt and must use the separate resume operation.
pub(crate) fn create(configuration: Configuration) -> Result<PathBuf> {
    configuration.validate()?;
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, configuration.port))
        .context("Study Observatory port is occupied; choose another port")?;
    let source_before = fs::metadata(&configuration.archive)?;
    let mut archive = zip::ZipArchive::new(fs::File::open(&configuration.archive)?)?;
    let mut workbooks = 0;
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        let name = entry.name().to_ascii_lowercase();
        if !entry.is_dir() && (name.ends_with(".xlsx") || name.ends_with(".xls")) {
            workbooks += 1;
        }
    }
    ensure!(
        workbooks > 0,
        "Archive contains no direct XLS or XLSX workbooks"
    );
    let archive_hash = crate::validation_reuse::file_sha256(&configuration.archive)?;
    let source_after = fs::metadata(&configuration.archive)?;
    ensure!(
        source_before.len() == source_after.len()
            && source_before.modified()? == source_after.modified()?,
        "Source archive changed while its launch identity was being frozen"
    );
    let request = configuration.output_root.join("request.json");
    let arguments = configuration.arguments(&request);
    crate::regression::RegressionCli::try_parse_from(
        std::iter::once("deltasieve".into()).chain(arguments.iter().skip(1).cloned()),
    )?;
    fs::create_dir(&configuration.output_root)?;
    fs::File::create(configuration.output_root.join(".run.lock"))?;
    let source_executable = std::env::current_exe()?;
    let binary_hash = crate::validation_reuse::file_sha256(&source_executable)?;
    let executable_directory = configuration
        .output_root
        .join("executables")
        .join(&binary_hash);
    fs::create_dir_all(&executable_directory)?;
    let executable = executable_directory.join("deltasieve.exe");
    fs::copy(&source_executable, &executable)?;
    ensure!(
        crate::validation_reuse::file_sha256(&executable)? == binary_hash,
        "Copied study executable checksum differs"
    );
    let capture = crate::validation::capture::capture_directory()?;
    let repository = capture
        .parent()
        .and_then(|path| path.parent())
        .context("Capture repository root")?;
    let runtime = configuration.output_root.join("runtime");
    fs::create_dir_all(runtime.join("temp"))?;
    let environment = BTreeMap::from([
        (
            "EXCEL_SHRINK_REPOSITORY_ROOT".into(),
            crate::regression::power_shell_path(repository)?,
        ),
        ("EXCEL_SHRINK_SQLITE_METADATA".into(), "1".into()),
        (
            "EXCEL_SHRINK_CAPTURE_HOST".into(),
            executable.to_string_lossy().into_owned(),
        ),
        ("EXCEL_SHRINK_SYSTEM_RESERVE_MIB".into(), "512".into()),
        ("EXCEL_SHRINK_RESIDENT_VISIBLE".into(), "0".into()),
        (
            "TEMP".into(),
            runtime.join("temp").to_string_lossy().into_owned(),
        ),
        (
            "TMP".into(),
            runtime.join("temp").to_string_lossy().into_owned(),
        ),
        (
            "EXCEL_SHRINK_LOG_DIR".into(),
            runtime.join("logs").to_string_lossy().into_owned(),
        ),
    ]);
    let controls = json!({
        "target_groups":configuration.comparison_groups, "paused":false,
        "prepare_processes":configuration.prepare_processes,
        "conversion_processes":configuration.conversion_processes,
        "shrink_threads":configuration.shrink_threads, "system_reserve_mib":512,
        "open_timeout_seconds":180, "job_timeout_seconds":900, "equal_preview_width":0,
        "geometry_batch_ranges":configuration.geometry_batch_ranges,
        "capture_batch_ranges":configuration.capture_batch_ranges,
        "capture_batch_megapixels":configuration.capture_batch_megapixels,
    });
    fs::create_dir(configuration.output_root.join("control-room"))?;
    let write = |name: &str, value: &serde_json::Value| {
        crate::regression::write_atomic_artifact(
            &configuration.output_root.join(name),
            &serde_json::to_vec_pretty(value)?,
        )
    };
    write("configuration.json", &serde_json::to_value(&configuration)?)?;
    write("control-room/control.json", &controls)?;
    write(
        "request.json",
        &json!({"corpora":[{"name":configuration.name,
        "id":archive_hash,"url":configuration.archive,"total_workbooks":workbooks}]}),
    )?;
    let receipt = Receipt {
        executable,
        binary_sha256: binary_hash,
        output_root: configuration.output_root.clone(),
        request_file: request,
        arguments,
        environment,
        control_room: format!(
            "http://observatory.deltasieve.localhost:{}/",
            configuration.port
        ),
    };
    let receipt_path = configuration.output_root.join("launch.json");
    write("launch.json", &serde_json::to_value(receipt)?)?;
    drop(listener);
    Ok(receipt_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn fixture() -> (PathBuf, Configuration) {
        let root = std::env::temp_dir().join(format!("study-config-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let archive = root.join("input.zip");
        let mut writer = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        writer
            .start_file("book.xlsx", zip::write::FileOptions::default())
            .unwrap();
        writer.write_all(b"metadata-only fixture").unwrap();
        writer.finish().unwrap();
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let configuration = Configuration {
            archive,
            output_root: root.join("study"),
            port: listener.local_addr().unwrap().port(),
            ..Configuration::default()
        };
        (root, configuration)
    }

    #[test]
    fn resolves_default_only_options_and_explicit_partitions() {
        let (root, mut configuration) = fixture();
        configuration.validate().unwrap();
        let arguments = configuration.arguments(&root.join("request.json"));
        assert!(arguments
            .iter()
            .any(|argument| argument == "--targeted-skip-check-performance"));
        assert!(!arguments.iter().any(|argument| argument == "--size-part"));
        configuration.partitions = 4;
        configuration.partition = 2;
        configuration.check_performance = true;
        let arguments = configuration.arguments(&root.join("request.json"));
        assert!(arguments
            .windows(2)
            .any(|pair| pair == ["--size-part", "2/4"]));
        assert!(arguments
            .iter()
            .any(|argument| argument == "--targeted-check-performance"));
        configuration.partition = 5;
        assert!(configuration.validate().is_err());
        configuration.partition = 0;
        configuration.capture_batch_ranges = crate::capture_settings::MAX_CAPTURE_RANGES + 1;
        assert!(configuration.validate().is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn freezes_a_new_receipt_without_starting_or_overwriting_a_run() {
        let (root, configuration) = fixture();
        let receipt_path = create(configuration.clone()).unwrap();
        let receipt: Receipt = serde_json::from_slice(&fs::read(&receipt_path).unwrap()).unwrap();
        receipt.validate().unwrap();
        assert_eq!(
            crate::validation_reuse::file_sha256(&receipt.executable).unwrap(),
            receipt.binary_sha256
        );
        assert_eq!(receipt.environment["EXCEL_SHRINK_SQLITE_METADATA"], "1");
        assert_eq!(
            receipt.environment["EXCEL_SHRINK_SYSTEM_RESERVE_MIB"],
            "512"
        );
        let original_receipt = fs::read(&receipt_path).unwrap();
        assert!(create(configuration).is_err());
        assert_eq!(fs::read(receipt_path).unwrap(), original_receipt);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn occupied_port_leaves_the_output_untouched() {
        let (root, mut configuration) = fixture();
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        configuration.port = listener.local_addr().unwrap().port();
        assert!(create(configuration.clone()).is_err());
        assert!(!configuration.output_root.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn saved_configuration_resolves_paths_from_its_own_directory() {
        let (root, mut configuration) = fixture();
        configuration.archive = PathBuf::from("input.zip");
        configuration.output_root = PathBuf::from("study");
        let path = root.join("configuration.json");
        fs::write(&path, serde_json::to_vec(&configuration).unwrap()).unwrap();
        let loaded = Configuration::from_file(&path).unwrap();
        let parent = fs::canonicalize(&root).unwrap();
        assert_eq!(loaded.archive, parent.join("input.zip"));
        assert_eq!(loaded.output_root, parent.join("study"));
        loaded.validate().unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}
