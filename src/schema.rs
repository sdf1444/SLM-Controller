//! This module contains types that client sends to and recieves from the server.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

use flexi_logger::LevelFilter;
use serde::{
    de::{Deserializer, Error, MapAccess, Visitor},
    Deserialize, Serialize,
};

#[derive(Deserialize, Debug, Clone)]
pub struct DirPath {
    pub base_patterns: PathBuf,
    pub flatness_corr_patterns: PathBuf,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Microscope {
    pub serial_nr: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct MqttConfig {
    pub broker_ip: String,
    pub port: u16,
}

impl MqttConfig {
    pub fn server_uri(&self) -> String {
        format!("tcp://{}:{}", self.broker_ip, self.port)
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct ScreenConfig {
    pub size: (u32, u32),
    pub fullscreen: bool,
}

#[derive(Deserialize, Debug, Clone)]
pub struct SLMCalibScaling {
    #[serde(rename = "wavelength")]
    pub known_wavelengths: Vec<u32>,
    #[serde(rename = "scale_factor")]
    pub scale_factors: Vec<f32>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct PatternComputationDebug {
    #[serde(rename = "save_computed_pattern_to_image_file")]
    pub save_computed_to_image: bool,
}

#[derive(Deserialize, Debug, Clone)]
pub struct PatternComputationConfig {
    pub slm_calib_scaling: SLMCalibScaling,
    pub add_flatness_correction: bool,
    pub debug: Option<PatternComputationDebug>,
}

#[serde(rename_all = "snake_case")]
#[derive(Deserialize, Debug, Clone)]
pub enum LogLevel {
    Debug,
    Info,
    Warning,
    Error,
    Critical,
}

impl LogLevel {
    pub fn into_level_filter(&self) -> LevelFilter {
        match self {
            Self::Debug => LevelFilter::Debug,
            Self::Info => LevelFilter::Info,
            Self::Warning => LevelFilter::Warn,
            Self::Error => LevelFilter::Error,
            // Rust doesn't have a critical level
            Self::Critical => LevelFilter::Error,
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct Logging {
    pub log_level: LogLevel,
}

#[derive(Deserialize, Debug, Clone)]
pub struct DefaultState {
    pub fresnel: u32,
    pub wavelength: u32,
    pub pattern: PatternParams,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Config {
    pub microscope: Microscope,
    pub dir_path: DirPath,
    pub mqtt: MqttConfig,
    pub screen: ScreenConfig,
    pub compute_pattern: PatternComputationConfig,
    pub image_file_extensions: Vec<String>,
    pub logging: Logging,
    pub defaults: DefaultState,
}

impl Config {
    pub fn main_topic(&self) -> &str {
        &self.microscope.serial_nr
    }
}

#[serde(rename_all = "snake_case")]
#[derive(Deserialize, Serialize, Debug, Clone)]
pub enum MessageType {
    Log,
    Device,
    Status,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LaserState {
    pub name: String,
    pub state: u32, // assuming u32, could be bool?
    pub wavelength: u32,
    pub intensity: u32, // Not sure if intensity is u32 or f32
}

#[serde(tag = "command")]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum LaserCommand {
    #[serde(rename = "get")]
    Get,
    #[serde(rename = "availablePatterns")]
    AvailablePatterns,
    #[serde(rename = "set")]
    Set { lasers: Vec<LaserState> },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AimState {
    pub pattern: PatternParams,
    pub fresnel: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CorrectionPatternDeltas {
    pub wavelength: u32,
    pub imagedata: String,
    pub shape_xy: [usize; 2],
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct APatternProp {
    pub values: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct APattern {
    #[serde(flatten)]
    pub property_values: HashMap<String, APatternProp>,
    pub properties: Vec<String>,
}
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct AvailablePatterns {
    #[serde(flatten)]
    pub patterns: HashMap<String, APattern>,
    #[serde(rename = "patternNames")]
    pub pattern_names: Vec<String>,
}

#[serde(tag = "command")]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum AimCommand {
    #[serde(rename = "get")]
    Get,
    #[serde(rename = "getAllPatterns")]
    GetAllPatterns,
    #[serde(rename = "set")]
    Set(AimState),
    PreStack(AimState),
    #[serde(rename = "setpattern")]
    SetPattern {
        pattern: PatternParams,
    },
    #[serde(rename = "setfresnel")]
    SetFresnel {
        value: u32,
    },
    #[serde(rename = "response")]
    Response {
        reply: String,
    },
    #[serde(rename = "uploadimage")]
    UploadImage {
        name: String,
        imagedata: String,
    },
    #[serde(rename = "deleteimage")]
    DeleteImage {
        name: String,
    },
    #[serde(rename = "disconnect")]
    Disconnect,
    #[serde(rename = "setCorrectionPatternDeltas")]
    SetCorrectionPatternDeltas(CorrectionPatternDeltas),
    // Skip deserializing, because it has the same name as setCorrectionPatternDeltas
    #[serde(rename = "setCorrectionPatternDeltas", skip_deserializing)]
    SetCorrectionPatternDeltasResponse {
        wavelength: u32,
        success: bool,
    },
    #[serde(rename = "availablePatterns")]
    AvailablePatterns {
        patterns: AvailablePatterns,
    },
    #[serde(rename = "reboot")]
    Reboot,
}

#[serde(tag = "command")]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum EmbeddedCommand {
    #[serde(rename = "initdone")]
    InitDone,
    #[serde(rename = "set")]
    Set,
}

#[serde(tag = "device", rename_all = "snake_case")]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum MessageData {
    Embedded(EmbeddedCommand),
    Lasers(LaserCommand),
    Aim(AimCommand),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Message {
    #[serde(rename = "type")]
    pub m_type: MessageType, // type is a Rust keyword
    pub data: MessageData,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SpotPattern {
    pub position_xy: (f32, f32),
    pub diameter: f32,
    pub gradient_xy: (f32, f32),
    pub background_gradient_xy: (f32, f32),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CustomPattern {
    pub filename: String,
}

struct BasePatternVistior {}

impl<'de> Visitor<'de> for BasePatternVistior {
    type Value = BasePattern;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a very special map")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let entry: (String, HashMap<String, String>) = map
            .next_entry()?
            .ok_or(Error::custom("empty base pattern"))?;
        Ok(BasePattern {
            filename: entry.0.clone(),
            properties: entry.1,
        })
    }
}

impl<'de> Deserialize<'de> for BasePattern {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(BasePatternVistior {})
    }
}
#[derive(Serialize, Debug, Clone)]
pub struct BasePattern {
    pub filename: String,
    pub properties: HashMap<String, String>,
}

#[serde(untagged)]
#[derive(Deserialize, Serialize, Debug, Clone)]
pub enum PatternParams {
    Spot {
        spot: SpotPattern,
    },
    Custom {
        custom: CustomPattern,
    },
    Base {
        #[serde(flatten)]
        base: BasePattern,
    },
}
