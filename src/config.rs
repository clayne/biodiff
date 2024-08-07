use std::{
    error::Error,
    fs::read_to_string,
    path::{Path, PathBuf},
};

use dirs::config_dir;
use serde::{Deserialize, Serialize};

use crate::{
    preset::PresetList,
    style::{ColumnSetting, DisplayMode, Layout, Style},
    Args,
};
use biodiff_align::rustbio::{Banded, RustBio};
use biodiff_align::{AlgorithmKind, AlignAlgorithm, AlignBackend, AlignMode};

fn config_path() -> Result<PathBuf, std::io::Error> {
    match std::env::var_os("BIODIFF_CONFIG_DIR") {
        Some(p) => Ok(PathBuf::from(p)),
        None => match config_dir() {
            Some(mut p) => {
                p.push("biodiff");
                Ok(p)
            }
            None => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Could not find configuration directory",
            )),
        },
    }
}

fn settings_file() -> Result<PathBuf, std::io::Error> {
    let mut path = config_path()?;
    path.push("config.json");
    Ok(path)
}

fn no_memory_warn_file() -> Result<PathBuf, std::io::Error> {
    let mut path = config_path()?;
    path.push("no_memory_warn");
    Ok(path)
}

/// Prior to version 1 of the config, users could use local
/// alignment, and semiglobal alignment was just done by ignoring
/// this parameter.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum AlignModeV0 {
    Local,
    Global,
    Blockwise(usize),
}

impl From<AlignModeV0> for AlignMode {
    fn from(s: AlignModeV0) -> Self {
        match s {
            // local alignment is now replaced by global alignment
            AlignModeV0::Local => AlignMode::Global,
            AlignModeV0::Global => AlignMode::Global,
            AlignModeV0::Blockwise(blocksize) => AlignMode::Blockwise(blocksize),
        }
    }
}

/// Old alignment algorithm parameters from version 0 of the config file
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AlignAlgorithmV0 {
    pub gap_open: i32,
    pub gap_extend: i32,
    pub mismatch_score: i32,
    pub match_score: i32,
    pub mode: AlignModeV0,
    pub band: Banded,
}

const DEFAULT_BLOCKSIZE_V0: usize = 8192;

impl Default for AlignAlgorithmV0 {
    fn default() -> Self {
        AlignAlgorithmV0 {
            gap_open: -5,
            gap_extend: -1,
            mismatch_score: -1,
            match_score: 1,
            mode: AlignModeV0::Blockwise(DEFAULT_BLOCKSIZE_V0),
            band: Banded::Normal,
        }
    }
}

impl From<AlignAlgorithmV0> for AlignAlgorithm {
    fn from(s: AlignAlgorithmV0) -> Self {
        AlignAlgorithm {
            name: "custom".to_string(),
            gap_open: s.gap_open,
            gap_extend: s.gap_extend,
            mismatch_score: s.mismatch_score,
            match_score: s.match_score,
            mode: s.mode.into(),
            backend: AlignBackend::RustBio(RustBio { band: s.band }),
        }
    }
}

#[derive(Clone, Copy, Default, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct StyleV0 {
    pub mode: DisplayMode,
    pub ascii_col: bool,
    pub bars_col: bool,
    pub vertical: bool,
    pub spacer: bool,
    pub right_to_left: bool,
    pub column_count: ColumnSetting,
    pub no_scroll: bool,
    #[serde(skip)]
    pub addr_width: u8,
}

impl From<StyleV0> for Style {
    fn from(s: StyleV0) -> Self {
        Style {
            mode: s.mode,
            ascii_col: s.ascii_col,
            bars_col: s.bars_col,
            layout: Layout::vertical(s.vertical),
            spacer: s.spacer,
            right_to_left: s.right_to_left,
            column_count: s.column_count,
            no_scroll: s.no_scroll,
            addr_width: s.addr_width,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfigV0 {
    pub algo: AlignAlgorithmV0,
    pub style: StyleV0,
    #[serde(skip)]
    pub save_path: Option<PathBuf>,
}

impl From<ConfigV0> for ConfigV1 {
    fn from(s: ConfigV0) -> Self {
        let mut presets = PresetList::default();
        presets.global.insert(0, s.algo.into());
        let mut semiglobal: AlignAlgorithm = s.algo.into();
        semiglobal.mode = AlignMode::Semiglobal;
        presets.semiglobal.insert(0, semiglobal);
        ConfigV1 {
            presets,
            style: s.style.into(),
            no_memory_warn: false,
            save_path: s.save_path,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ConfigV1 {
    pub presets: PresetList,
    pub style: Style,
    #[serde(skip)]
    pub no_memory_warn: bool,
    #[serde(skip)]
    pub save_path: Option<PathBuf>,
}

impl ConfigV1 {
    pub fn load_memory_warn_status(&mut self) {
        self.no_memory_warn = no_memory_warn_file().map(|p| p.exists()).unwrap_or(false);
    }
    pub fn set_no_memory_warn(&mut self) {
        self.no_memory_warn = true;
        // attempt to remember the choice for next start
        no_memory_warn_file()
            .and_then(|p| std::fs::write(p, b""))
            .ok();
    }
}

impl From<ConfigV1> for Config {
    fn from(s: ConfigV1) -> Self {
        Config::Versioned(VersionedConfig::V1(s))
    }
}

pub type Settings = ConfigV1;

// right now, the only version with a version tag is version 1
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum VersionedConfig {
    #[serde(rename = "1")]
    V1(ConfigV1),
}

// the version "0" of the config file did not have a version tag
// so we try to parse it as the old format first
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Config {
    V0(ConfigV0),
    Versioned(VersionedConfig),
}

impl Config {
    pub fn from_config(path: Option<&Path>) -> Result<Option<Self>, Box<dyn Error + 'static>> {
        fn filter_file_not_found<T>(
            r: Result<T, std::io::Error>,
        ) -> Result<Option<T>, Box<dyn Error + 'static>> {
            match r {
                Ok(t) => Ok(Some(t)),
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        Ok(None)
                    } else {
                        Err(e.into())
                    }
                }
            }
        }
        // if an explicit file path is given, we want to give an error
        // if it doesn't actually exist instead of defaulting
        if let Some(config_file) = path {
            if !config_file.exists() {
                return Err(format!("Config file {} not found", config_file.display()).into());
            }
        }
        let Some(config_file) = (if let Some(p) = path {
            Some(p.to_path_buf())
        } else {
            filter_file_not_found(settings_file())?
        }) else {
            return Ok(None);
        };
        let Some(config) = filter_file_not_found(read_to_string(&config_file))? else {
            return Ok(None);
        };
        let mut settings = serde_json::from_str(&config)?;
        match &mut settings {
            Some(Config::V0(t)) => t.save_path = Some(config_file),
            Some(Config::Versioned(VersionedConfig::V1(t))) => t.save_path = Some(config_file),
            None => {}
        }
        Ok(settings)
    }

    pub fn save_config(&self) -> Result<(), Box<dyn Error + 'static>> {
        let config = serde_json::to_string_pretty(self)?;
        let save_path = match self {
            Config::V0(s) => &s.save_path,
            Config::Versioned(VersionedConfig::V1(s)) => &s.save_path,
        };
        let (dir, path) = if let Some(save_path) = save_path {
            (
                save_path
                    .parent()
                    .ok_or("No parent directory")?
                    .to_path_buf(),
                save_path.clone(),
            )
        } else {
            (config_path()?, settings_file()?)
        };
        let r = std::fs::create_dir_all(dir);
        if let Err(ref e) = r {
            match e.kind() {
                std::io::ErrorKind::AlreadyExists => (),
                _ => r?,
            }
        }
        std::fs::write(path, config)?;
        Ok(())
    }

    pub fn into_current_version(self) -> ConfigV1 {
        match self {
            Config::V0(s) => s.into(),
            Config::Versioned(VersionedConfig::V1(s)) => s,
        }
    }
}

pub fn get_settings(args: &Args) -> Result<Settings, String> {
    let mut settings = Config::from_config(args.config.as_deref())
        .map_err(|e| e.to_string())?
        .map(Config::into_current_version)
        .unwrap_or_default();
    if let Some(global) = &args.global_preset {
        let cursor = settings.presets.find(AlgorithmKind::Global, global);
        if cursor.preset.is_none() {
            return Err(format!("No global preset named {}", global));
        }
        settings.presets.select(cursor);
    }
    if let Some(semiglobal) = &args.semiglobal_preset {
        let cursor = settings.presets.find(AlgorithmKind::Semiglobal, semiglobal);
        if cursor.preset.is_none() {
            return Err(format!("No semiglobal preset named {}", semiglobal));
        }
        settings.presets.select(cursor);
    }
    if let Some(cols) = args.columns {
        if cols == 0 {
            return Err("Number of columns must be greater than 0".to_string());
        }
        settings.style.column_count = ColumnSetting::Fixed(cols);
    }
    settings.load_memory_warn_status();
    Ok(settings)
}
