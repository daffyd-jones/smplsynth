mod audio;
mod midi;
mod ui;

use audio::AudioCommand;
use rtrb::RingBuffer;

fn main() -> Result<(), eframe::Error> {
    // Create separate ring buffers for UI and MIDI
    // rtrb is specifically designed for real-time audio - lock-free, wait-free
    let (ui_tx, ui_rx) = RingBuffer::<AudioCommand>::new(1024);
    let (midi_tx, midi_rx) = RingBuffer::<AudioCommand>::new(512);

    // Start audio thread with both consumers
    let _stream = audio::start_audio_thread(ui_rx, midi_rx);

    // Create MIDI handler with its dedicated producer
    let midi_handler = midi::MidiHandler::new(midi_tx);

    // Create app with UI producer
    let app = ui::SynthApp::new(ui_tx, midi_handler);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([800.0, 600.0])
            .with_title("8-Channel Synth/Sampler"),
        ..Default::default()
    };

    eframe::run_native("Synth/Sampler", options, Box::new(|_cc| Ok(Box::new(app))))
}
