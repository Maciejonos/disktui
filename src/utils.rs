pub fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_000_000_000_000 {
        let val = bytes as f64 / 1_000_000_000_000.0;
        if bytes.is_multiple_of(1_000_000_000_000) {
            format!("{}TB", bytes / 1_000_000_000_000)
        } else {
            format!("{:.1}TB", val)
        }
    } else if bytes >= 1_000_000_000 {
        let val = bytes as f64 / 1_000_000_000.0;
        if bytes.is_multiple_of(1_000_000_000) {
            format!("{}GB", bytes / 1_000_000_000)
        } else {
            format!("{:.1}GB", val)
        }
    } else if bytes >= 1_000_000 {
        let val = bytes as f64 / 1_000_000.0;
        if bytes.is_multiple_of(1_000_000) {
            format!("{}MB", bytes / 1_000_000)
        } else {
            format!("{:.1}MB", val)
        }
    } else if bytes >= 1_000 {
        let val = bytes as f64 / 1_000.0;
        if bytes.is_multiple_of(1_000) {
            format!("{}KB", bytes / 1_000)
        } else {
            format!("{:.1}KB", val)
        }
    } else {
        format!("{}B", bytes)
    }
}
