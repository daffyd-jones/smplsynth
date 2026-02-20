use crate::audio::AudioCommand;
use midir::{MidiInput, MidiInputConnection}; // MidiInputPort removed
use rtrb::Producer;
use std::sync::{Arc, Mutex};

pub struct MidiHandler {
    connection: Option<MidiInputConnection<()>>,
    ports: Vec<String>,
    connected_port: Option<String>,
    producer: Arc<Mutex<Producer<AudioCommand>>>,
}

impl MidiHandler {
    pub fn new(producer: Producer<AudioCommand>) -> Self {
        let mut handler = Self {
            connection: None,
            ports: Vec::new(),
            connected_port: None,
            producer: Arc::new(Mutex::new(producer)),
        };
        handler.refresh_ports();
        handler
    }

    pub fn refresh_ports(&mut self) {
        self.ports.clear();
        if let Ok(midi_in) = MidiInput::new("synth-scan") {
            for port in midi_in.ports() {
                if let Ok(name) = midi_in.port_name(&port) {
                    self.ports.push(name);
                }
            }
        }
    }

    pub fn port_names(&self) -> Vec<String> {
        self.ports.clone()
    }

    pub fn connected_port_name(&self) -> Option<&str> {
        self.connected_port.as_deref()
    }

    pub fn is_connected(&self) -> bool {
        self.connection.is_some()
    }

    pub fn connect(&mut self, port_index: usize) -> Result<(), String> {
        self.disconnect();

        let midi_in = MidiInput::new("synth-sampler")
            .map_err(|e| format!("Failed to create MIDI input: {}", e))?;

        let ports = midi_in.ports();
        if port_index >= ports.len() {
            return Err(format!("Port index {} out of range", port_index));
        }

        let port = &ports[port_index];
        let port_name = midi_in
            .port_name(port)
            .unwrap_or_else(|_| "Unknown".to_string());

        let producer = Arc::clone(&self.producer);

        let connection = midi_in
            .connect(
                port,
                "synth-input",
                move |_ts, message, _| {
                    Self::handle_message(message, &producer);
                },
                (),
            )
            .map_err(|e| format!("Failed to connect: {}", e))?;

        self.connected_port = Some(port_name);
        self.connection = Some(connection);

        Ok(())
    }

    pub fn disconnect(&mut self) {
        self.connection = None;
        self.connected_port = None;
    }

    fn handle_message(message: &[u8], producer: &Arc<Mutex<Producer<AudioCommand>>>) {
        if message.len() < 3 {
            return;
        }

        let status = message[0];
        let data1 = message[1];
        let data2 = message[2];

        let command = status & 0xF0;
        let midi_channel = (status & 0x0F) as usize;

        if midi_channel >= 8 {
            return;
        }

        let cmd = match command {
            0x90 if data2 > 0 => Some(AudioCommand::NoteOn {
                channel: midi_channel,
                note: data1,
                velocity: data2,
            }),
            0x80 | 0x90 => Some(AudioCommand::NoteOff {
                channel: midi_channel,
                note: data1,
            }),
            _ => None,
        };

        if let Some(cmd) = cmd {
            if let Ok(mut tx) = producer.lock() {
                let _ = tx.push(cmd);
            }
        }
    }
}

