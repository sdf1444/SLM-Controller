use std::convert::TryInto;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::string::ToString;

use log::{error, info};
use mqtt::{Client, Message as MqttMessage};
use sdl2::{event::Event, keyboard::Keycode};
use walkdir::WalkDir;

type Dim = ndarray::Dim<[usize; 2]>;

use crate::{
    schema::{
        APattern, AimCommand, AvailablePatterns, CorrectionPatternDeltas, EmbeddedCommand,
        LaserCommand, Message, MessageData, MessageType, PatternParams,
    },
    util::Subtopic,
    Array, Context, Result, State,
};

const TWO_PI: f32 = std::f32::consts::PI * 2.0;

fn base64_to_ndarray(s: &str, dim: Dim) -> Result<Array> {
    Ok(ndarray::Array2::from_shape_vec(
        dim,
        base64::decode(s)?
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
            .collect(),
    )?)
}

fn read_image_from_file(path: &Path, dim: Option<Dim>) -> Result<Array> {
    let factor = TWO_PI / 256.0;
    // a 2d array with !u8! elements
    let array = ndarray_image::open_gray_image(path)?;

    let normalize = |e: &u8| *e as f32 * factor;

    if let Some(dim) = dim {
        Ok(Array::from_shape_fn(dim, |id| {
            array.get(id).map(normalize).unwrap_or(0.0)
        }))
    } else {
        Ok(array.map(normalize))
    }
}

fn save_image(path: &Path, array: &Array) -> Result<()> {
    Ok(ndarray_image::save_gray_image(
        path,
        array
            .mapv(|e| (e.rem_euclid(TWO_PI) * 255.0 / TWO_PI) as u8)
            .view(),
    )?)
}

fn save_image_data(mut path: PathBuf, b64_data: String) -> Result<()> {
    let mut parts = b64_data.split(";base64,");
    let header = parts
        .next()
        .ok_or_else(|| format!("image data {} doesn't have a header", b64_data))?;
    let body = parts
        .next()
        .ok_or_else(|| format!("image data {} doesn't have a body", b64_data))?;

    let extension = header
        .split('/')
        .nth(1)
        .ok_or_else(|| format!("image header {} doesn't contain an extenstion", header))?;

    path.set_extension(extension);

    info!("Saving image to {:?}", path);
    File::create(path)?.write_all(&base64::decode(body)?)?;

    Ok(())
}

fn send_message(client: &mut Client, topic: &str, message: &Message) -> Result<()> {
    info!(
        "Sent message: Topic: {}, Contents:\n{}",
        topic,
        serde_json::to_string_pretty(&message).unwrap(),
    );

    client.publish(MqttMessage::new(topic, serde_json::to_vec(message)?, 0))?;

    Ok(())
}

impl<'a, 'b> Context<'a, 'b> {
    fn send_aim_message(&mut self, message: &Message) -> Result<&mut Self> {
        send_message(&mut self.client, &self.main_topic_aim, message)?;
        Ok(self)
    }

    fn available_patterns(&self) -> AvailablePatterns {
        let mut path = self.config.dir_path.base_patterns.clone();
        let mut patterns = AvailablePatterns::default();

        let process_value = |map_entry: &mut APattern, property: String, value: String| {
            if !map_entry.properties.contains(&property) {
                map_entry
                    .property_values
                    .insert(property.clone(), Default::default());
                map_entry.properties.push(property.clone());
            }
            map_entry
                .property_values
                .get_mut(&property)
                .unwrap()
                .values
                .push(value.to_owned());
        };

        for entry in WalkDir::new(&path).min_depth(1).max_depth(1) {
            // A wrapper for '?' operations
            let process_entry = || -> Option<()> {
                let entry = entry.ok()?;

                if !entry.file_type().is_file() {
                    return None;
                }

                let file_name = entry.path().file_stem()?.to_str()?;
                let mut parts_iter = file_name.split('_');
                let name = parts_iter.next()?;

                let map_entry = patterns.patterns.entry(name.to_owned()).or_default();

                loop {
                    let property = match parts_iter.next() {
                        Some(property) => property.to_owned(),
                        None => break,
                    };
                    let value = parts_iter.next()?.to_owned();

                    process_value(map_entry, property, value);
                }

                Some(())
            };

            process_entry();
        }

        path.push("custom_patterns");

        for entry in WalkDir::new(path).min_depth(1).max_depth(1) {
            // A wrapper for '?' operations
            let process_entry = || -> Option<()> {
                let entry = entry.ok()?;

                if !entry.file_type().is_file() {
                    return None;
                }

                let file_name = entry.file_name().to_str()?.to_owned();

                let map_entry = patterns.patterns.entry("custom".into()).or_default();
                process_value(map_entry, "filename".into(), file_name);

                Some(())
            };

            process_entry();
        }
        patterns.pattern_names = patterns.patterns.keys().cloned().collect();
        patterns.pattern_names.sort();

        patterns
    }

    fn send_available_patterns(&mut self) -> Result<&mut Self> {
        self.send_aim_message(&Message {
            m_type: MessageType::Device,
            data: MessageData::Aim(AimCommand::AvailablePatterns {
                patterns: self.available_patterns(),
            }),
        })
    }

    fn send_current_state(&mut self) -> Result<&mut Self> {
        //FIXME
        Ok(self)
    }

    fn send_prestack_done(&mut self) -> Result<&mut Self> {
        self.send_aim_message(&Message {
            m_type: MessageType::Device,
            data: MessageData::Aim(AimCommand::Response {
                reply: "PreStack done".to_string(),
            }),
        })?;
        Ok(self)
    }

    fn send_get_lasers(&mut self) -> Result<&mut Self> {
        // Possible improvement: cache this?
        self.send_aim_message(&Message {
            m_type: MessageType::Device,
            data: MessageData::Lasers(LaserCommand::Get),
        })
    }

    fn send_set_correction_pattern_deltas(&mut self, wavelength: u32) -> Result<&mut Self> {
        self.send_aim_message(&Message {
            m_type: MessageType::Device,
            data: MessageData::Aim(AimCommand::SetCorrectionPatternDeltasResponse {
                wavelength,
                success: true,
            }),
        })?;
        Ok(self)
    }

    fn load_data(&mut self, path: &Path, dim: Option<Dim>) -> Result<&Array> {
        if !self.state.cache.contains_key(path) {
            self.state
                .cache
                .insert(path.to_owned(), read_image_from_file(path, dim)?);
        }

        Ok(&self.state.cache[path])
    }

    fn put_pattern(&mut self, pattern: &ndarray::Array2<u8>) -> Result<()> {
        let size = self.config.screen.size;
        let pixels = &mut self.screen_context.pixels;

        for (id, value) in pattern.indexed_iter() {
            let idd = (id.1 * size.0 as usize + id.0) * 4;
            pixels[idd + 0] = *value;
            pixels[idd + 1] = *value;
            pixels[idd + 2] = *value;
        }
        let pitch = sdl2::pixels::PixelFormatEnum::ARGB8888.byte_size_of_pixels(size.0 as usize);
        self.screen_context.texture.update(None, &pixels, pitch)?;
        self.screen_context
            .canvas
            .copy(self.screen_context.texture, None, None)?;
        self.screen_context.canvas.present();
        Ok(())
    }

    fn get_file_path_for_flatness_corr_pattern(&self, wavelength: u32) -> Result<PathBuf> {
        let filename = "flatness_wavelength_".to_owned() + &wavelength.to_string();

        for factory in &["", "_factory"] {
            for ext in &self.config.image_file_extensions {
                let path = self
                    .config
                    .dir_path
                    .flatness_corr_patterns
                    .clone()
                    .join(filename.clone() + factory + ext);
                if path.is_file() {
                    return Ok(path);
                }
            }
        }
        Err(format!(
            "No flatness correction pattern for wavelength {}",
            wavelength
        ))?
    }

    fn get_file_path_for_base_corr_pattern(&self, pattern: &PatternParams) -> Result<PathBuf> {
        match pattern {
            PatternParams::Spot { .. } => Err("Cannot get file path for the spot pattern")?,
            PatternParams::Custom { custom } => {
                Ok(self.config.dir_path.base_patterns.join(&custom.filename))
            }
            PatternParams::Base { base } => {
                let mut filename = base.filename.clone();

                for (property, value) in &base.properties {
                    filename = filename + "_" + property + "_" + value;
                }
                let mut path = self.config.dir_path.base_patterns.join(filename);
                for ext in &self.config.image_file_extensions {
                    path.set_extension(ext);
                    if path.is_file() {
                        return Ok(path);
                    }
                }

                Err(format!("Can't find file for base pattern {:?}", pattern))?
            }
        }
    }

    fn add_correction_pattern_deltas(
        &mut self,
        pattern_deltas: &CorrectionPatternDeltas,
    ) -> Result<&mut Self> {
        let mut fp = self.get_file_path_for_flatness_corr_pattern(pattern_deltas.wavelength)?;
        let old_pattern = self.load_data(&fp, None)?;
        let delta = base64_to_ndarray(
            &pattern_deltas.imagedata,
            ndarray::Dim(pattern_deltas.shape_xy),
        )?;
        let new_pattern = old_pattern + &delta;

        let filename = fp
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .replace("_factory", "");
        fp.set_file_name(filename);
        save_image(&fp, &new_pattern)?;

        self.state.cache.insert(fp, new_pattern);

        Ok(self)
    }

    fn compute_pattern(&mut self) -> Result<ndarray::Array2<u8>> {
        let (size_x, size_y) = self.config.screen.size;
        let (size_x, size_y) = (size_x as usize, size_y as usize);

        let State {
            fresnel,
            wavelength,
            ref pattern_params,
            ..
        } = self.state;

        let dim = ndarray::Dim([size_x, size_y]);

        let lx = ndarray::Array1::linspace(0 as f32, size_x as f32, size_x);
        let ly = ndarray::Array1::linspace(0 as f32, size_y as f32, size_y);
        let mut xx = Array::zeros(ndarray::Dim([size_x, size_y]));
        let mut yy = xx.clone();
        for mut row in xx.gencolumns_mut() {
            row.assign(&lx);
        }

        for mut row in yy.genrows_mut() {
            row.assign(&ly);
        }

        let mut pattern = match &pattern_params {
            PatternParams::Spot { spot } => {
                let r2 = (spot.diameter / 2.0).powf(2.0);
                Array::from_shape_fn(dim, |id| {
                    if (xx[id] - spot.position_xy.0).powf(2.0)
                        + (yy[id] - spot.position_xy.1).powf(2.0)
                        < r2
                    {
                        spot.gradient_xy.0 * xx[id] + spot.gradient_xy.1 * yy[id]
                    } else {
                        spot.background_gradient_xy.0 * xx[id]
                            + spot.background_gradient_xy.1 * yy[id]
                    }
                })
            }
            PatternParams::Base { .. } | PatternParams::Custom { .. } => {
                let path = self.get_file_path_for_base_corr_pattern(&pattern_params)?;
                self.load_data(&path, Some(dim))?.clone()
            }
        };

        if self.config.compute_pattern.add_flatness_correction {
            let path = self.get_file_path_for_flatness_corr_pattern(wavelength)?;
            let flat_corr = self.load_data(&path, Some(dim))?;
            pattern += flat_corr;
        }

        let wvlen_fact = TWO_PI * 488.0 / wavelength as f32;
        let phi_max_x = 80.0; // Change for 12-bit mode
        let slope_x = -phi_max_x * wvlen_fact / size_x as f32;
        let gradient = slope_x * &xx + phi_max_x * wvlen_fact * 1.1;

        // TODO: add masking
        pattern += &gradient;

        if fresnel != 0 {
            let (xc, yc) = (size_x as f32 / 2.0, size_y as f32 / 2.0); // center of the SLM
            let pixel_size_nm = 12500_f32;
            let fresnel_in_1_over_nm = fresnel as f32 * 1e-9;
            let pre_factor = pixel_size_nm.powf(2.0) * std::f32::consts::PI * fresnel_in_1_over_nm
                / wavelength as f32;

            pattern += &(pre_factor
                * ((xx - xc).mapv_into(|e| e.powf(2.0)) + (yy - yc).mapv_into(|e| e.powf(2.0))));
        }

        let scaling = &self.config.compute_pattern.slm_calib_scaling;
        let scale_id = match scaling
            .known_wavelengths
            .iter()
            .position(|&e| e == wavelength)
        {
            Some(id) => id,
            None => {
                scaling
                    .known_wavelengths
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, &e)| e.max(wavelength) - e.min(wavelength))
                    .ok_or("no known wavelengths available")?
                    .0
            }
        };
        let scale = scaling.scale_factors[scale_id];

        let pattern = pattern.mapv(|e| (e.rem_euclid(TWO_PI) / TWO_PI * scale) as u8);

        if self
            .config
            .compute_pattern
            .debug
            .as_ref()
            .map(|d| d.save_computed_to_image)
            .unwrap_or(false)
        {
            ndarray_image::save_gray_image("computed_pattern.png", pattern.view())?;
        }

        Ok(pattern)
    }

    /// Update current state, with an ability to leave
    /// the existing value if passed `None`
    pub fn update_state(
        &mut self,
        pattern_params: Option<PatternParams>,
        fresnel: Option<u32>,
        wavelength: Option<u32>,
    ) -> Result<&mut Self> {
        if let Some(pattern_params) = pattern_params {
            self.state.pattern_params = pattern_params;
        }
        self.state.fresnel = fresnel.unwrap_or(self.state.fresnel);
        self.state.wavelength = wavelength.unwrap_or(self.state.wavelength);
        let pattern = self.compute_pattern()?;
        self.put_pattern(&pattern)?;

        Ok(self)
    }

    fn on_connect(&mut self) -> Result<()> {
        const SUBTOPICS: [&str; 4] = [
            "embedded/aim",
            "gui/aim",
            "calibration/aim",
            "embedded/lasers", // start -> give me all the lasers -> reply
        ];

        info!("Subscribing to topics");
        for subtopic in SUBTOPICS.iter() {
            let topic = self.config.main_topic().subtopic(subtopic);
            self.client.subscribe(&topic, 0)?;
            info!("Subscribed to {}", topic);
        }

        self.send_get_lasers()?
            .send_available_patterns()?
            .send_current_state()?;

        Ok(())
    }

    pub fn process_message(&mut self, mqtt_message: &MqttMessage) -> Result<()> {
        // Note: here the python script decodes the payload as a cp437 string,
        // however I feel like here there shouldn't be any interesting characters from cp437,
        // so it's fine to parse it as unicode
        let message: Message = serde_json::from_slice(mqtt_message.payload())?;

        info!(
            "Message recieved: Topic: {}, Contents: {:?}",
            mqtt_message.topic(),
            message
        );

        match (&message.m_type, &message.data) {
            (MessageType::Status, MessageData::Embedded(EmbeddedCommand::InitDone)) => {
                self.send_get_lasers()?
                    .send_available_patterns()?
                    .send_current_state()?;
            }
            (MessageType::Device, MessageData::Lasers(LaserCommand::Set { lasers })) => {
                info!("Received laser wavelengths and intensities.");
                info!("Selecting wavelength with highest intensity.");

                let strongest = lasers
                    .iter()
                    .filter(|laser| laser.state != 0 && laser.name != "led")
                    .max_by_key(|laser| laser.intensity);

                let strongest = match strongest {
                    Some(strongest) => strongest.wavelength,
                    None => {
                        info!("No lasers enabled; skipping");
                        return Ok(());
                    }
                };

                self.update_state(None, None, Some(strongest))?
                    .send_current_state()?;
            }
            _ => (),
        };

        let aim_command = match (&message.m_type, &message.data) {
            (MessageType::Device, MessageData::Aim(aim_command)) => aim_command.clone(),
            _ => Err(format!("Unexpected message: {:?}", message))?,
        };

        let custom_pattern_path = |name: &str| -> Result<PathBuf> {
            let mut path = std::env::current_dir()?;
            path.push(&self.config.dir_path.base_patterns);
            path.push("custom_patterns");
            path.push(name);
            Ok(path)
        };

        match aim_command {
            AimCommand::Set(aim_state) => {
                self.update_state(Some(aim_state.pattern), Some(aim_state.fresnel), None)?
                    .send_current_state()?;
            }
            AimCommand::PreStack(aim_state) => {
                self.update_state(Some(aim_state.pattern), Some(aim_state.fresnel), None)?
                    .send_current_state()?
                    .send_prestack_done()?;
            }
            // --------------  Messages coming from LuxControl GUI in live mode -----------
            AimCommand::Get => {
                self.send_current_state()?;
            }
            AimCommand::GetAllPatterns => {
                self.send_available_patterns()?;
            }
            AimCommand::SetFresnel { value } => {
                self.update_state(None, Some(value), None)?
                    .send_current_state()?;
            }
            AimCommand::SetPattern { pattern } => {
                self.update_state(Some(pattern), None, None)?
                    .send_current_state()?;
            }
            AimCommand::UploadImage { name, imagedata } => {
                save_image_data(custom_pattern_path(&name)?, imagedata)?;
                self.send_available_patterns()?.send_current_state()?;
            }
            AimCommand::DeleteImage { name } => {
                std::fs::remove_file(custom_pattern_path(&name)?)?;
                self.send_available_patterns()?.send_current_state()?;
            }
            // ----------  END Messages coming from LuxControl GUI in live mode -----------
            // --------------  Messages coming from SLM-calibraton software ---------------
            AimCommand::SetCorrectionPatternDeltas(pattern_deltas) => {
                self.add_correction_pattern_deltas(&pattern_deltas)?
                    .send_set_correction_pattern_deltas(pattern_deltas.wavelength)?;
            }
            // ----------- END Messages coming from SLM-calibraton software ---------------
            AimCommand::Reboot => {
                system_shutdown::reboot()?;
            }
            _ => (),
        }

        Ok(())
    }

    /// Subscribe to required topics and start dispatching messages
    pub fn message_loop(mut self) -> Result<()> {
        info!("Initializing message loop");

        // need to create the channel before calling on_connect, otherwise messages might be lost
        let message_channel = self.client.start_consuming();

        let mut window_events = self.screen_context.sdl_context.event_pump()?;

        self.on_connect()?;

        info!("Starting message processing");
        'message_loop: loop {
            // process messages from server
            if let Ok(Some(message)) = message_channel.try_recv() {
                if let Err(err) = self.process_message(&message) {
                    error!(
                        "Error {} while processing message {}; continuing",
                        err, message
                    );
                }
                continue;
            }

            // process events from the display window
            if let Some(event) = window_events.poll_event() {
                match event {
                    Event::KeyDown {
                        keycode: Some(Keycode::Escape),
                        ..
                    }
                    | Event::Quit { .. } => break 'message_loop,
                    _ => {}
                }

                continue;
            }
        }

        Ok(())
    }
}
