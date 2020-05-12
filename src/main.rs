use std::collections::HashMap;
use std::error::Error;
use std::fs::File;
use std::io::{BufReader, Write};
use std::path::PathBuf;
use std::time::Duration;

use flexi_logger::{DeferredNow, LogSpecification, Logger};
use log::{error, info, Record as LogRecord};
use mqtt::{Client, ConnectOptionsBuilder, Message as MqttMessage};
use sdl2::{
    pixels::PixelFormatEnum,
    render::{Canvas, Texture},
    video::Window,
    Sdl,
};

pub type Array = ndarray::Array2<f32>;

mod message_loop;
mod schema;
mod util;

use schema::{AimCommand, Config, Message, MessageData, MessageType, PatternParams};
use util::Subtopic;

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub struct ScreenContext<'a, 'b> {
    pub canvas: &'a mut Canvas<Window>,
    pub texture: &'a mut Texture<'b>,
    pub sdl_context: &'a Sdl,
    pub pixels: Vec<u8>,
}

pub struct State {
    pub wavelength: u32,
    pub fresnel: u32,
    pub pattern_params: PatternParams,
    pub cache: HashMap<PathBuf, Array>,
}
pub struct Context<'a, 'b> {
    pub config: Config,
    pub client: Client,
    pub screen_context: ScreenContext<'a, 'b>,
    pub state: State,
    pub main_topic_aim: String, // We need this a lot, might as well precalucalate it
}

impl<'a, 'b> Context<'a, 'b> {
    fn new(
        config: Config,
        client: Client,
        screen_context: ScreenContext<'a, 'b>,
        state: State,
    ) -> Self {
        Context {
            main_topic_aim: config.main_topic().subtopic("aim"),
            config,
            screen_context,
            client,
            state,
        }
    }
}

fn last_will_message(config: &Config) -> MqttMessage {
    let message = Message {
        m_type: MessageType::Device,
        data: MessageData::Aim(AimCommand::Disconnect),
    };
    let topic = config.main_topic().subtopic("aim");

    info!(
        "Set last will message: Topic: {}, Contents: {}",
        topic,
        serde_json::to_string_pretty(&message).unwrap()
    );

    MqttMessage::new(&topic, serde_json::to_vec(&message).unwrap(), 0)
}

/// Format function for printing log entries
fn logger_format(
    write: &mut dyn Write,
    now: &mut DeferredNow,
    record: &LogRecord,
) -> std::result::Result<(), std::io::Error> {
    // timestamp - caller - level - message
    write!(
        write,
        "{} - {} - {} - {}",
        now.now(),
        record.target(),
        record.level(),
        record.args()
    )
}

fn initialize_logger(config: &Config) -> Result<()> {
    // Set level filter to the config value
    Logger::with(
        LogSpecification::default(config.logging.log_level.into_level_filter()).finalize(),
    )
    .log_to_file()
    .format(logger_format) // set format function for log entries
    .rotate(
        // rotation logger settings
        flexi_logger::Criterion::Size(500000), // Maximum size of each log file
        flexi_logger::Naming::Numbers,
        flexi_logger::Cleanup::KeepLogFiles(2), // Number of log files to keep
    )
    .start()?;
    Ok(())
}

fn initialize_state(config: &Config) -> State {
    State {
        wavelength: config.defaults.wavelength,
        fresnel: config.defaults.fresnel,
        pattern_params: config.defaults.pattern.clone(),
        cache: Default::default(),
    }
}

/// Parse config from `config.json`;
/// Initialize logger;
/// Connect to the server
fn initialize() -> Result<(Config, Client)> {
    let config: Config = serde_json::from_reader(BufReader::new(File::open("config.json")?))?;
    initialize_logger(&config)?;
    info!("Parsed config; initialized logger");

    // Create a client instance with the address given in config
    let client = Client::new(config.mqtt.server_uri())?;

    let connect_options = ConnectOptionsBuilder::new()
        .clean_session(true)
        .retry_interval(Duration::from_secs(10))
        .automatic_reconnect(Duration::from_secs(1), Duration::from_secs(120))
        .will_message(last_will_message(&config))
        .finalize();

    info!(
        "Connecting to the server on {}...",
        config.mqtt.server_uri()
    );
    let response = client.connect(connect_options)?;
    info!("Connected with result code {}", response.1);

    Ok((config, client))
}

// A convenience function to propagate all errors to one place
fn err_wrapper() -> Result<()> {
    let (config, client) = initialize()?;

    // Initialize SDL structures
    let sdl_context = sdl2::init()?;
    let video_subsystem = sdl_context.video()?;

    let (width, height) = config.screen.size;

    // create window
    let mut window = video_subsystem.window("pew-pew", width, height);
    if config.screen.fullscreen {
        window.fullscreen_desktop().borderless();
    };
    let window = window.position_centered().opengl().build()?;

    // create handles for drawing to the window
    let mut canvas = window.into_canvas().build()?;
    let creator = canvas.texture_creator();
    let mut texture = creator.create_texture_target(PixelFormatEnum::ARGB8888, width, height)?;

    let screen_context = ScreenContext {
        canvas: &mut canvas,
        texture: &mut texture,
        pixels: vec![0; (width * height * 4) as usize],
        sdl_context: &sdl_context,
    };

    let state = initialize_state(&config);

    let mut context = Context::new(config, client, screen_context, state);

    // Update state from the defaults
    context.update_state(None, None, None)?;

    // Start dispatching messages
    context.message_loop()?;

    Ok(())
}

fn main() {
    if let Err(err) = err_wrapper() {
        let error = format!("Encountered an unrecoverable error: {}", err);
        error!("{}", error);
        println!("{}", error);
        return;
    };
}
