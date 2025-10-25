use ratatui::style::Color;

#[derive(Debug, Clone)]
pub struct Theme {
    pub focus_border: Color,
    pub normal_border: Color,
    pub highlight_bg: Color,
    pub highlight_fg: Color,
    pub header: Color,
    pub error: Color,
    pub warning: Color,
    pub success: Color,

    pub disk_name_width: u16,
    pub disk_size_width: u16,
    pub disk_type_width: u16,
    pub disk_model_width: u16,
    pub disk_serial_width: u16,

    pub partition_name_width: u16,
    pub partition_size_width: u16,
    pub partition_fs_width: u16,
    pub partition_mount_width: u16,
    pub partition_label_width: u16,
    pub partition_usage_min_width: u16,

    pub error_ttl: u16,
    pub warning_ttl: u16,
    pub info_ttl: u16,

    pub usage_bar_filled: &'static str,
    pub usage_bar_empty: &'static str,
    pub usage_bar_length: u8,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            focus_border: Color::Indexed(2),
            normal_border: Color::Reset,
            highlight_bg: Color::Indexed(8),
            highlight_fg: Color::Reset,
            header: Color::Indexed(3),
            error: Color::Indexed(1),
            warning: Color::Indexed(3),
            success: Color::Indexed(2),

            disk_name_width: 12,
            disk_size_width: 10,
            disk_type_width: 10,
            disk_model_width: 25,
            disk_serial_width: 20,

            partition_name_width: 15,
            partition_size_width: 10,
            partition_fs_width: 12,
            partition_mount_width: 20,
            partition_label_width: 15,
            partition_usage_min_width: 40,

            error_ttl: 5,
            warning_ttl: 3,
            info_ttl: 2,

            usage_bar_filled: "|",
            usage_bar_empty: "-",
            usage_bar_length: 10,
        }
    }
}

impl Theme {
    pub fn new() -> Self {
        Self::default()
    }
}
