use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;
use crate::event::Event;
use crate::notification::{Notification, NotificationLevel};
use crate::partition::Partition;
use crate::utils::format_bytes;

fn validate_device_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(anyhow!("Device name cannot be empty"));
    }

    if name.contains("..") || name.contains('/') {
        return Err(anyhow!("Invalid device name: contains illegal characters"));
    }

    if !name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        return Err(anyhow!("Invalid device name: contains illegal characters"));
    }

    if name.len() > 32 {
        return Err(anyhow!("Invalid device name: too long"));
    }

    Ok(())
}

#[derive(Debug, Clone)]
pub struct BlockDevice {
    pub name: String,
    pub size: u64,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub partitions: Vec<Partition>,
}

#[derive(Debug, Clone)]
pub struct SmartData {
    pub health: String,
    pub temperature: Option<i32>,
    pub power_on_hours: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FilesystemType {
    Ext4,
    Fat32,
    Ntfs,
    Exfat,
    Btrfs,
    Xfs,
}

impl FilesystemType {
    pub fn as_str(&self) -> &str {
        match self {
            FilesystemType::Ext4 => "ext4",
            FilesystemType::Fat32 => "fat32",
            FilesystemType::Ntfs => "ntfs",
            FilesystemType::Exfat => "exfat",
            FilesystemType::Btrfs => "btrfs",
            FilesystemType::Xfs => "xfs",
        }
    }

    pub fn all() -> Vec<FilesystemType> {
        vec![
            FilesystemType::Ext4,
            FilesystemType::Fat32,
            FilesystemType::Ntfs,
            FilesystemType::Exfat,
            FilesystemType::Btrfs,
            FilesystemType::Xfs,
        ]
    }
}

impl std::fmt::Display for FilesystemType {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

fn parse_size(input: &str) -> Result<u64> {
    let input = input.trim().to_uppercase();
    let input = input.trim_end_matches('B');

    let (num_str, unit) = if input.ends_with("TB") || input.ends_with('T') {
        let len = if input.ends_with("TB") { input.len() - 2 } else { input.len() - 1 };
        (&input[..len], 1_000_000_000_000u64)
    } else if input.ends_with("GB") || input.ends_with('G') {
        let len = if input.ends_with("GB") { input.len() - 2 } else { input.len() - 1 };
        (&input[..len], 1_000_000_000u64)
    } else if input.ends_with("MB") || input.ends_with('M') {
        let len = if input.ends_with("MB") { input.len() - 2 } else { input.len() - 1 };
        (&input[..len], 1_000_000u64)
    } else if input.ends_with("KB") || input.ends_with('K') {
        let len = if input.ends_with("KB") { input.len() - 2 } else { input.len() - 1 };
        (&input[..len], 1_000u64)
    } else {
        (&input[..], 1u64)
    };

    let num: f64 = num_str.parse()
        .map_err(|_| anyhow!("Invalid size format. Use format like: 100M, 2.5GB, 1TB"))?;

    Ok((num * unit as f64).round() as u64)
}

async fn get_filesystem_usage(mount_point: &str) -> Option<(u64, u64)> {
    let output = Command::new("df")
        .args(["-B1", mount_point])
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    let lines: Vec<&str> = stdout.lines().collect();

    if lines.len() < 2 {
        return None;
    }

    let parts: Vec<&str> = lines[1].split_whitespace().collect();
    if parts.len() < 4 {
        return None;
    }

    let used = parts[2].parse::<u64>().ok()?;
    let available = parts[3].parse::<u64>().ok()?;

    Some((used, available))
}

pub async fn list_block_devices() -> Result<Vec<BlockDevice>> {
    let output = Command::new("lsblk")
        .args(["-J", "-b", "-o", "NAME,SIZE,TYPE,MODEL,SERIAL,MOUNTPOINT,FSTYPE,LABEL"])
        .output()
        .await
        .context("Failed to execute lsblk")?;

    if !output.status.success() {
        return Err(anyhow!("lsblk failed"));
    }

    let json: Value = serde_json::from_slice(&output.stdout)
        .context("Failed to parse lsblk JSON")?;

    let mut devices = Vec::new();

    if let Some(blockdevices) = json["blockdevices"].as_array() {
        for device in blockdevices {
            let name = device["name"].as_str().unwrap_or("").to_string();
            let dtype = device["type"].as_str().unwrap_or("").to_string();

            if dtype != "disk" {
                continue;
            }

            let size = device["size"].as_u64().unwrap_or(0);
            let model = device["model"].as_str().map(|s| s.trim().to_string());
            let serial = device["serial"].as_str().map(|s| s.trim().to_string());

            let mut partitions = Vec::new();
            if let Some(children) = device["children"].as_array() {
                for part in children {
                    let part_name = part["name"].as_str().unwrap_or("").to_string();
                    let part_size = part["size"].as_u64().unwrap_or(0);
                    let filesystem = part["fstype"].as_str().map(|s| s.to_string());
                    let mount_point = part["mountpoint"].as_str().map(|s| s.to_string());
                    let label = part["label"].as_str().map(|s| s.to_string());

                    let (used_bytes, available_bytes) = if let Some(ref mp) = mount_point {
                        if let Some((used, avail)) = get_filesystem_usage(mp).await {
                            (Some(used), Some(avail))
                        } else {
                            (None, None)
                        }
                    } else {
                        (None, None)
                    };

                    partitions.push(Partition {
                        name: part_name,
                        size: part_size,
                        filesystem,
                        mount_point: mount_point.clone(),
                        is_mounted: mount_point.is_some(),
                        label,
                        used_bytes,
                        available_bytes,
                    });
                }
            } else {
                let disk_fs = device["fstype"].as_str().map(|s| s.to_string());
                let disk_mount = device["mountpoint"].as_str().map(|s| s.to_string());
                let disk_label = device["label"].as_str().map(|s| s.to_string());

                if disk_fs.is_some() || disk_mount.is_some() {
                    let (used_bytes, available_bytes) = if let Some(ref mp) = disk_mount {
                        if let Some((used, avail)) = get_filesystem_usage(mp).await {
                            (Some(used), Some(avail))
                        } else {
                            (None, None)
                        }
                    } else {
                        (None, None)
                    };

                    partitions.push(Partition {
                        name: name.clone(),
                        size,
                        filesystem: disk_fs,
                        mount_point: disk_mount.clone(),
                        is_mounted: disk_mount.is_some(),
                        label: disk_label,
                        used_bytes,
                        available_bytes,
                    });
                }
            }

            devices.push(BlockDevice {
                name,
                size,
                model,
                serial,
                partitions,
            });
        }
    }

    Ok(devices)
}

pub async fn is_mounted(partition: &str) -> Result<bool> {
    let output = Command::new("findmnt")
        .args(["-n", &format!("/dev/{}", partition)])
        .output()
        .await
        .context("Failed to execute findmnt")?;

    Ok(output.status.success())
}

pub async fn mount_partition(partition: &str, sender: &UnboundedSender<Event>) -> Result<()> {
    validate_device_name(partition)?;

    if is_mounted(partition).await? {
        Notification::send(
            format!("{} already mounted", partition),
            NotificationLevel::Warning,
            sender,
        )?;
        return Ok(());
    }

    let mount_point = format!("/mnt/{}", partition);

    Command::new("mkdir")
        .args(["-p", &mount_point])
        .output()
        .await
        .context("Failed to create mount point")?;

    let output = Command::new("mount")
        .args([&format!("/dev/{}", partition), &mount_point])
        .output()
        .await
        .context("Failed to mount partition")?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        Notification::send(
            format!("Mount failed: {}", err),
            NotificationLevel::Error,
            sender,
        )?;
        return Err(anyhow!("Mount failed"));
    }

    Notification::send(
        format!("Mounted {} at {}", partition, mount_point),
        NotificationLevel::Info,
        sender,
    )?;

    Ok(())
}

pub async fn unmount_partition(partition: &str, sender: &UnboundedSender<Event>) -> Result<()> {
    // Validate partition name
    validate_device_name(partition)?;

    if !is_mounted(partition).await? {
        Notification::send(
            format!("{} not mounted", partition),
            NotificationLevel::Warning,
            sender,
        )?;
        return Ok(());
    }

    let output = Command::new("umount")
        .arg(format!("/dev/{}", partition))
        .output()
        .await
        .context("Failed to unmount partition")?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        Notification::send(
            format!("Unmount failed: {}", err),
            NotificationLevel::Error,
            sender,
        )?;
        return Err(anyhow!("Unmount failed"));
    }

    Notification::send(
        format!("Unmounted {}", partition),
        NotificationLevel::Info,
        sender,
    )?;

    Ok(())
}

pub async fn format_whole_disk(
    disk: &str,
    fs_type: FilesystemType,
    sender: UnboundedSender<Event>,
) -> Result<()> {
    validate_device_name(disk)?;

    let cmd = match fs_type {
        FilesystemType::Ext4 => "mkfs.ext4",
        FilesystemType::Fat32 => "mkfs.fat",
        FilesystemType::Ntfs => "mkfs.ntfs",
        FilesystemType::Exfat => "mkfs.exfat",
        FilesystemType::Btrfs => "mkfs.btrfs",
        FilesystemType::Xfs => "mkfs.xfs",
    };

    let check_cmd = Command::new("which")
        .arg(cmd)
        .output()
        .await;

    if check_cmd.is_err() || !check_cmd.unwrap().status.success() {
        Notification::send(
            format!("Formatting tool '{}' not found. Install the required package.", cmd),
            NotificationLevel::Error,
            &sender,
        )?;
        return Err(anyhow!("Command not found: {}", cmd));
    }

    sender.send(Event::StartProgress(format!("Formatting {} as whole disk...", disk)))?;

    let output = Command::new("parted")
        .args(["-s", &format!("/dev/{}", disk), "mklabel", "gpt"])
        .output()
        .await
        .context("Failed to create partition table")?;

    if !output.status.success() {
        sender.send(Event::EndProgress)?;
        let err = String::from_utf8_lossy(&output.stderr);
        Notification::send(
            format!("Failed to create partition table: {}", err),
            NotificationLevel::Error,
            &sender,
        )?;
        return Err(anyhow!("Failed to create partition table"));
    }

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    let output = Command::new("parted")
        .args([
            "-s",
            &format!("/dev/{}", disk),
            "mkpart",
            "primary",
            "0%",
            "100%",
        ])
        .output()
        .await
        .context("Failed to create partition")?;

    if !output.status.success() {
        sender.send(Event::EndProgress)?;
        let err = String::from_utf8_lossy(&output.stderr);
        Notification::send(
            format!("Failed to create partition: {}", err),
            NotificationLevel::Error,
            &sender,
        )?;
        return Err(anyhow!("Failed to create partition"));
    }

    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    let devices = list_block_devices().await?;
    let device = devices.iter().find(|d| d.name == disk);

    if let Some(device) = device {
        if let Some(new_partition) = device.partitions.first() {
            let part_name = new_partition.name.clone();
            let fs_str = fs_type.as_str().to_string();
            format_partition(&part_name, fs_type, sender.clone()).await?;

            sender.send(Event::EndProgress)?;

            Notification::send(
                format!("Formatted {} as whole disk with {}", disk, fs_str),
                NotificationLevel::Info,
                &sender,
            )?;
            return Ok(());
        }
    }

    sender.send(Event::EndProgress)?;
    Err(anyhow!("Failed to find new partition"))
}

pub async fn format_partition(
    partition: &str,
    fs_type: FilesystemType,
    sender: UnboundedSender<Event>,
) -> Result<()> {
    validate_device_name(partition)?;

    if is_mounted(partition).await? {
        Notification::send(
            format!("{} is mounted. Unmount it first (press 'm')", partition),
            NotificationLevel::Error,
            &sender,
        )?;
        return Err(anyhow!("Partition is mounted"));
    }

    let device_path = format!("/dev/{}", partition);

    let (cmd, args): (&str, Vec<&str>) = match fs_type {
        FilesystemType::Ext4 => ("mkfs.ext4", vec!["-F", "-E", "lazy_itable_init=1,lazy_journal_init=1", &device_path]),
        FilesystemType::Fat32 => ("mkfs.fat", vec!["-F", "32", &device_path]),
        FilesystemType::Ntfs => ("mkfs.ntfs", vec!["-f", "-Q", &device_path]),
        FilesystemType::Exfat => ("mkfs.exfat", vec![&device_path]),
        FilesystemType::Btrfs => ("mkfs.btrfs", vec!["-f", &device_path]),
        FilesystemType::Xfs => ("mkfs.xfs", vec!["-f", &device_path]),
    };

    let check_cmd = Command::new("which")
        .arg(cmd)
        .output()
        .await;

    if check_cmd.is_err() || !check_cmd.unwrap().status.success() {
        Notification::send(
            format!("Formatting tool '{}' not found. Install the required package.", cmd),
            NotificationLevel::Error,
            &sender,
        )?;
        return Err(anyhow!("Command not found: {}", cmd));
    }

    sender.send(Event::StartProgress(format!("Formatting {} as {}...", partition, fs_type.as_str())))?;

    let output = match Command::new(cmd)
        .args(&args)
        .output()
        .await
    {
        Ok(output) => output,
        Err(e) => {
            sender.send(Event::EndProgress)?;
            Notification::send(
                format!("Failed to execute {}: {}", cmd, e),
                NotificationLevel::Error,
                &sender,
            )?;
            return Err(anyhow!("Failed to execute {}", cmd));
        }
    };

    sender.send(Event::EndProgress)?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        Notification::send(
            format!("Format failed: {}", err),
            NotificationLevel::Error,
            &sender,
        )?;
        return Err(anyhow!("Format failed"));
    }

    Notification::send(
        format!("Formatted {} as {}", partition, fs_type.as_str()),
        NotificationLevel::Info,
        &sender,
    )?;

    Ok(())
}


pub async fn create_partition_table(disk: &str, table_type: &str, sender: &UnboundedSender<Event>) -> Result<()> {
    validate_device_name(disk)?;

    let output = Command::new("parted")
        .args(["-s", &format!("/dev/{}", disk), "mklabel", table_type])
        .output()
        .await
        .context("Failed to execute parted")?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        Notification::send(
            format!("Create table failed: {}", err),
            NotificationLevel::Error,
            sender,
        )?;
        return Err(anyhow!("Create partition table failed"));
    }

    Notification::send(
        format!("Created {} partition table on {}", table_type, disk),
        NotificationLevel::Info,
        sender,
    )?;

    Ok(())
}

pub async fn create_partition_with_fs(
    disk: &str,
    size_input: &str,
    fs_type: FilesystemType,
    sender: &UnboundedSender<Event>,
) -> Result<()> {
    validate_device_name(disk)?;

    sender.send(Event::StartProgress(format!("Creating partition on {}...", disk)))?;

    let devices = list_block_devices().await?;
    let device = devices.iter().find(|d| d.name == disk);

    if device.is_none() {
        sender.send(Event::EndProgress)?;
        Notification::send(
            format!("Disk {} not found", disk),
            NotificationLevel::Error,
            sender,
        )?;
        return Err(anyhow!("Disk not found"));
    }

    let device = device.unwrap();
    let used_space: u64 = device.partitions.iter().map(|p| p.size).sum();
    let free_space = device.size.saturating_sub(used_space);

    if free_space == 0 {
        sender.send(Event::EndProgress)?;
        Notification::send(
            "No free space available".to_string(),
            NotificationLevel::Error,
            sender,
        )?;
        return Err(anyhow!("No free space"));
    }

    let requested_size = if size_input.trim().is_empty() {
        free_space
    } else {
        parse_size(size_input)?
    };

    if requested_size > free_space {
        sender.send(Event::EndProgress)?;
        Notification::send(
            format!("Requested size exceeds available space ({} > {})",
                format_bytes(requested_size),
                format_bytes(free_space)),
            NotificationLevel::Error,
            sender,
        )?;
        return Err(anyhow!("Size too large"));
    }

    let start_offset = used_space;
    let start_mb = start_offset / 1_000_000;
    let end_offset = start_offset + requested_size;
    let end_mb = end_offset / 1_000_000;

    let output = Command::new("parted")
        .args([
            "-s",
            &format!("/dev/{}", disk),
            "mkpart",
            "primary",
            &format!("{}MB", start_mb),
            &format!("{}MB", end_mb),
        ])
        .output()
        .await
        .context("Failed to execute parted")?;

    if !output.status.success() {
        sender.send(Event::EndProgress)?;
        let err = String::from_utf8_lossy(&output.stderr);

        let error_msg = if err.contains("unrecognised disk label") || err.contains("unrecognized disk label") {
            format!("No partition table on {}. Press 'p' to create one first.", disk)
        } else {
            format!("Create partition failed: {}", err.trim())
        };

        Notification::send(
            error_msg,
            NotificationLevel::Error,
            sender,
        )?;
        return Err(anyhow!("Create partition failed"));
    }

    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    let devices = list_block_devices().await?;
    let device = devices.iter().find(|d| d.name == disk);

    if let Some(device) = device {
        if let Some(new_partition) = device.partitions.last() {
            let part_name = new_partition.name.clone();

            Notification::send(
                format!("Formatting {} as {}...", part_name, fs_type),
                NotificationLevel::Info,
                sender,
            )?;

            format_partition(&part_name, fs_type, sender.clone()).await?;
        }
    }

    sender.send(Event::EndProgress)?;

    Notification::send(
        format!("Created and formatted partition on {}", disk),
        NotificationLevel::Info,
        sender,
    )?;

    Ok(())
}

pub async fn delete_partition(partition: &str, sender: &UnboundedSender<Event>) -> Result<()> {
    validate_device_name(partition)?;

    if is_mounted(partition).await? {
        unmount_partition(partition, sender).await?;
    }

    let (disk, part_num) = if partition.starts_with("nvme") || partition.starts_with("mmcblk") {
        let parts: Vec<&str> = partition.rsplitn(2, 'p').collect();
        if parts.len() == 2 {
            (parts[1], parts[0])
        } else {
            return Err(anyhow!("Invalid partition name format: {}", partition));
        }
    } else {
        let disk = partition.trim_end_matches(|c: char| c.is_numeric());
        let part_num = partition.trim_start_matches(disk);
        (disk, part_num)
    };

    let output = Command::new("parted")
        .args([
            "-s",
            &format!("/dev/{}", disk),
            "rm",
            part_num,
        ])
        .output()
        .await
        .context("Failed to execute parted")?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        Notification::send(
            format!("Delete partition failed: {}", err),
            NotificationLevel::Error,
            sender,
        )?;
        return Err(anyhow!("Delete partition failed"));
    }

    Notification::send(
        format!("Deleted partition {}", partition),
        NotificationLevel::Info,
        sender,
    )?;

    Ok(())
}

pub async fn get_smart_data(disk: &str) -> Result<SmartData> {
    let output = Command::new("smartctl")
        .args(["-H", "-A", &format!("/dev/{}", disk)])
        .output()
        .await;

    if output.is_err() {
        return Ok(SmartData {
            health: "N/A".to_string(),
            temperature: None,
            power_on_hours: None,
        });
    }

    let output = output.unwrap();
    let text = String::from_utf8_lossy(&output.stdout);

    let health = if text.contains("PASSED") {
        "PASSED".to_string()
    } else if text.contains("FAILED") {
        "FAILED".to_string()
    } else {
        "N/A".to_string()
    };

    let temperature = text
        .lines()
        .find(|l| l.contains("Temperature_Celsius") || l.contains("Temperature"))
        .and_then(|l| {
            l.split_whitespace()
                .filter_map(|s| s.parse::<i32>().ok())
                .find(|&n| n > 0 && n < 100)
        });

    let power_on_hours = text
        .lines()
        .find(|l| l.contains("Power_On_Hours"))
        .and_then(|l| {
            l.split_whitespace()
                .filter_map(|s| s.parse::<u64>().ok())
                .find(|&n| n > 0)
        });

    Ok(SmartData {
        health,
        temperature,
        power_on_hours,
    })
}
