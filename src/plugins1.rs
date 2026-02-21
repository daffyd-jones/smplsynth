use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::path::PathBuf;

use clack_host::prelude::*;
use clack_host::events::event_types::*;

pub type PluginId = String;

// ============================================================================
// Host Implementation
// ============================================================================

pub struct PluginHostShared;

impl<'a> SharedHandler<'a> for PluginHostShared {
    fn request_restart(&self) {
        println!("Plugin requested restart");
    }

    fn request_process(&self) {
        println!("Plugin requested process");
    }

    fn request_callback(&self) {
        println!("Plugin requested callback");
    }
}

pub struct PluginHost;

impl HostHandlers for PluginHost {
    type Shared<'a> = PluginHostShared;
    type MainThread<'a> = ();
    type AudioProcessor<'a> = ();
}

// Plugin instance for audio processing
pub struct PluginInstance {
    pub plugin_id: PluginId,
    pub instance: Arc<Mutex<clack_host::plugin::PluginInstance<PluginHost>>>,
    pub audio_processor: Arc<Mutex<Option<clack_host::plugin::PluginAudioProcessor<PluginHost>>>>,
    pub is_instrument: bool,
}

unsafe impl Send for PluginInstance {}
unsafe impl Sync for PluginInstance {}

impl PluginInstance {
    pub fn new(plugin_id: String, instance: clack_host::plugin::PluginInstance<PluginHost>, is_instrument: bool) -> Self {
        Self {
            plugin_id,
            instance: Arc::new(Mutex::new(instance)),
            audio_processor: Arc::new(Mutex::new(None)),
            is_instrument,
        }
    }

    pub fn send_note_on(&self, channel: u16, note: u8, velocity: f64) {
        if let Ok(mut instance) = self.instance.lock() {
            if let Some(processor) = self.audio_processor.lock().unwrap().as_mut() {
                let note_on = NoteOnEvent::new(
                    0,
                    Pckn::new(channel, 0, 0, note as u32),
                    velocity,
                );
                let input_events = [note_on];
                let _ = processor.process(
                    &[],
                    &mut [],
                    &InputEvents::from_buffer(&input_events),
                    &mut OutputEvents::new(),
                    None,
                );
            }
        }
    }

    pub fn send_note_off(&self, channel: u16, note: u8) {
        if let Ok(mut instance) = self.instance.lock() {
            if let Some(processor) = self.audio_processor.lock().unwrap().as_mut() {
                let note_off = NoteOffEvent::new(
                    0,
                    Pckn::new(channel, 0, 0, note as u32),
                );
                let input_events = [note_off];
                let _ = processor.process(
                    &[],
                    &mut [],
                    &InputEvents::from_buffer(&input_events),
                    &mut OutputEvents::new(),
                );
            }
        }
    }

    pub fn process_audio(&self, inputs: &[&[f32]], outputs: &mut [&mut [f32]], num_frames: usize) {
        if let Some(processor) = self.audio_processor.lock().unwrap().as_mut() {
            let input_ports = if inputs.is_empty() {
                AudioPorts::with_capacity(0, 0)
            } else {
                AudioPorts::with_capacity(inputs.len(), 1)
            };
            
            let output_ports = AudioPorts::with_capacity(outputs.len(), 1);
            
            let input_audio = if inputs.is_empty() {
                input_ports.with_input_buffers([])
            } else {
                input_ports.with_input_buffers([AudioPortBuffer {
                    latency: 0,
                    channels: AudioPortBufferType::f32_input_only(
                        inputs.iter().map(|ch| InputChannel::constant(ch.as_slice()))
                    ),
                }])
            };
            
            let mut output_buffers: Vec<Vec<f32>> = outputs.iter().map(|_| vec![0.0; num_frames]).collect();
            let output_audio = output_ports.with_output_buffers([AudioPortBuffer {
                latency: 0,
                channels: AudioPortBufferType::f32_output_only(
                    output_buffers.iter_mut().map(|buf| buf.as_mut_slice())
                ),
            }]);
            
            let _ = processor.process(
                &input_audio,
                &mut output_audio,
                &InputEvents::new(),
                &mut OutputEvents::new(),
                None,
                None,
            );
            
            // Copy output back
            for (i, output) in outputs.iter_mut().enumerate() {
                if i < output_buffers.len() {
                    output.copy_from_slice(&output_buffers[i]);
                }
            }
        }
    }
}

#[derive(Clone)]
pub struct PluginInfo {
    pub id: PluginId,
    pub name: String,
    pub version: String,
    pub description: String,
    pub is_instrument: bool,
    pub is_effect: bool,
}

pub struct PluginManager {
    plugins: HashMap<PluginId, PluginWrapper>,
    loaded_instances: HashMap<PluginId, Arc<PluginInstance>>,
    host_info: HostInfo,
}

pub struct PluginWrapper {
    pub info: PluginInfo,
    pub path: PathBuf,
}

impl PluginManager {
    pub fn new() -> Self {
        let host_info = HostInfo::new(
            "SMPLSYNTH",
            "SMPLSYNTH",
            "https://github.com/user/smplsynth",
            "0.1.0"
        ).expect("Failed to create host info");
        
        Self {
            plugins: HashMap::new(),
            loaded_instances: HashMap::new(),
            host_info,
        }
    }

    pub fn scan_plugins(&mut self) -> Result<Vec<PluginInfo>, String> {
        let mut discovered_plugins = Vec::new();
        
        // Add test plugins for development
        discovered_plugins.push(PluginInfo {
            id: "test_synth".to_string(),
            name: "Test Synth".to_string(),
            version: "1.0.0".to_string(),
            description: "Test instrument plugin".to_string(),
            is_instrument: true,
            is_effect: false,
        });
        
        discovered_plugins.push(PluginInfo {
            id: "test_reverb".to_string(),
            name: "Test Reverb".to_string(),
            version: "1.0.0".to_string(),
            description: "Test effect plugin".to_string(),
            is_instrument: false,
            is_effect: true,
        });
        
        // Common CLAP plugin paths
        let paths = [
            "/usr/lib/clap",
            "/usr/local/lib/clap",
            "~/.clap",
            "/Library/Audio/Plug-Ins/CLAP",
        ];

        for path in &paths {
            let expanded_path = expand_user(path);
            println!("Checking CLAP path: {}", expanded_path.display());
            if let Ok(canonical_path) = std::fs::canonicalize(&expanded_path) {
                if let Ok(entries) = std::fs::read_dir(canonical_path) {
                    for entry in entries.flatten() {
                        println!("Found plugin file: {}", entry.path().display());
                        if let Ok(plugin_info) = self.scan_plugin_file(entry.path()) {
                            discovered_plugins.push(plugin_info.clone());
                            self.plugins.insert(plugin_info.id.clone(), PluginWrapper {
                                info: plugin_info,
                                path: entry.path(),
                            });
                        }
                    }
                }
            } else {
                println!("CLAP path does not exist: {}", expanded_path.display());
            }
        }

        println!("Discovered {} plugins", discovered_plugins.len());
        Ok(discovered_plugins)
    }

    fn scan_plugin_file(&self, path: std::path::PathBuf) -> Result<PluginInfo, String> {
        // Try to load the plugin to get real info
        let bundle = unsafe {
            match PluginBundle::load(&path) {
                Ok(bundle) => bundle,
                Err(e) => {
                    // Fall back to filename-based detection
                    let filename = path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown");
                    return Ok(PluginInfo {
                        id: filename.to_string(),
                        name: filename.replace(".clap", ""),
                        version: "1.0.0".to_string(),
                        description: "CLAP Plugin".to_string(),
                        is_instrument: filename.to_lowercase().contains("synth") 
                            || filename.to_lowercase().contains("instrument")
                            || (!filename.to_lowercase().contains("fx") && !filename.to_lowercase().contains("effect")),
                        is_effect: filename.to_lowercase().contains("fx") 
                            || filename.to_lowercase().contains("effect")
                            || filename.to_lowercase().contains("reverb")
                            || filename.to_lowercase().contains("delay")
                            || filename.to_lowercase().contains("chorus")
                            || filename.to_lowercase().contains("distortion"),
                    });
                }
            }
        };

        let plugin_factory = bundle.get_plugin_factory()
            .ok_or_else(|| "Plugin has no factory".to_string())?;

        let plugin_descriptor = plugin_factory.plugin_descriptors()
            .next()
            .ok_or_else(|| "No plugin descriptors found".to_string())?;

        let id = plugin_descriptor.id()
            .and_then(|id| std::str::from_utf8(id.to_bytes()).ok())
            .unwrap_or("unknown")
            .to_string();
        
        let name = plugin_descriptor.name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_else(|| "Unknown".into())
            .to_string();
        
        let version = plugin_descriptor.version()
            .map(|v| v.to_string_lossy())
            .unwrap_or_else(|| "1.0.0".into())
            .to_string();
        
        let description = plugin_descriptor.description()
            .map(|d| d.to_string_lossy())
            .unwrap_or_else(|| "CLAP Plugin".into())
            .to_string();

        // Determine if it's an instrument or effect
        let plugin_features = plugin_descriptor.features();
        let is_instrument = plugin_features.iter().any(|f| f.to_bytes() == b"instrument");
        let is_effect = plugin_features.iter().any(|f| f.to_bytes() == b"audio-effect");

        Ok(PluginInfo {
            id,
            name,
            version,
            description,
            is_instrument,
            is_effect,
        })
    }

    pub fn get_available_plugins(&self) -> Vec<&PluginInfo> {
        self.plugins.values().map(|p| &p.info).collect()
    }

    pub fn get_plugin_path(&self, plugin_id: &str) -> Option<PathBuf> {
        self.plugins.get(plugin_id).map(|p| p.path.clone())
    }

    pub fn load_plugin(&mut self, plugin_id: &str, sample_rate: f32) -> Result<Arc<PluginInstance>, String> {
        // Check if already loaded
        if let Some(instance) = self.loaded_instances.get(plugin_id) {
            return Ok(Arc::clone(instance));
        }

        let wrapper = self.plugins.get(plugin_id)
            .ok_or_else(|| format!("Plugin not found: {}", plugin_id))?;

        // Load the plugin bundle
        let bundle = unsafe {
            PluginBundle::load(&wrapper.path)
                .map_err(|e| format!("Failed to load plugin bundle: {}", e))?
        };

        let plugin_factory = bundle.get_plugin_factory()
            .ok_or("Plugin has no factory")?;

        // Find the plugin descriptor
        let plugin_descriptor = plugin_factory.plugin_descriptors()
            .find(|d| d.id().unwrap().to_bytes() == plugin_id.as_bytes())
            .ok_or_else(|| format!("Plugin descriptor not found for: {}", plugin_id))?;

        // Create the plugin instance
        let plugin_instance = PluginInstance::<PluginHost>::new(
            &bundle,
            plugin_descriptor.id().unwrap(),
            &self.host_info,
        ).map_err(|e| format!("Failed to create plugin instance: {}", e))?;

        // Configure audio
        let audio_configuration = PluginAudioConfiguration {
            sample_rate: sample_rate as _,
            min_frames_count: 64,
            max_frames_count: 512,
        };

        // Activate the plugin
        let audio_processor = plugin_instance.activate(|_, _| (), audio_configuration)
            .map_err(|e| format!("Failed to activate plugin: {}", e))?;

        // Create the instance wrapper
        let instance = Arc::new(PluginInstance::new(
            plugin_id.to_string(),
            plugin_instance,
            wrapper.info.is_instrument,
        ));

        // Store the audio processor
        if let Ok(mut instance_lock) = instance.instance.lock() {
            if let Ok(mut audio_lock) = instance.audio_processor.lock() {
                *audio_lock = Some(audio_processor);
            }
        }

        // Store the instance
        self.loaded_instances.insert(plugin_id.to_string(), Arc::clone(&instance));

        Ok(instance)
    }

    pub fn get_loaded_plugin(&self, plugin_id: &str) -> Option<Arc<PluginInstance>> {
        self.loaded_instances.get(plugin_id).cloned()
    }
}

// Helper function to expand user home directory
fn expand_user(path: &str) -> PathBuf {
    if path.starts_with('~') {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(path.replace('~', &home.to_string_lossy()));
        }
    }
    PathBuf::from(path)
}
