use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rtrb::Consumer;
use std::sync::Arc;

// ============================================================================
// Types
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VoiceType {
    Sine,
    Sawtooth,
    Square,
    Triangle,
    Sample,
}

impl VoiceType {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Sine => "Sine",
            Self::Sawtooth => "Sawtooth",
            Self::Square => "Square",
            Self::Triangle => "Triangle",
            Self::Sample => "Sample",
        }
    }

    pub const ALL: [Self; 5] = [
        Self::Sine,
        Self::Sawtooth,
        Self::Square,
        Self::Triangle,
        Self::Sample,
    ];
}

/// Decoded sample data, stored in Arc for zero-copy sharing
#[derive(Clone)]
pub struct SampleData {
    /// Mono samples, normalized to -1.0..1.0
    pub samples: Arc<Vec<f32>>,
    /// Original sample rate of the audio file
    pub sample_rate: u32,
    /// Name of the loaded file (for UI display)
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct ChannelParams {
    pub voice: VoiceType,
    pub volume: f32,
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
    pub filter_freq: f32,
    pub filter_q: f32,
    pub muted: bool,
}

impl Default for ChannelParams {
    fn default() -> Self {
        Self {
            voice: VoiceType::Sine,
            volume: 0.7,
            attack: 0.01,
            decay: 0.1,
            sustain: 0.7,
            release: 0.3,
            filter_freq: 2000.0,
            filter_q: 1.0,
            muted: false,
        }
    }
}

#[derive(Clone)]
pub enum AudioCommand {
    NoteOn {
        channel: usize,
        note: u8,
        velocity: u8,
    },
    NoteOff {
        channel: usize,
        note: u8,
    },
    UpdateParams {
        channel: usize,
        params: ChannelParams,
    },
    SetMasterVolume(f32),
    LoadSample {
        channel: usize,
        sample_data: SampleData,
    },
}

impl std::fmt::Debug for AudioCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoteOn {
                channel,
                note,
                velocity,
            } => {
                write!(
                    f,
                    "NoteOn {{ ch: {}, note: {}, vel: {} }}",
                    channel, note, velocity
                )
            }
            Self::NoteOff { channel, note } => {
                write!(f, "NoteOff {{ ch: {}, note: {} }}", channel, note)
            }
            Self::UpdateParams { channel, .. } => {
                write!(f, "UpdateParams {{ ch: {} }}", channel)
            }
            Self::SetMasterVolume(v) => write!(f, "SetMasterVolume({})", v),
            Self::LoadSample {
                channel,
                sample_data,
            } => {
                write!(
                    f,
                    "LoadSample {{ ch: {}, name: {} }}",
                    channel, sample_data.name
                )
            }
        }
    }
}

// ============================================================================
// Oscillator
// ============================================================================

#[derive(Clone)]
struct Oscillator {
    phase: f32,
    phase_inc: f32,
    voice: VoiceType,
}

impl Oscillator {
    fn new(freq: f32, sample_rate: f32, voice: VoiceType) -> Self {
        Self {
            phase: 0.0,
            phase_inc: freq / sample_rate,
            voice,
        }
    }

    fn next_sample(&mut self) -> f32 {
        let sample = match self.voice {
            VoiceType::Sine => (self.phase * std::f32::consts::TAU).sin(),
            VoiceType::Sawtooth => 2.0 * self.phase - 1.0,
            VoiceType::Square => {
                if self.phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            VoiceType::Triangle => {
                if self.phase < 0.5 {
                    4.0 * self.phase - 1.0
                } else {
                    3.0 - 4.0 * self.phase
                }
            }
            VoiceType::Sample => 0.0, // Handled separately
        };
        self.phase = (self.phase + self.phase_inc) % 1.0;
        sample
    }
}

// ============================================================================
// Sample Player
// ============================================================================

#[derive(Clone)]
struct SamplePlayer {
    /// Current playback position (sub-sample precision for pitch shifting)
    position: f64,
    /// Playback rate (1.0 = original pitch, 2.0 = octave up, 0.5 = octave down)
    playback_rate: f64,
    /// Reference to the sample data
    sample_data: SampleData,
    /// Whether playback has finished
    finished: bool,
}

impl SamplePlayer {
    fn new(sample_data: SampleData, note: u8, output_sample_rate: f32) -> Self {
        // Calculate playback rate:
        // - Base pitch is C4 (MIDI note 60)
        // - Each semitone = 2^(1/12) ratio
        // - Also compensate for sample rate difference
        let semitones = note as f64 - 60.0;
        let pitch_ratio = 2.0f64.powf(semitones / 12.0);
        let rate_ratio = sample_data.sample_rate as f64 / output_sample_rate as f64;

        Self {
            position: 0.0,
            playback_rate: pitch_ratio * rate_ratio,
            sample_data,
            finished: false,
        }
    }

    fn next_sample(&mut self) -> f32 {
        if self.finished {
            return 0.0;
        }

        let samples = &self.sample_data.samples;
        let len = samples.len();

        if len == 0 {
            self.finished = true;
            return 0.0;
        }

        let pos = self.position;
        let idx = pos as usize;

        // Check if we've reached the end
        if idx >= len - 1 {
            self.finished = true;
            return 0.0;
        }

        // Linear interpolation between samples for smooth pitch shifting
        let frac = (pos - idx as f64) as f32;
        let sample = samples[idx] * (1.0 - frac) + samples[idx + 1] * frac;

        self.position += self.playback_rate;

        sample
    }

    fn is_finished(&self) -> bool {
        self.finished
    }
}

// ============================================================================
// ADSR Envelope
// ============================================================================

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum AdsrStage {
    Attack,
    Decay,
    Sustain,
    Release,
    Done,
}

#[derive(Clone)]
struct Adsr {
    stage: AdsrStage,
    value: f32,
    attack_rate: f32,
    decay_rate: f32,
    sustain_level: f32,
    release_rate: f32,
}

impl Adsr {
    fn new(params: &ChannelParams, sample_rate: f32) -> Self {
        let attack_samples = (params.attack * sample_rate).max(1.0);
        let decay_samples = (params.decay * sample_rate).max(1.0);
        let release_samples = (params.release * sample_rate).max(1.0);

        Self {
            stage: AdsrStage::Attack,
            value: 0.0,
            attack_rate: 1.0 / attack_samples,
            decay_rate: (1.0 - params.sustain) / decay_samples,
            sustain_level: params.sustain,
            release_rate: params.sustain.max(0.001) / release_samples,
        }
    }

    fn trigger_release(&mut self) {
        if self.stage != AdsrStage::Done {
            self.stage = AdsrStage::Release;
        }
    }

    fn next_sample(&mut self) -> f32 {
        match self.stage {
            AdsrStage::Attack => {
                self.value += self.attack_rate;
                if self.value >= 1.0 {
                    self.value = 1.0;
                    self.stage = AdsrStage::Decay;
                }
            }
            AdsrStage::Decay => {
                self.value -= self.decay_rate;
                if self.value <= self.sustain_level {
                    self.value = self.sustain_level;
                    self.stage = AdsrStage::Sustain;
                }
            }
            AdsrStage::Sustain => {}
            AdsrStage::Release => {
                self.value -= self.release_rate;
                if self.value <= 0.0 {
                    self.value = 0.0;
                    self.stage = AdsrStage::Done;
                }
            }
            AdsrStage::Done => {
                self.value = 0.0;
            }
        }
        self.value
    }

    fn is_done(&self) -> bool {
        self.stage == AdsrStage::Done
    }
}

// ============================================================================
// Biquad Low-Pass Filter
// ============================================================================

#[derive(Clone)]
struct BiquadFilter {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
    last_freq: f32,
    last_q: f32,
}

impl BiquadFilter {
    fn new(cutoff_hz: f32, q: f32, sample_rate: f32) -> Self {
        let mut filter = Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1: 0.0,
            z2: 0.0,
            last_freq: -1.0,
            last_q: -1.0,
        };
        filter.set_params(cutoff_hz, q, sample_rate);
        filter
    }

    fn set_params(&mut self, cutoff_hz: f32, q: f32, sample_rate: f32) {
        if (self.last_freq - cutoff_hz).abs() < 0.1 && (self.last_q - q).abs() < 0.01 {
            return;
        }

        self.last_freq = cutoff_hz;
        self.last_q = q;

        let cutoff = cutoff_hz.clamp(20.0, sample_rate * 0.49);
        let q = q.clamp(0.1, 30.0);

        let omega = 2.0 * std::f32::consts::PI * cutoff / sample_rate;
        let sin_omega = omega.sin();
        let cos_omega = omega.cos();
        let alpha = sin_omega / (2.0 * q);

        let a0 = 1.0 + alpha;
        self.b0 = ((1.0 - cos_omega) / 2.0) / a0;
        self.b1 = (1.0 - cos_omega) / a0;
        self.b2 = ((1.0 - cos_omega) / 2.0) / a0;
        self.a1 = (-2.0 * cos_omega) / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    fn process(&mut self, input: f32) -> f32 {
        let output = self.b0 * input + self.z1;
        self.z1 = self.b1 * input - self.a1 * output + self.z2;
        self.z2 = self.b2 * input - self.a2 * output;
        output
    }
}

// ============================================================================
// Voice Source (either oscillator or sample)
// ============================================================================

#[derive(Clone)]
enum VoiceSource {
    Oscillator(Oscillator),
    Sample(SamplePlayer),
}

impl VoiceSource {
    fn next_sample(&mut self) -> f32 {
        match self {
            VoiceSource::Oscillator(osc) => osc.next_sample(),
            VoiceSource::Sample(player) => player.next_sample(),
        }
    }

    fn is_finished(&self) -> bool {
        match self {
            VoiceSource::Oscillator(_) => false, // Oscillators never finish
            VoiceSource::Sample(player) => player.is_finished(),
        }
    }
}

// ============================================================================
// Voice
// ============================================================================

#[derive(Clone)]
struct Voice {
    note: u8,
    channel: usize,
    velocity: f32,
    source: VoiceSource,
    envelope: Adsr,
    filter: BiquadFilter,
    sample_rate: f32,
}

impl Voice {
    fn new_synth(
        channel: usize,
        note: u8,
        velocity: u8,
        params: &ChannelParams,
        sample_rate: f32,
    ) -> Self {
        let freq = midi_note_to_freq(note);
        Self {
            note,
            channel,
            velocity: velocity as f32 / 127.0,
            source: VoiceSource::Oscillator(Oscillator::new(freq, sample_rate, params.voice)),
            envelope: Adsr::new(params, sample_rate),
            filter: BiquadFilter::new(params.filter_freq, params.filter_q, sample_rate),
            sample_rate,
        }
    }

    fn new_sample(
        channel: usize,
        note: u8,
        velocity: u8,
        params: &ChannelParams,
        sample_data: SampleData,
        sample_rate: f32,
    ) -> Self {
        Self {
            note,
            channel,
            velocity: velocity as f32 / 127.0,
            source: VoiceSource::Sample(SamplePlayer::new(sample_data, note, sample_rate)),
            envelope: Adsr::new(params, sample_rate),
            filter: BiquadFilter::new(params.filter_freq, params.filter_q, sample_rate),
            sample_rate,
        }
    }

    fn process_sample(&mut self, params: &ChannelParams) -> f32 {
        // Check if sample finished playing
        if self.source.is_finished() {
            self.envelope.stage = AdsrStage::Done;
            return 0.0;
        }

        self.filter
            .set_params(params.filter_freq, params.filter_q, self.sample_rate);

        let source_sample = self.source.next_sample();
        let filtered = self.filter.process(source_sample);
        let env = self.envelope.next_sample();

        filtered * env * self.velocity * params.volume
    }

    fn release(&mut self) {
        self.envelope.trigger_release();
    }

    fn is_done(&self) -> bool {
        self.envelope.is_done() || self.source.is_finished()
    }
}

// ============================================================================
// Voice Pool
// ============================================================================

const MAX_VOICES: usize = 64;

struct VoicePool {
    voices: [Option<Voice>; MAX_VOICES],
    sample_rate: f32,
}

impl VoicePool {
    fn new(sample_rate: f32) -> Self {
        Self {
            voices: std::array::from_fn(|_| None),
            sample_rate,
        }
    }

    fn note_on(
        &mut self,
        channel: usize,
        note: u8,
        velocity: u8,
        params: &ChannelParams,
        sample_data: Option<&SampleData>,
    ) {
        // Release existing note on same channel+note
        self.note_off(channel, note);

        // Create appropriate voice type
        let voice = if params.voice == VoiceType::Sample {
            if let Some(data) = sample_data {
                Voice::new_sample(
                    channel,
                    note,
                    velocity,
                    params,
                    data.clone(),
                    self.sample_rate,
                )
            } else {
                // No sample loaded, fall back to sine
                Voice::new_synth(channel, note, velocity, params, self.sample_rate)
            }
        } else {
            Voice::new_synth(channel, note, velocity, params, self.sample_rate)
        };

        // Find free slot
        if let Some(slot) = self.voices.iter_mut().find(|v| v.is_none()) {
            *slot = Some(voice);
        } else {
            // Voice stealing
            let steal_idx = self
                .voices
                .iter()
                .position(|v| v.as_ref().map_or(false, |v| v.envelope.is_done()))
                .or_else(|| {
                    self.voices.iter().position(|v| {
                        v.as_ref()
                            .map_or(false, |v| matches!(v.envelope.stage, AdsrStage::Release))
                    })
                })
                .unwrap_or(0);

            self.voices[steal_idx] = Some(voice);
        }
    }

    fn note_off(&mut self, channel: usize, note: u8) {
        for voice in self.voices.iter_mut().flatten() {
            if voice.channel == channel && voice.note == note {
                voice.release();
            }
        }
    }

    fn process_sample(&mut self, channel_params: &[ChannelParams; 8], master_volume: f32) -> f32 {
        let mut output = 0.0f32;

        for slot in self.voices.iter_mut() {
            if let Some(voice) = slot {
                if voice.channel < 8 {
                    let params = &channel_params[voice.channel];
                    if !params.muted {
                        output += voice.process_sample(params);
                    }
                }

                if voice.is_done() {
                    *slot = None;
                }
            }
        }

        soft_clip(output * master_volume)
    }
}

// ============================================================================
// Command Processor
// ============================================================================

struct CommandProcessor {
    voice_pool: VoicePool,
    channel_params: [ChannelParams; 8],
    channel_samples: [Option<SampleData>; 8],
    master_volume: f32,
}

impl CommandProcessor {
    fn new(sample_rate: f32) -> Self {
        Self {
            voice_pool: VoicePool::new(sample_rate),
            channel_params: std::array::from_fn(|_| ChannelParams::default()),
            channel_samples: std::array::from_fn(|_| None),
            master_volume: 0.75,
        }
    }

    fn process_command(&mut self, cmd: AudioCommand) {
        match cmd {
            AudioCommand::NoteOn {
                channel,
                note,
                velocity,
            } => {
                if channel < 8 {
                    let params = &self.channel_params[channel];
                    let sample_data = self.channel_samples[channel].as_ref();
                    self.voice_pool
                        .note_on(channel, note, velocity, params, sample_data);
                }
            }
            AudioCommand::NoteOff { channel, note } => {
                if channel < 8 {
                    self.voice_pool.note_off(channel, note);
                }
            }
            AudioCommand::UpdateParams { channel, params } => {
                if channel < 8 {
                    self.channel_params[channel] = params;
                }
            }
            AudioCommand::SetMasterVolume(vol) => {
                self.master_volume = vol.clamp(0.0, 1.0);
            }
            AudioCommand::LoadSample {
                channel,
                sample_data,
            } => {
                if channel < 8 {
                    println!(
                        "Loaded sample '{}' on channel {} ({} samples, {} Hz)",
                        sample_data.name,
                        channel + 1,
                        sample_data.samples.len(),
                        sample_data.sample_rate
                    );
                    self.channel_samples[channel] = Some(sample_data);
                }
            }
        }
    }

    fn process_sample(&mut self) -> f32 {
        self.voice_pool
            .process_sample(&self.channel_params, self.master_volume)
    }
}

// ============================================================================
// Audio Thread
// ============================================================================

pub fn start_audio_thread(
    mut ui_rx: Consumer<AudioCommand>,
    mut midi_rx: Consumer<AudioCommand>,
) -> cpal::Stream {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .expect("No output device available");
    let config = device.default_output_config().expect("No default config");
    let sample_rate = config.sample_rate().0 as f32;
    let channels = config.channels() as usize;

    println!("Audio: {:.0} Hz, {} ch", sample_rate, channels);

    let mut processor = CommandProcessor::new(sample_rate);

    let stream = device
        .build_output_stream(
            &config.into(),
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                // Drain commands
                while let Ok(cmd) = ui_rx.pop() {
                    processor.process_command(cmd);
                }
                while let Ok(cmd) = midi_rx.pop() {
                    processor.process_command(cmd);
                }

                // Generate audio
                for frame in data.chunks_mut(channels) {
                    let sample = processor.process_sample();
                    for s in frame.iter_mut() {
                        *s = sample;
                    }
                }
            },
            |err| eprintln!("Audio error: {}", err),
            None,
        )
        .expect("Failed to build output stream");

    stream.play().expect("Failed to start audio stream");
    stream
}

// ============================================================================
// Sample Loading (called from UI thread)
// ============================================================================

pub fn load_sample_from_file(path: &std::path::Path) -> Result<SampleData, String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let file = std::fs::File::open(path).map_err(|e| format!("Failed to open file: {}", e))?;

    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let format_opts = FormatOptions::default();
    let metadata_opts = MetadataOptions::default();
    let decoder_opts = DecoderOptions::default();

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &format_opts, &metadata_opts)
        .map_err(|e| format!("Failed to probe file: {}", e))?;

    let mut format = probed.format;

    let track = format.default_track().ok_or("No audio track found")?;

    let track_id = track.id;
    let sample_rate = track.codec_params.sample_rate.ok_or("No sample rate")?;
    let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(1);

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &decoder_opts)
        .map_err(|e| format!("Failed to create decoder: {}", e))?;

    let mut all_samples: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(e) => return Err(format!("Error reading packet: {}", e)),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(e) => {
                eprintln!("Decode error (skipping): {}", e);
                continue;
            }
        };

        let spec = *decoded.spec();
        let num_frames = decoded.frames();

        let mut sample_buf = SampleBuffer::<f32>::new(num_frames as u64, spec);
        sample_buf.copy_interleaved_ref(decoded);
        let samples = sample_buf.samples();

        // Convert to mono by averaging channels
        if channels > 1 {
            for frame_idx in 0..num_frames {
                let mut sum = 0.0f32;
                for ch in 0..channels {
                    sum += samples[frame_idx * channels + ch];
                }
                all_samples.push(sum / channels as f32);
            }
        } else {
            all_samples.extend_from_slice(samples);
        }
    }

    if all_samples.is_empty() {
        return Err("No audio data decoded".to_string());
    }

    // Normalize
    let max_amp = all_samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);

    if max_amp > 0.0 && max_amp != 1.0 {
        let scale = 1.0 / max_amp;
        for s in all_samples.iter_mut() {
            *s *= scale;
        }
    }

    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    Ok(SampleData {
        samples: Arc::new(all_samples),
        sample_rate,
        name,
    })
}

// ============================================================================
// Utilities
// ============================================================================

fn midi_note_to_freq(note: u8) -> f32 {
    440.0 * 2.0f32.powf((note as f32 - 69.0) / 12.0)
}

fn soft_clip(x: f32) -> f32 {
    x.tanh()
}

pub fn note_name(note: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let octave = (note / 12) as i32 - 1;
    format!("{}{}", NAMES[(note % 12) as usize], octave)
}

