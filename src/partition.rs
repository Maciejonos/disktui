use serde::{Deserialize, Serialize};
use crate::utils::format_bytes;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Partition {
    pub name: String,
    pub size: u64,
    pub filesystem: Option<String>,
    pub mount_point: Option<String>,
    pub is_mounted: bool,
    pub label: Option<String>,
    pub used_bytes: Option<u64>,
    pub available_bytes: Option<u64>,
}

impl Partition {
    pub fn size_str(&self) -> String {
        format_bytes(self.size)
    }

    pub fn usage_percentage(&self) -> Option<u8> {
        match (self.used_bytes, self.available_bytes) {
            (Some(used), Some(avail)) => {
                let total = used + avail;
                if total > 0 {
                    Some((used as f64 / total as f64 * 100.0) as u8)
                } else {
                    Some(0)
                }
            }
            _ => None,
        }
    }

    pub fn usage_bar(&self, percentage: u8, filled_char: &str, empty_char: &str, length: u8) -> String {
        let bar_length = length as usize;
        let filled = ((percentage as usize * bar_length) / 100).min(bar_length);
        let empty = bar_length - filled;
        format!("[{}{}] {}%",
            filled_char.repeat(filled),
            empty_char.repeat(empty),
            percentage
        )
    }

    pub fn usage_str(&self, filled_char: &str, empty_char: &str, length: u8) -> String {
        match self.usage_percentage() {
            Some(percentage) => {
                match (self.used_bytes, self.available_bytes) {
                    (Some(used), Some(avail)) => {
                        let total = used + avail;
                        format!("{}/{} {}",
                            format_bytes(used),
                            format_bytes(total),
                            self.usage_bar(percentage, filled_char, empty_char, length)
                        )
                    }
                    _ => "N/A".to_string(),
                }
            }
            None => "N/A".to_string(),
        }
    }
}
