use crate::{Cli, InactiveProtectionCellPolicyArg};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const REMOVE_TEXTLESS_DEFAULT_FONT_REPRESENTATIVES: &str =
    "remove-textless-default-font-representatives";
const REMOVE_TEXTLESS_STYLE_REPRESENTATIVES: &str = "remove-textless-style-representatives";
const REMOVE_TEXTLESS_COLUMN_STYLE_REPRESENTATIVES: &str =
    "remove-textless-column-style-representatives";
const REMOVE_UNREFERENCED_WHITESPACE_CELLS: &str = "remove-unreferenced-whitespace-cells";
const KEEP_UNREFERENCED_HIDDEN_CONTENT_CELLS: &str = "keep-unreferenced-hidden-content-cells";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisualProfileConfig {
    pub remove_textless_default_font_representatives: bool,
    pub remove_textless_style_representatives: bool,
    pub remove_textless_column_style_representatives: bool,
    pub inactive_protection_cell_policy: InactiveProtectionCellPolicyArg,
    pub remove_unreferenced_whitespace_cells: bool,
    pub keep_unreferenced_hidden_content_cells: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct PredefinedVisualProfile {
    pub name: &'static str,
    pub config: VisualProfileConfig,
}

pub const PREDEFINED_VISUAL_PROFILES: [PredefinedVisualProfile; 8] = [
    PredefinedVisualProfile {
        name: "default",
        config: VisualProfileConfig {
            remove_textless_default_font_representatives: false,
            remove_textless_style_representatives: false,
            remove_textless_column_style_representatives: false,
            inactive_protection_cell_policy: InactiveProtectionCellPolicyArg::Remove,
            remove_unreferenced_whitespace_cells: false,
            keep_unreferenced_hidden_content_cells: false,
        },
    },
    PredefinedVisualProfile {
        name: "remove-basic-textless-representatives",
        config: VisualProfileConfig {
            remove_textless_default_font_representatives: true,
            remove_textless_style_representatives: true,
            remove_textless_column_style_representatives: false,
            inactive_protection_cell_policy: InactiveProtectionCellPolicyArg::Remove,
            remove_unreferenced_whitespace_cells: false,
            keep_unreferenced_hidden_content_cells: false,
        },
    },
    PredefinedVisualProfile {
        name: "remove-all-textless-representatives",
        config: VisualProfileConfig {
            remove_textless_default_font_representatives: true,
            remove_textless_style_representatives: true,
            remove_textless_column_style_representatives: true,
            inactive_protection_cell_policy: InactiveProtectionCellPolicyArg::Remove,
            remove_unreferenced_whitespace_cells: false,
            keep_unreferenced_hidden_content_cells: false,
        },
    },
    PredefinedVisualProfile {
        name: "remove-unreferenced-whitespace-cells",
        config: VisualProfileConfig {
            remove_textless_default_font_representatives: false,
            remove_textless_style_representatives: false,
            remove_textless_column_style_representatives: false,
            inactive_protection_cell_policy: InactiveProtectionCellPolicyArg::Remove,
            remove_unreferenced_whitespace_cells: true,
            keep_unreferenced_hidden_content_cells: false,
        },
    },
    PredefinedVisualProfile {
        name: "keep-unreferenced-hidden-content-cells",
        config: VisualProfileConfig {
            remove_textless_default_font_representatives: false,
            remove_textless_style_representatives: false,
            remove_textless_column_style_representatives: false,
            inactive_protection_cell_policy: InactiveProtectionCellPolicyArg::Remove,
            remove_unreferenced_whitespace_cells: false,
            keep_unreferenced_hidden_content_cells: true,
        },
    },
    PredefinedVisualProfile {
        name: "keep-inactive-protection-cells-in-data-range",
        config: VisualProfileConfig {
            remove_textless_default_font_representatives: false,
            remove_textless_style_representatives: false,
            remove_textless_column_style_representatives: false,
            inactive_protection_cell_policy: InactiveProtectionCellPolicyArg::KeepInDataRange,
            remove_unreferenced_whitespace_cells: false,
            keep_unreferenced_hidden_content_cells: false,
        },
    },
    PredefinedVisualProfile {
        name: "keep-all-inactive-protection-cells",
        config: VisualProfileConfig {
            remove_textless_default_font_representatives: false,
            remove_textless_style_representatives: false,
            remove_textless_column_style_representatives: false,
            inactive_protection_cell_policy: InactiveProtectionCellPolicyArg::KeepAll,
            remove_unreferenced_whitespace_cells: false,
            keep_unreferenced_hidden_content_cells: false,
        },
    },
    PredefinedVisualProfile {
        name: "maximum-cleaning",
        config: VisualProfileConfig {
            remove_textless_default_font_representatives: true,
            remove_textless_style_representatives: true,
            remove_textless_column_style_representatives: true,
            inactive_protection_cell_policy: InactiveProtectionCellPolicyArg::Remove,
            remove_unreferenced_whitespace_cells: true,
            keep_unreferenced_hidden_content_cells: false,
        },
    },
];

impl Default for VisualProfileConfig {
    fn default() -> Self {
        Self {
            remove_textless_default_font_representatives: false,
            remove_textless_style_representatives: false,
            remove_textless_column_style_representatives: false,
            inactive_protection_cell_policy: InactiveProtectionCellPolicyArg::Remove,
            remove_unreferenced_whitespace_cells: false,
            keep_unreferenced_hidden_content_cells: false,
        }
    }
}

pub fn predefined_visual_profiles() -> &'static [PredefinedVisualProfile] {
    &PREDEFINED_VISUAL_PROFILES
}

impl VisualProfileConfig {
    pub fn from_cli(cli: &Cli) -> Self {
        Self {
            remove_textless_default_font_representatives: cli
                .remove_textless_default_font_representatives,
            remove_textless_style_representatives: cli.remove_textless_style_representatives,
            remove_textless_column_style_representatives: cli
                .remove_textless_column_style_representatives,
            inactive_protection_cell_policy: cli.inactive_protection_cell_policy,
            remove_unreferenced_whitespace_cells: cli.remove_unreferenced_whitespace_cells,
            keep_unreferenced_hidden_content_cells: cli.keep_unreferenced_hidden_content_cells,
        }
    }

    pub fn active_flags(&self) -> Vec<&'static str> {
        let mut flags = Vec::new();
        if self.remove_textless_default_font_representatives {
            flags.push(REMOVE_TEXTLESS_DEFAULT_FONT_REPRESENTATIVES);
        }
        if self.remove_textless_style_representatives {
            flags.push(REMOVE_TEXTLESS_STYLE_REPRESENTATIVES);
        }
        if self.remove_textless_column_style_representatives {
            flags.push(REMOVE_TEXTLESS_COLUMN_STYLE_REPRESENTATIVES);
        }
        if self.remove_unreferenced_whitespace_cells {
            flags.push(REMOVE_UNREFERENCED_WHITESPACE_CELLS);
        }
        if self.keep_unreferenced_hidden_content_cells {
            flags.push(KEEP_UNREFERENCED_HIDDEN_CONTENT_CELLS);
        }
        flags
    }

    pub fn inactive_protection_cell_policy_name(&self) -> &'static str {
        inactive_protection_cell_policy_name(self.inactive_protection_cell_policy)
    }

    pub fn config_hash(&self) -> String {
        stable_hash_64_hex(self.canonical_config().as_bytes())
    }

    fn canonical_config(&self) -> String {
        format!(
            "{REMOVE_TEXTLESS_DEFAULT_FONT_REPRESENTATIVES}={}\n\
             {REMOVE_TEXTLESS_STYLE_REPRESENTATIVES}={}\n\
             {REMOVE_TEXTLESS_COLUMN_STYLE_REPRESENTATIVES}={}\n\
             inactive-protection-cell-policy={}\n\
             {REMOVE_UNREFERENCED_WHITESPACE_CELLS}={}\n\
             {KEEP_UNREFERENCED_HIDDEN_CONTENT_CELLS}={}\n",
            self.remove_textless_default_font_representatives,
            self.remove_textless_style_representatives,
            self.remove_textless_column_style_representatives,
            self.inactive_protection_cell_policy_name(),
            self.remove_unreferenced_whitespace_cells,
            self.keep_unreferenced_hidden_content_cells
        )
    }

    pub fn cli_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if self.remove_textless_default_font_representatives {
            args.push(format!("--{REMOVE_TEXTLESS_DEFAULT_FONT_REPRESENTATIVES}"));
        }
        if self.remove_textless_style_representatives {
            args.push(format!("--{REMOVE_TEXTLESS_STYLE_REPRESENTATIVES}"));
        }
        if self.remove_textless_column_style_representatives {
            args.push(format!("--{REMOVE_TEXTLESS_COLUMN_STYLE_REPRESENTATIVES}"));
        }
        match self.inactive_protection_cell_policy {
            InactiveProtectionCellPolicyArg::Remove => {}
            policy => {
                args.push("--inactive-protection-cell-policy".to_string());
                args.push(inactive_protection_cell_policy_name(policy).to_string());
            }
        }
        if self.remove_unreferenced_whitespace_cells {
            args.push(format!("--{REMOVE_UNREFERENCED_WHITESPACE_CELLS}"));
        }
        if self.keep_unreferenced_hidden_content_cells {
            args.push(format!("--{KEEP_UNREFERENCED_HIDDEN_CONTENT_CELLS}"));
        }
        args
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VisualProfile {
    Default,
    RemoveBasicTextlessRepresentatives,
    RemoveAllTextlessRepresentatives,
    RemoveUnreferencedWhitespaceCells,
    KeepUnreferencedHiddenContentCells,
    KeepInactiveProtectionCellsInDataRange,
    KeepAllInactiveProtectionCells,
    MaximumCleaning,
    Custom { config_hash: String },
}

impl VisualProfile {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::RemoveBasicTextlessRepresentatives => "remove-basic-textless-representatives",
            Self::RemoveAllTextlessRepresentatives => "remove-all-textless-representatives",
            Self::RemoveUnreferencedWhitespaceCells => "remove-unreferenced-whitespace-cells",
            Self::KeepUnreferencedHiddenContentCells => "keep-unreferenced-hidden-content-cells",
            Self::KeepInactiveProtectionCellsInDataRange => {
                "keep-inactive-protection-cells-in-data-range"
            }
            Self::KeepAllInactiveProtectionCells => "keep-all-inactive-protection-cells",
            Self::MaximumCleaning => "maximum-cleaning",
            Self::Custom { .. } => "custom",
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Custom { .. } => "custom",
            _ => "predefined",
        }
    }

    pub fn relative_path(&self) -> PathBuf {
        match self {
            Self::Custom { config_hash } => PathBuf::from("custom").join(config_hash),
            _ => PathBuf::from(self.name()),
        }
    }

    pub fn relative_path_string(&self) -> String {
        self.relative_path().to_string_lossy().replace('\\', "/")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedVisualProfile {
    pub config: VisualProfileConfig,
    pub profile: VisualProfile,
    pub config_hash: String,
}

impl ResolvedVisualProfile {
    pub fn from_config(config: VisualProfileConfig) -> Self {
        let config_hash = config.config_hash();
        let profile = visual_profile_from_config(&config);
        Self {
            config,
            profile,
            config_hash,
        }
    }

    pub fn from_cli(cli: &Cli) -> Self {
        Self::from_config(VisualProfileConfig::from_cli(cli))
    }

    pub fn profile_name(&self) -> &'static str {
        self.profile.name()
    }

    pub fn profile_kind(&self) -> &'static str {
        self.profile.kind()
    }

    pub fn profile_path(&self) -> PathBuf {
        self.profile.relative_path()
    }

    pub fn profile_path_string(&self) -> String {
        self.profile.relative_path_string()
    }

    pub fn metadata(&self, xml_hash: Option<&str>) -> VisualProfileMetadata {
        VisualProfileMetadata {
            profile: self.profile_name().to_string(),
            profile_kind: self.profile_kind().to_string(),
            custom_profile_path: matches!(self.profile, VisualProfile::Custom { .. })
                .then(|| self.profile_path_string()),
            active_flags: self
                .config
                .active_flags()
                .into_iter()
                .map(str::to_string)
                .collect(),
            inactive_protection_cell_policy: self
                .config
                .inactive_protection_cell_policy_name()
                .to_string(),
            config_hash: self.config_hash.clone(),
            xml_hash: xml_hash.map(str::to_string),
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            created_at: current_unix_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct VisualProfileMetadata {
    pub profile: String,
    pub profile_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_profile_path: Option<String>,
    pub active_flags: Vec<String>,
    pub inactive_protection_cell_policy: String,
    pub config_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xml_hash: Option<String>,
    pub tool_version: String,
    pub created_at: u128,
}

pub fn visual_profile_from_config(config: &VisualProfileConfig) -> VisualProfile {
    visual_profile_from_config_with_hash(config, &config.config_hash())
}

pub fn write_visual_profile_metadata(
    path: &Path,
    profile: &ResolvedVisualProfile,
    xml_hash: Option<&str>,
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&profile.metadata(xml_hash))?,
    )?;
    Ok(())
}

pub fn profile_artifact_root(
    repo_root: &Path,
    area: &str,
    profile: &ResolvedVisualProfile,
) -> PathBuf {
    repo_root
        .join(".tmp")
        .join(area)
        .join(profile.profile_path())
}

pub fn inactive_protection_cell_policy_name(
    policy: InactiveProtectionCellPolicyArg,
) -> &'static str {
    match policy {
        InactiveProtectionCellPolicyArg::Remove => "remove",
        InactiveProtectionCellPolicyArg::KeepInDataRange => "keep-in-data-range",
        InactiveProtectionCellPolicyArg::KeepAll => "keep-all",
    }
}

fn visual_profile_from_config_with_hash(
    config: &VisualProfileConfig,
    config_hash: &str,
) -> VisualProfile {
    use InactiveProtectionCellPolicyArg::{KeepAll, KeepInDataRange, Remove};

    let VisualProfileConfig {
        remove_textless_default_font_representatives: remove_default_font,
        remove_textless_style_representatives: remove_style,
        remove_textless_column_style_representatives: remove_column_style,
        inactive_protection_cell_policy,
        remove_unreferenced_whitespace_cells: remove_whitespace,
        keep_unreferenced_hidden_content_cells: keep_hidden,
    } = *config;

    match (
        remove_default_font,
        remove_style,
        remove_column_style,
        inactive_protection_cell_policy,
        remove_whitespace,
        keep_hidden,
    ) {
        (false, false, false, Remove, false, false) => VisualProfile::Default,
        (true, true, false, Remove, false, false) => {
            VisualProfile::RemoveBasicTextlessRepresentatives
        }
        (true, true, true, Remove, false, false) => VisualProfile::RemoveAllTextlessRepresentatives,
        (false, false, false, Remove, true, false) => {
            VisualProfile::RemoveUnreferencedWhitespaceCells
        }
        (false, false, false, Remove, false, true) => {
            VisualProfile::KeepUnreferencedHiddenContentCells
        }
        (false, false, false, KeepInDataRange, false, false) => {
            VisualProfile::KeepInactiveProtectionCellsInDataRange
        }
        (false, false, false, KeepAll, false, false) => {
            VisualProfile::KeepAllInactiveProtectionCells
        }
        (true, true, true, Remove, true, false) => VisualProfile::MaximumCleaning,
        _ => VisualProfile::Custom {
            config_hash: config_hash.to_string(),
        },
    }
}

fn stable_hash_64_hex(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn current_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before unix epoch")
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn config(
        remove_default_font: bool,
        remove_style: bool,
        remove_column_style: bool,
        inactive_protection_cell_policy: InactiveProtectionCellPolicyArg,
        remove_whitespace: bool,
        keep_hidden: bool,
    ) -> VisualProfileConfig {
        VisualProfileConfig {
            remove_textless_default_font_representatives: remove_default_font,
            remove_textless_style_representatives: remove_style,
            remove_textless_column_style_representatives: remove_column_style,
            inactive_protection_cell_policy,
            remove_unreferenced_whitespace_cells: remove_whitespace,
            keep_unreferenced_hidden_content_cells: keep_hidden,
        }
    }

    fn resolved(config: VisualProfileConfig) -> ResolvedVisualProfile {
        ResolvedVisualProfile::from_config(config)
    }

    #[test]
    fn derives_predefined_visual_profiles() {
        use InactiveProtectionCellPolicyArg::{KeepAll, KeepInDataRange, Remove};

        assert_eq!(
            visual_profile_from_config(&VisualProfileConfig::default()).name(),
            "default"
        );
        assert_eq!(
            visual_profile_from_config(&config(true, true, false, Remove, false, false)).name(),
            "remove-basic-textless-representatives"
        );
        assert_eq!(
            visual_profile_from_config(&config(true, true, true, Remove, false, false)).name(),
            "remove-all-textless-representatives"
        );
        assert_eq!(
            visual_profile_from_config(&config(false, false, false, Remove, true, false)).name(),
            "remove-unreferenced-whitespace-cells"
        );
        assert_eq!(
            visual_profile_from_config(&config(false, false, false, Remove, false, true)).name(),
            "keep-unreferenced-hidden-content-cells"
        );
        assert_eq!(
            visual_profile_from_config(&config(false, false, false, KeepInDataRange, false, false))
                .name(),
            "keep-inactive-protection-cells-in-data-range"
        );
        assert_eq!(
            visual_profile_from_config(&config(false, false, false, KeepAll, false, false)).name(),
            "keep-all-inactive-protection-cells"
        );
        assert_eq!(
            visual_profile_from_config(&config(true, true, true, Remove, true, false)).name(),
            "maximum-cleaning"
        );
    }

    #[test]
    fn custom_visual_profiles_are_not_collapsed_to_predefined_profiles() {
        use InactiveProtectionCellPolicyArg::{KeepAll, Remove};

        for custom in [
            config(false, false, true, Remove, false, false),
            config(false, false, false, KeepAll, true, false),
            config(true, true, true, Remove, false, true),
        ] {
            let profile = resolved(custom);
            assert_eq!(profile.profile_name(), "custom");
            assert_eq!(profile.profile_kind(), "custom");
            assert!(
                profile.profile_path_string().starts_with("custom/"),
                "custom profile path should live below custom/: {}",
                profile.profile_path_string()
            );
        }
    }

    #[test]
    fn custom_config_hashes_are_stable_and_define_distinct_paths() {
        use InactiveProtectionCellPolicyArg::{KeepAll, Remove};

        let first = resolved(config(false, false, true, Remove, false, false));
        let same = resolved(config(false, false, true, Remove, false, false));
        let different = resolved(config(false, false, false, KeepAll, true, false));

        assert_eq!(first.config_hash, same.config_hash);
        assert_eq!(first.profile_path_string(), same.profile_path_string());
        assert_ne!(first.config_hash, different.config_hash);
        assert_ne!(first.profile_path_string(), different.profile_path_string());
        assert_ne!(first.profile_path_string(), "default");
    }

    #[test]
    fn predefined_profiles_use_their_slug_as_path() {
        let default = resolved(VisualProfileConfig::default());
        let maximum = resolved(config(
            true,
            true,
            true,
            InactiveProtectionCellPolicyArg::Remove,
            true,
            false,
        ));

        assert_eq!(default.profile_path_string(), "default");
        assert_eq!(maximum.profile_path_string(), "maximum-cleaning");
        assert_ne!(default.profile_path_string(), maximum.profile_path_string());
    }

    #[test]
    fn metadata_records_profile_config_and_xml_hash() {
        let custom = resolved(config(
            false,
            false,
            true,
            InactiveProtectionCellPolicyArg::KeepInDataRange,
            true,
            false,
        ));
        let metadata = custom.metadata(Some("abc123"));
        let json = serde_json::to_value(metadata).unwrap();

        assert_eq!(json["profile"], "custom");
        assert_eq!(json["profile_kind"], "custom");
        assert_eq!(
            json["custom_profile_path"],
            format!("custom/{}", custom.config_hash)
        );
        assert_eq!(
            json["inactive_protection_cell_policy"],
            "keep-in-data-range"
        );
        assert_eq!(json["config_hash"], custom.config_hash);
        assert_eq!(json["xml_hash"], "abc123");
        assert!(json["active_flags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|flag| flag == "remove-textless-column-style-representatives"));
        assert!(json["active_flags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|flag| flag == "remove-unreferenced-whitespace-cells"));
    }

    #[test]
    fn predefined_visual_profiles_resolve_to_their_named_profile() {
        for predefined in predefined_visual_profiles() {
            let resolved = ResolvedVisualProfile::from_config(predefined.config);

            assert_eq!(resolved.profile_name(), predefined.name);
            assert_eq!(resolved.profile_kind(), "predefined");
        }
    }

    #[test]
    fn predefined_visual_profile_cli_args_round_trip() {
        for predefined in predefined_visual_profiles() {
            let mut args = vec![
                "excel-shrink".to_string(),
                "input.xlsx".to_string(),
                "out".to_string(),
            ];
            args.extend(predefined.config.cli_args());
            let cli = Cli::parse_from(args);
            let resolved = ResolvedVisualProfile::from_cli(&cli);

            assert_eq!(resolved.profile_name(), predefined.name);
        }
    }
}
