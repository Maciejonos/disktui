use serde::Deserialize;

#[derive(Deserialize, Debug, Default)]
pub struct Config {
    #[serde(default)]
    pub navigation: Navigation,

    #[serde(default)]
    pub disk: DiskKeys,
}

#[derive(Deserialize, Debug)]
pub struct Navigation {
    #[serde(default = "default_scroll_down")]
    pub scroll_down: char,

    #[serde(default = "default_scroll_up")]
    pub scroll_up: char,
}

impl Default for Navigation {
    fn default() -> Self {
        Self {
            scroll_down: 'j',
            scroll_up: 'k',
        }
    }
}

fn default_scroll_down() -> char {
    'j'
}

fn default_scroll_up() -> char {
    'k'
}

#[derive(Deserialize, Debug)]
pub struct DiskKeys {
    #[serde(default = "default_info")]
    pub info: char,

    #[serde(default = "default_format")]
    pub format: char,

    #[serde(default = "default_partition")]
    pub partition: char,

    #[serde(default = "default_mount")]
    pub mount: char,

    #[serde(default = "default_delete")]
    pub delete: char,
}

impl Default for DiskKeys {
    fn default() -> Self {
        Self {
            info: 'i',
            format: 'f',
            partition: 'p',
            mount: 'm',
            delete: 'd',
        }
    }
}

fn default_info() -> char {
    'i'
}

fn default_format() -> char {
    'f'
}

fn default_partition() -> char {
    'p'
}

fn default_mount() -> char {
    'm'
}

fn default_delete() -> char {
    'd'
}

impl Config {
    pub fn new() -> Self {
        let conf_path = dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("disktui")
            .join("config.toml");

        match std::fs::read_to_string(&conf_path) {
            Ok(config_str) => {
                toml::from_str(&config_str).unwrap_or_else(|e| {
                    eprintln!("Warning: Failed to parse config file {:?}: {}", conf_path, e);
                    eprintln!("Using default configuration.");
                    Config::default()
                })
            }
            Err(_) => Config::default(),
        }
    }
}
