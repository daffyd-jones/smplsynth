use crate::audio::{
    load_sample_from_file, note_name, AudioCommand, ChannelParams, SampleData, VoiceType,
};
use crate::midi::MidiHandler;
use eframe::egui;
use rtrb::Producer;
use std::path::PathBuf;
use std::sync::mpsc;

// ============================================================================
// Color Theme
// ============================================================================

mod colors {
    use eframe::egui::Color32;

    pub const BG_DARK: Color32 = Color32::from_rgb(10, 14, 20);
    pub const BG_PANEL: Color32 = Color32::from_rgb(19, 23, 31);
    pub const BG_CHANNEL: Color32 = Color32::from_rgb(26, 31, 43);
    pub const ACCENT_CYAN: Color32 = Color32::from_rgb(0, 229, 255);
    pub const ACCENT_MAGENTA: Color32 = Color32::from_rgb(255, 0, 170);
    pub const ACCENT_YELLOW: Color32 = Color32::from_rgb(255, 214, 0);
    pub const BORDER: Color32 = Color32::from_rgb(42, 49, 64);
    pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(224, 230, 240);
    pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(122, 131, 148);
    pub const SUCCESS: Color32 = Color32::from_rgb(0, 255, 136);
}

// ============================================================================
// Sample Load State
// ============================================================================

/// Tracks sample loading state per channel
#[derive(Default)]
struct ChannelSampleState {
    /// Name of the loaded sample (for display)
    loaded_name: Option<String>,
    /// Loading in progress
    loading: bool,
    /// Error message if load failed
    error: Option<String>,
}

// ============================================================================
// Application State
// ============================================================================

pub struct SynthApp {
    pub ui_tx: Producer<AudioCommand>,
    midi_handler: MidiHandler,
    channels: [ChannelParams; 8],
    sample_states: [ChannelSampleState; 8],
    master_volume: f32,
    selected_midi_port: Option<usize>,
    test_notes_held: [bool; 4],
    /// Receiver for completed sample loads (from background threads)
    sample_load_rx: mpsc::Receiver<(usize, Result<SampleData, String>)>,
    /// Sender cloned into background load threads
    sample_load_tx: mpsc::Sender<(usize, Result<SampleData, String>)>,
}

impl SynthApp {
    pub fn new(ui_tx: Producer<AudioCommand>, midi_handler: MidiHandler) -> Self {
        let (sample_load_tx, sample_load_rx) = mpsc::channel();

        Self {
            ui_tx,
            midi_handler,
            channels: std::array::from_fn(|_| ChannelParams::default()),
            sample_states: std::array::from_fn(|_| ChannelSampleState::default()),
            master_volume: 0.75,
            selected_midi_port: None,
            test_notes_held: [false; 4],
            sample_load_rx,
            sample_load_tx,
        }
    }

    fn send(&mut self, cmd: AudioCommand) {
        let _ = self.ui_tx.push(cmd);
    }

    fn send_params(&mut self, channel: usize) {
        let params = self.channels[channel].clone();
        self.send(AudioCommand::UpdateParams { channel, params });
    }

    /// Process any completed sample loads
    fn process_sample_loads(&mut self) {
        while let Ok((channel, result)) = self.sample_load_rx.try_recv() {
            self.sample_states[channel].loading = false;

            match result {
                Ok(sample_data) => {
                    self.sample_states[channel].loaded_name = Some(sample_data.name.clone());
                    self.sample_states[channel].error = None;
                    self.send(AudioCommand::LoadSample {
                        channel,
                        sample_data,
                    });
                }
                Err(e) => {
                    self.sample_states[channel].error = Some(e);
                }
            }
        }
    }

    /// Start loading a sample file in a background thread
    fn start_sample_load(&mut self, channel: usize, path: PathBuf) {
        self.sample_states[channel].loading = true;
        self.sample_states[channel].error = None;

        let tx = self.sample_load_tx.clone();

        std::thread::spawn(move || {
            let result = load_sample_from_file(&path);
            let _ = tx.send((channel, result));
        });
    }
}

impl eframe::App for SynthApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Check for completed sample loads
        self.process_sample_loads();

        apply_theme(ctx);

        egui::TopBottomPanel::top("header")
            .frame(egui::Frame::none().fill(colors::BG_DARK).inner_margin(12.0))
            .show(ctx, |ui| {
                self.render_header(ui);
            });

        egui::TopBottomPanel::bottom("controls")
            .frame(egui::Frame::none().fill(colors::BG_DARK).inner_margin(12.0))
            .show(ctx, |ui| {
                self.render_controls(ui);
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(colors::BG_DARK).inner_margin(12.0))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.render_channels(ui);
                });
            });

        ctx.request_repaint();
    }
}

// ============================================================================
// Theme
// ============================================================================

fn apply_theme(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    let mut visuals = egui::Visuals::dark();

    visuals.panel_fill = colors::BG_DARK;
    visuals.window_fill = colors::BG_PANEL;

    visuals.widgets.noninteractive.bg_fill = colors::BG_CHANNEL;
    visuals.widgets.inactive.bg_fill = colors::BG_CHANNEL;
    visuals.widgets.hovered.bg_fill = colors::BG_PANEL;
    visuals.widgets.active.bg_fill = colors::BG_PANEL;

    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, colors::TEXT_PRIMARY);
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, colors::TEXT_PRIMARY);
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, colors::ACCENT_CYAN);
    visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, colors::ACCENT_CYAN);

    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, colors::BORDER);
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, colors::BORDER);
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, colors::ACCENT_CYAN);
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, colors::ACCENT_CYAN);

    visuals.selection.bg_fill = colors::ACCENT_CYAN.gamma_multiply(0.3);
    visuals.selection.stroke = egui::Stroke::new(1.0, colors::ACCENT_CYAN);

    style.visuals = visuals;
    ctx.set_style(style);
}

// ============================================================================
// Header
// ============================================================================

impl SynthApp {
    fn render_header(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(
                egui::RichText::new("SYNTH/SAMPLER")
                    .color(colors::ACCENT_CYAN)
                    .size(28.0)
                    .strong(),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (midi_text, midi_color) = if self.midi_handler.is_connected() {
                    let port = self
                        .midi_handler
                        .connected_port_name()
                        .unwrap_or("Connected");
                    let display = if port.len() > 30 {
                        format!("{}...", &port[..27])
                    } else {
                        port.to_string()
                    };
                    (format!("MIDI: {}", display), colors::SUCCESS)
                } else {
                    ("MIDI: Disconnected".to_string(), colors::TEXT_SECONDARY)
                };

                ui.label(egui::RichText::new(midi_text).color(midi_color).size(11.0));
                ui.separator();
                ui.label(
                    egui::RichText::new("AUDIO: Active")
                        .color(colors::SUCCESS)
                        .size(11.0),
                );
            });
        });
    }
}

// ============================================================================
// Controls Bar
// ============================================================================

impl SynthApp {
    fn render_controls(&mut self, ui: &mut egui::Ui) {
        egui::Frame::none()
            .fill(colors::BG_PANEL)
            .inner_margin(16.0)
            .stroke(egui::Stroke::new(1.0, colors::BORDER))
            .rounding(4.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    self.render_midi_controls(ui);

                    ui.add_space(24.0);
                    ui.separator();
                    ui.add_space(24.0);

                    self.render_master_volume(ui);

                    ui.add_space(24.0);
                    ui.separator();
                    ui.add_space(24.0);

                    self.render_test_buttons(ui);
                });
            });
    }

    fn render_midi_controls(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new("MIDI INPUT")
                    .color(colors::TEXT_SECONDARY)
                    .size(10.0),
            );

            let port_names: Vec<String> = self.midi_handler.port_names();

            let current_text = self
                .selected_midi_port
                .and_then(|i| port_names.get(i).cloned())
                .unwrap_or_else(|| "Select Device".to_string());

            ui.horizontal(|ui| {
                let mut connect_to: Option<usize> = None;

                egui::ComboBox::from_id_salt("midi_select")
                    .selected_text(&current_text)
                    .width(220.0)
                    .show_ui(ui, |ui| {
                        for (i, name) in port_names.iter().enumerate() {
                            let selected = self.selected_midi_port == Some(i);
                            if ui.selectable_label(selected, name).clicked() {
                                self.selected_midi_port = Some(i);
                                connect_to = Some(i);
                            }
                        }
                    });

                if let Some(idx) = connect_to {
                    if let Err(e) = self.midi_handler.connect(idx) {
                        eprintln!("MIDI connect error: {}", e);
                    }
                }

                if ui.small_button("⟳ Refresh").clicked() {
                    self.midi_handler.refresh_ports();
                    self.selected_midi_port = None;
                }
            });
        });
    }

    fn render_master_volume(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new("MASTER VOLUME")
                    .color(colors::TEXT_SECONDARY)
                    .size(10.0),
            );

            ui.horizontal(|ui| {
                if ui
                    .add(
                        egui::Slider::new(&mut self.master_volume, 0.0..=1.0)
                            .show_value(false)
                            .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                    )
                    .changed()
                {
                    self.send(AudioCommand::SetMasterVolume(self.master_volume));
                }

                ui.label(
                    egui::RichText::new(format!("{:.0}%", self.master_volume * 100.0))
                        .color(colors::ACCENT_CYAN)
                        .strong()
                        .size(14.0),
                );
            });
        });
    }

    fn render_test_buttons(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new("TEST (CH 1)")
                    .color(colors::TEXT_SECONDARY)
                    .size(10.0),
            );

            ui.horizontal(|ui| {
                const TEST_NOTES: [u8; 4] = [60, 64, 67, 72];

                let mut transitions: Vec<(u8, bool)> = Vec::new();

                for (i, &note) in TEST_NOTES.iter().enumerate() {
                    let btn = ui.button(egui::RichText::new(note_name(note)).size(12.0).color(
                        if self.test_notes_held[i] {
                            colors::ACCENT_CYAN
                        } else {
                            colors::TEXT_PRIMARY
                        },
                    ));

                    let is_held = btn.is_pointer_button_down_on();
                    let was_held = self.test_notes_held[i];

                    if is_held != was_held {
                        transitions.push((note, is_held));
                    }
                    self.test_notes_held[i] = is_held;
                }

                for (note, pressed) in transitions {
                    if pressed {
                        self.send(AudioCommand::NoteOn {
                            channel: 0,
                            note,
                            velocity: 100,
                        });
                    } else {
                        self.send(AudioCommand::NoteOff { channel: 0, note });
                    }
                }
            });
        });
    }
}

// ============================================================================
// Channels
// ============================================================================

impl SynthApp {
    fn render_channels(&mut self, ui: &mut egui::Ui) {
        let available_width = ui.available_width();
        let columns = ((available_width / 300.0).floor() as usize).clamp(1, 4);

        // Collect changes and sample load requests
        let mut changed_indices: Vec<usize> = Vec::new();
        let mut load_requests: Vec<(usize, PathBuf)> = Vec::new();

        ui.columns(columns, |cols| {
            for idx in 0..8 {
                let col = idx % columns;
                let mut params = self.channels[idx].clone();
                let sample_state = &self.sample_states[idx];

                let (changed, load_path) =
                    render_channel_strip(&mut cols[col], idx, &mut params, sample_state);

                if changed {
                    self.channels[idx] = params;
                    changed_indices.push(idx);
                }

                if let Some(path) = load_path {
                    load_requests.push((idx, path));
                }
            }
        });

        for idx in changed_indices {
            self.send_params(idx);
        }

        for (idx, path) in load_requests {
            self.start_sample_load(idx, path);
        }
    }
}

// ============================================================================
// Channel Strip
// ============================================================================

fn render_channel_strip(
    ui: &mut egui::Ui,
    idx: usize,
    params: &mut ChannelParams,
    sample_state: &ChannelSampleState,
) -> (bool, Option<PathBuf>) {
    let mut changed = false;
    let mut load_path: Option<PathBuf> = None;

    egui::Frame::none()
        .fill(colors::BG_CHANNEL)
        .inner_margin(16.0)
        .stroke(egui::Stroke::new(1.0, colors::BORDER))
        .rounding(4.0)
        .show(ui, |ui| {
            ui.set_min_width(270.0);

            // Header
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("CH {}", idx + 1))
                        .color(colors::ACCENT_CYAN)
                        .size(20.0)
                        .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(format!("MIDI CH {}", idx + 1))
                            .color(colors::TEXT_SECONDARY)
                            .size(9.0),
                    );
                });
            });

            ui.separator();
            ui.add_space(4.0);

            // Voice selector
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("VOICE")
                        .color(colors::TEXT_SECONDARY)
                        .size(10.0),
                );

                egui::ComboBox::from_id_salt(format!("voice_{}", idx))
                    .selected_text(params.voice.name())
                    .show_ui(ui, |ui| {
                        for voice in VoiceType::ALL {
                            if ui
                                .selectable_value(&mut params.voice, voice, voice.name())
                                .changed()
                            {
                                changed = true;
                            }
                        }
                    });
            });

            // Sample controls (only shown when Sample voice selected)
            if params.voice == VoiceType::Sample {
                ui.add_space(4.0);
                load_path = render_sample_controls(ui, idx, sample_state);
            }

            // Volume
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("VOL")
                        .color(colors::TEXT_SECONDARY)
                        .size(10.0),
                );

                if ui
                    .add(egui::Slider::new(&mut params.volume, 0.0..=1.0).show_value(false))
                    .changed()
                {
                    changed = true;
                }

                ui.label(
                    egui::RichText::new(format!("{:.0}", params.volume * 100.0))
                        .color(colors::TEXT_PRIMARY)
                        .size(11.0),
                );
            });

            ui.add_space(8.0);

            // ADSR
            changed |= render_adsr(ui, params);

            ui.add_space(8.0);

            // Filter
            changed |= render_filter(ui, params);

            ui.add_space(8.0);

            // Mute
            let mute_color = if params.muted {
                colors::ACCENT_MAGENTA
            } else {
                colors::TEXT_SECONDARY
            };
            if ui
                .button(
                    egui::RichText::new(if params.muted { "MUTED" } else { "MUTE" })
                        .color(mute_color)
                        .size(11.0),
                )
                .clicked()
            {
                params.muted = !params.muted;
                changed = true;
            }
        });

    ui.add_space(12.0);
    (changed, load_path)
}

// ============================================================================
// Sample Controls
// ============================================================================

fn render_sample_controls(
    ui: &mut egui::Ui,
    idx: usize,
    sample_state: &ChannelSampleState,
) -> Option<PathBuf> {
    let mut load_path: Option<PathBuf> = None;

    egui::Frame::none()
        .fill(colors::BG_PANEL)
        .inner_margin(10.0)
        .stroke(egui::Stroke::new(
            1.0,
            colors::ACCENT_YELLOW.gamma_multiply(0.3),
        ))
        .rounding(2.0)
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new("SAMPLE")
                    .color(colors::ACCENT_YELLOW)
                    .size(10.0),
            );

            ui.add_space(4.0);

            if sample_state.loading {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        egui::RichText::new("Loading...")
                            .color(colors::TEXT_SECONDARY)
                            .size(10.0),
                    );
                });
            } else {
                // Load button
                if ui
                    .button(
                        egui::RichText::new("📁 Load Sample")
                            .color(colors::TEXT_PRIMARY)
                            .size(11.0),
                    )
                    .clicked()
                {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Audio", &["wav", "mp3", "ogg", "flac", "aac", "m4a"])
                        .set_title(&format!("Load Sample for Channel {}", idx + 1))
                        .pick_file()
                    {
                        load_path = Some(path);
                    }
                }

                // Show loaded sample name
                if let Some(name) = &sample_state.loaded_name {
                    ui.add_space(4.0);
                    let display_name = if name.len() > 25 {
                        format!("{}...", &name[..22])
                    } else {
                        name.clone()
                    };
                    ui.label(
                        egui::RichText::new(format!("✓ {}", display_name))
                            .color(colors::SUCCESS)
                            .size(10.0),
                    );
                }

                // Show error if any
                if let Some(err) = &sample_state.error {
                    ui.add_space(4.0);
                    let display_err = if err.len() > 30 {
                        format!("{}...", &err[..27])
                    } else {
                        err.clone()
                    };
                    ui.label(
                        egui::RichText::new(format!("✗ {}", display_err))
                            .color(colors::ACCENT_MAGENTA)
                            .size(10.0),
                    );
                }
            }
        });

    load_path
}

// ============================================================================
// ADSR Section
// ============================================================================

fn render_adsr(ui: &mut egui::Ui, params: &mut ChannelParams) -> bool {
    let mut changed = false;

    egui::Frame::none()
        .fill(colors::BG_PANEL)
        .inner_margin(12.0)
        .stroke(egui::Stroke::new(
            1.0,
            colors::ACCENT_CYAN.gamma_multiply(0.2),
        ))
        .rounding(2.0)
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new("ENVELOPE")
                    .color(colors::ACCENT_CYAN)
                    .size(10.0),
            );
            ui.add_space(4.0);

            ui.columns(4, |cols| {
                let sliders: [(&str, &mut f32, std::ops::RangeInclusive<f32>); 4] = [
                    ("ATK", &mut params.attack, 0.001..=2.0),
                    ("DEC", &mut params.decay, 0.001..=2.0),
                    ("SUS", &mut params.sustain, 0.0..=1.0),
                    ("REL", &mut params.release, 0.001..=4.0),
                ];

                for (i, (label, val, range)) in sliders.into_iter().enumerate() {
                    cols[i].vertical(|ui| {
                        ui.label(
                            egui::RichText::new(label)
                                .color(colors::TEXT_SECONDARY)
                                .size(9.0),
                        );

                        let slider = egui::Slider::new(val, range)
                            .vertical()
                            .show_value(false)
                            .logarithmic(label != "SUS");

                        if ui.add(slider).changed() {
                            changed = true;
                        }

                        ui.label(
                            egui::RichText::new(format!("{:.3}", *val))
                                .color(colors::TEXT_PRIMARY)
                                .size(9.0),
                        );
                    });
                }
            });
        });

    changed
}

// ============================================================================
// Filter Section
// ============================================================================

fn render_filter(ui: &mut egui::Ui, params: &mut ChannelParams) -> bool {
    let mut changed = false;

    egui::Frame::none()
        .fill(colors::BG_PANEL)
        .inner_margin(12.0)
        .stroke(egui::Stroke::new(
            1.0,
            colors::ACCENT_MAGENTA.gamma_multiply(0.2),
        ))
        .rounding(2.0)
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new("LOW PASS FILTER")
                    .color(colors::ACCENT_MAGENTA)
                    .size(10.0),
            );
            ui.add_space(4.0);

            // Cutoff
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("CUTOFF")
                        .color(colors::TEXT_SECONDARY)
                        .size(9.0),
                );

                if ui
                    .add(
                        egui::Slider::new(&mut params.filter_freq, 20.0..=20000.0)
                            .logarithmic(true)
                            .show_value(false),
                    )
                    .changed()
                {
                    changed = true;
                }

                let freq_label = if params.filter_freq >= 1000.0 {
                    format!("{:.1}kHz", params.filter_freq / 1000.0)
                } else {
                    format!("{:.0}Hz", params.filter_freq)
                };
                ui.label(
                    egui::RichText::new(freq_label)
                        .color(colors::TEXT_PRIMARY)
                        .size(10.0),
                );
            });

            // Resonance
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("RES")
                        .color(colors::TEXT_SECONDARY)
                        .size(9.0),
                );

                if ui
                    .add(
                        egui::Slider::new(&mut params.filter_q, 0.1..=20.0)
                            .logarithmic(true)
                            .show_value(false),
                    )
                    .changed()
                {
                    changed = true;
                }

                ui.label(
                    egui::RichText::new(format!("{:.2}", params.filter_q))
                        .color(colors::TEXT_PRIMARY)
                        .size(10.0),
                );
            });
        });

    changed
}
