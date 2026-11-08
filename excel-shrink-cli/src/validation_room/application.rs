//! Application identity changes the entry view; scientific execution remains shared.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Application {
    DeltaSieve,
    SheetShrink,
}

impl Application {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::DeltaSieve => "DeltaSieve",
            Self::SheetShrink => "SheetShrink",
        }
    }

    pub(super) fn hostname(self) -> &'static str {
        match self {
            Self::DeltaSieve => "deltasieve.localhost",
            Self::SheetShrink => "sheetshrink.localhost",
        }
    }

    pub(super) fn html(self) -> String {
        let purpose = match self {
            Self::DeltaSieve => "Spreadsheet visual validation",
            Self::SheetShrink => "Spreadsheet cleanup and validation",
        };
        HTML.replace("{{APPLICATION_NAME}}", self.name())
            .replace("{{APPLICATION_HOST}}", self.hostname())
            .replace("{{APPLICATION_PURPOSE}}", purpose)
    }

    /// New SheetShrink sessions retain cleaned files and start with the built-in producer.
    pub(crate) fn configure(self, mut config: RunConfig) -> Result<RunConfig> {
        if self == Self::SheetShrink && config.steps.is_empty() && config.source_b.is_none() {
            config.steps.push(sheetshrink_stage()?);
            if matches!(config.output, validation::Output::Temporary) {
                config.output = validation::Output::Directory {
                    path: config
                        .report_dir
                        .parent()
                        .context("Run directory missing")?
                        .join("cleaned"),
                };
            }
        }
        Ok(config)
    }
}

/// Native child processes dispatch cleanup through the same application executable.
pub(super) fn sheetshrink_stage() -> Result<crate::validation_process::Stage> {
    let mut stage = validation::excel_shrink_preset(1);
    stage.name = "SheetShrink".into();
    stage.program = std::env::current_exe()?;
    stage.arguments.insert(0, "shrink".into());
    Ok(stage)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_views_have_distinct_identity_and_defaults() -> Result<()> {
        let config = RunConfig {
            report_dir: PathBuf::from("run/report"),
            ..RunConfig::default()
        };
        let delta = Application::DeltaSieve.configure(config.clone())?;
        assert!(delta.steps.is_empty());
        assert!(delta.source_b.is_none());
        let shrink = Application::SheetShrink.configure(config)?;
        assert_eq!(shrink.steps[0].name, "SheetShrink");
        assert_eq!(shrink.steps[0].arguments[0], "shrink");
        assert!(matches!(
            shrink.output,
            validation::Output::Directory { .. }
        ));
        for application in [Application::DeltaSieve, Application::SheetShrink] {
            let html = application.html();
            assert!(html.contains(application.name()));
            assert!(html.contains(application.hostname()));
            assert!(!html.contains("{{APPLICATION_"));
        }
        Ok(())
    }

    #[test]
    fn explicit_comparison_and_producer_configurations_remain_intact() -> Result<()> {
        let config = RunConfig {
            source_b: Some("existing comparison".into()),
            ..RunConfig::default()
        };
        assert!(Application::SheetShrink.configure(config)?.steps.is_empty());
        let mut config = RunConfig::default();
        let mut producer = validation::excel_shrink_preset(2);
        producer.name = "Configured processor".into();
        config.steps.push(producer);
        let configured = Application::SheetShrink.configure(config)?;
        assert_eq!(configured.steps.len(), 1);
        assert_eq!(configured.steps[0].name, "Configured processor");
        assert!(matches!(configured.output, validation::Output::Temporary));
        Ok(())
    }
}
