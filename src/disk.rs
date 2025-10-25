use crate::operations::{BlockDevice, SmartData};
use crate::utils::format_bytes;

#[derive(Debug, Clone)]
pub struct Disk {
    pub device: BlockDevice,
    pub smart_data: Option<SmartData>,
}

impl Disk {
    pub fn new(device: BlockDevice, smart_data: Option<SmartData>) -> Self {
        Self { device, smart_data }
    }

    pub fn size_str(&self) -> String {
        format_bytes(self.device.size)
    }

    pub fn device_type(&self) -> &str {
        let name = &self.device.name;
        match name {
            n if n.starts_with("nvme") => "NVME",
            n if n.starts_with("sd") => "SSD/HDD",
            n if n.starts_with("mmcblk") => "MMC",
            n if n.starts_with("loop") => "LOOP",
            n if n.starts_with("dm-") => "LVM",
            n if n.starts_with("md") => "RAID",
            n if n.starts_with("vd") => "VIRTIO",
            n if n.starts_with("hd") => "IDE",
            _ => "DISK",
        }
    }
}
