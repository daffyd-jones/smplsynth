use std::sync::Arc;
use std::collections::HashMap;
use std::path::PathBuf;

pub type PluginId = String;

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
}

pub struct PluginWrapper {
    pub info: PluginInfo,
    pub path: PathBuf,
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
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
        // For now, just extract basic info from filename
        // In a real implementation, you'd load the CLAP descriptor
        let filename = path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        Ok(PluginInfo {
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
        })
    }

    pub fn get_available_plugins(&self) -> Vec<&PluginInfo> {
        self.plugins.values().map(|p| &p.info).collect()
    }

    pub fn get_plugin_path(&self, plugin_id: &str) -> Option<PathBuf> {
        self.plugins.get(plugin_id).map(|p| p.path.clone())
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
