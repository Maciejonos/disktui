use crate::event::Event;
use crate::notification::{Notification, NotificationLevel};
use crate::partition::Partition;
use crate::utils::format_bytes;
use anyhow::{Context, Result, anyhow};
use serde_json::Value;
use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;

fn validate_device_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(anyhow!("Device name cannot be empty"));
    }

    if name.contains("..") || name.contains('/') {
        return Err(anyhow!("Invalid device name: contains illegal characters"));
    }

    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(anyhow!("Invalid device name: contains illegal characters"));
    }

    if name.len() > 32 {
        return Err(anyhow!("Invalid device name: too long"));
    }

    Ok(())
}

fn get_device_path(device_name: &str) -> String {
    if device_name.starts_with("luks-") {
        let mapper_path = format!("/dev/mapper/{}", device_name);
        if std::path::Path::new(&mapper_path).exists() {
            mapper_path
        } else {
            let base_device = device_name.strip_prefix("luks-").unwrap_or(device_name);
            format!("/dev/{}", base_device)
        }
    } else {
        let mapper_path = format!("/dev/mapper/{}", device_name);
        if std::path::Path::new(&mapper_path).exists() {
            mapper_path
        } else {
            format!("/dev/{}", device_name)
        }
    }
}

async fn wait_for_device(device_path: &str, timeout_secs: u64) -> Result<()> {
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(timeout_secs);

    while start.elapsed() < timeout {
        if std::path::Path::new(device_path).exists() {
            let verify = Command::new("blockdev")
                .args(["--getsize64", device_path])
                .output()
                .await;

            if verify.is_ok() && verify.unwrap().status.success() {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                return Ok(());
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    }

    Err(anyhow!("Timeout waiting for device: {}", device_path))
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

#[derive(Debug, Clone)]
pub struct LuksInfo {
    pub version: String,
    pub uuid: String,
    pub cipher: String,
    pub key_size: String,
}

#[derive(Debug, Clone)]
pub struct LuksStatus {
    pub is_active: bool,
    pub mapper_name: Option<String>,
    pub device_path: Option<String>,
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
        let len = if input.ends_with("TB") {
            input.len() - 2
        } else {
            input.len() - 1
        };
        (&input[..len], 1_000_000_000_000u64)
    } else if input.ends_with("GB") || input.ends_with('G') {
        let len = if input.ends_with("GB") {
            input.len() - 2
        } else {
            input.len() - 1
        };
        (&input[..len], 1_000_000_000u64)
    } else if input.ends_with("MB") || input.ends_with('M') {
        let len = if input.ends_with("MB") {
            input.len() - 2
        } else {
            input.len() - 1
        };
        (&input[..len], 1_000_000u64)
    } else if input.ends_with("KB") || input.ends_with('K') {
        let len = if input.ends_with("KB") {
            input.len() - 2
        } else {
            input.len() - 1
        };
        (&input[..len], 1_000u64)
    } else {
        (&input[..], 1u64)
    };

    let num: f64 = num_str
        .parse()
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

async fn get_mapper_mount_point(mapper_name: &str, fallback: Option<String>) -> Option<String> {
    let mapper_mount_check = Command::new("findmnt")
        .args(["-n", "-o", "TARGET", &format!("/dev/mapper/{}", mapper_name)])
        .output()
        .await;

    if let Ok(output) = mapper_mount_check {
        if output.status.success() {
            let mount_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !mount_str.is_empty() {
                return Some(mount_str);
            }
        }
    }
    fallback
}

pub async fn list_block_devices() -> Result<Vec<BlockDevice>> {
    let output = Command::new("lsblk")
        .args([
            "-J",
            "-b",
            "-o",
            "NAME,SIZE,TYPE,MODEL,SERIAL,MOUNTPOINT,FSTYPE,LABEL",
        ])
        .output()
        .await
        .context("Failed to execute lsblk")?;

    if !output.status.success() {
        return Err(anyhow!("lsblk failed"));
    }

    let json: Value =
        serde_json::from_slice(&output.stdout).context("Failed to parse lsblk JSON")?;

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

                    let is_encrypted = is_luks_device(&part_name).await.unwrap_or(false);
                    let (encryption_type, luks_uuid, mapper_device) = if is_encrypted {
                        let luks_info = get_luks_info(&part_name).await.ok();
                        let luks_status = get_luks_status(&part_name).await.ok();
                        (
                            luks_info.as_ref().map(|info| info.version.clone()),
                            luks_info.as_ref().map(|info| info.uuid.clone()),
                            luks_status.and_then(|status| status.mapper_name),
                        )
                    } else {
                        (None, None, None)
                    };

                    let actual_mount_point = if let Some(ref mapper_name) = mapper_device {
                        get_mapper_mount_point(mapper_name, mount_point.clone()).await
                    } else {
                        mount_point.clone()
                    };

                    let (used_bytes, available_bytes) = if let Some(ref mp) = actual_mount_point {
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
                        mount_point: actual_mount_point.clone(),
                        is_mounted: actual_mount_point.is_some(),
                        label,
                        used_bytes,
                        available_bytes,
                        is_encrypted,
                        encryption_type,
                        luks_uuid,
                        mapper_device,
                    });
                }
            } else {
                let disk_fs = device["fstype"].as_str().map(|s| s.to_string());
                let disk_mount = device["mountpoint"].as_str().map(|s| s.to_string());
                let disk_label = device["label"].as_str().map(|s| s.to_string());

                if disk_fs.is_some() || disk_mount.is_some() {
                    let is_encrypted = is_luks_device(&name).await.unwrap_or(false);
                    let (encryption_type, luks_uuid, mapper_device) = if is_encrypted {
                        let luks_info = get_luks_info(&name).await.ok();
                        let luks_status = get_luks_status(&name).await.ok();
                        (
                            luks_info.as_ref().map(|info| info.version.clone()),
                            luks_info.as_ref().map(|info| info.uuid.clone()),
                            luks_status.and_then(|status| status.mapper_name),
                        )
                    } else {
                        (None, None, None)
                    };

                    let actual_mount_point = if let Some(ref mapper_name) = mapper_device {
                        get_mapper_mount_point(mapper_name, disk_mount.clone()).await
                    } else {
                        disk_mount.clone()
                    };

                    let (used_bytes, available_bytes) = if let Some(ref mp) = actual_mount_point {
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
                        mount_point: actual_mount_point.clone(),
                        is_mounted: actual_mount_point.is_some(),
                        label: disk_label,
                        used_bytes,
                        available_bytes,
                        is_encrypted,
                        encryption_type,
                        luks_uuid,
                        mapper_device,
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
    let device_path = get_device_path(partition);
    let output = Command::new("findmnt")
        .args(["-n", &device_path])
        .output()
        .await
        .context("Failed to execute findmnt")?;

    Ok(output.status.success())
}

pub async fn mount_partition(partition: &str, sender: &UnboundedSender<Event>) -> Result<()> {
    validate_device_name(partition)?;

    let is_luks = is_luks_device(partition).await.unwrap_or(false);
    if is_luks {
        let luks_status = get_luks_status(partition).await?;
        if luks_status.is_active {
            if let Some(mapper_name) = luks_status.mapper_name {
                Notification::send(
                    format!(
                        "{} is an unlocked encrypted device. Mount the mapper device instead: {}",
                        partition, mapper_name
                    ),
                    NotificationLevel::Error,
                    sender,
                )?;
                return Err(anyhow!(
                    "Cannot mount base device of unlocked LUKS partition. Use mapper device."
                ));
            }
        } else {
            Notification::send(
                format!(
                    "{} is a locked encrypted device. Unlock it first (press 'l').",
                    partition
                ),
                NotificationLevel::Error,
                sender,
            )?;
            return Err(anyhow!("Cannot mount locked encrypted device"));
        }
    }

    if is_mounted(partition).await? {
        Notification::send(
            format!("{} already mounted", partition),
            NotificationLevel::Warning,
            sender,
        )?;
        return Ok(());
    }

    let device_path = get_device_path(partition);

    if !std::path::Path::new(&device_path).exists() {
        Notification::send(
            format!(
                "Device {} does not exist. If this is a LUKS device, ensure it is unlocked first.",
                device_path
            ),
            NotificationLevel::Error,
            sender,
        )?;
        return Err(anyhow!("Device does not exist: {}", device_path));
    }

    let mount_point = format!("/mnt/{}", partition);

    Command::new("mkdir")
        .args(["-p", &mount_point])
        .output()
        .await
        .context("Failed to create mount point")?;

    let output = Command::new("mount")
        .args([&device_path, &mount_point])
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
    validate_device_name(partition)?;

    let is_luks = is_luks_device(partition).await.unwrap_or(false);
    if is_luks {
        let luks_status = get_luks_status(partition).await?;
        if luks_status.is_active {
            Notification::send(
                format!(
                    "{} is an encrypted device that is unlocked. Lock it first instead of unmounting.",
                    partition
                ),
                NotificationLevel::Error,
                sender,
            )?;
            return Err(anyhow!(
                "Cannot unmount unlocked encrypted device directly. Lock it first."
            ));
        }
    }

    if !is_mounted(partition).await? {
        Notification::send(
            format!("{} not mounted", partition),
            NotificationLevel::Warning,
            sender,
        )?;
        return Ok(());
    }

    let device_path = get_device_path(partition);

    if !std::path::Path::new(&device_path).exists() {
        Notification::send(
            format!(
                "Device {} does not exist. If this is a LUKS device, ensure it is unlocked first.",
                device_path
            ),
            NotificationLevel::Error,
            sender,
        )?;
        return Err(anyhow!("Device does not exist: {}", device_path));
    }

    sender.send(Event::StartProgress(format!("Unmounting {}...", partition)))?;

    let unmount_future = Command::new("umount").arg(&device_path).output();

    let output =
        match tokio::time::timeout(tokio::time::Duration::from_secs(5), unmount_future).await {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                sender.send(Event::EndProgress)?;
                Notification::send(
                    format!("Failed to unmount: {}", e),
                    NotificationLevel::Error,
                    sender,
                )?;
                return Err(anyhow!("Failed to execute unmount"));
            }
            Err(_) => {
                sender.send(Event::EndProgress)?;
                Notification::send(
                    format!("Device is busy. Attempting lazy unmount..."),
                    NotificationLevel::Info,
                    sender,
                )?;

                sender.send(Event::StartProgress(format!(
                    "Lazy unmounting {}...",
                    partition
                )))?;

                let lazy_output = Command::new("umount")
                    .args(["-l", &device_path])
                    .output()
                    .await
                    .context("Failed to lazy unmount")?;

                sender.send(Event::EndProgress)?;

                if !lazy_output.status.success() {
                    let err = String::from_utf8_lossy(&lazy_output.stderr);
                    Notification::send(
                        format!("Lazy unmount failed: {}", err),
                        NotificationLevel::Error,
                        sender,
                    )?;
                    return Err(anyhow!("Lazy unmount failed"));
                }

                let mount_point = format!("/mnt/{}", partition);
                let _ = Command::new("rmdir").arg(&mount_point).output().await;

                Notification::send(
                    format!(
                        "Lazy unmounted {} (will complete when no longer in use)",
                        partition
                    ),
                    NotificationLevel::Info,
                    sender,
                )?;
                return Ok(());
            }
        };

    sender.send(Event::EndProgress)?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);

        if err.contains("target is busy") || err.contains("device is busy") {
            Notification::send(
                format!("Device is busy. Attempting lazy unmount..."),
                NotificationLevel::Info,
                sender,
            )?;

            sender.send(Event::StartProgress(format!(
                "Lazy unmounting {}...",
                partition
            )))?;

            let lazy_output = Command::new("umount")
                .args(["-l", &device_path])
                .output()
                .await
                .context("Failed to lazy unmount")?;

            sender.send(Event::EndProgress)?;

            if !lazy_output.status.success() {
                let err = String::from_utf8_lossy(&lazy_output.stderr);
                Notification::send(
                    format!("Lazy unmount failed: {}", err),
                    NotificationLevel::Error,
                    sender,
                )?;
                return Err(anyhow!("Lazy unmount failed"));
            }

            let mount_point = format!("/mnt/{}", partition);
            let _ = Command::new("rmdir").arg(&mount_point).output().await;

            Notification::send(
                format!(
                    "Lazy unmounted {} (will complete when no longer in use)",
                    partition
                ),
                NotificationLevel::Info,
                sender,
            )?;
            return Ok(());
        }

        Notification::send(
            format!("Unmount failed: {}", err),
            NotificationLevel::Error,
            sender,
        )?;
        return Err(anyhow!("Unmount failed"));
    }

    let mount_point = format!("/mnt/{}", partition);
    let _ = Command::new("rmdir").arg(&mount_point).output().await;

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

    let devices = list_block_devices().await?;
    if let Some(device) = devices.iter().find(|d| d.name == disk) {
        for partition in &device.partitions {
            let luks_status = get_luks_status(&partition.name).await?;
            if luks_status.is_active {
                if let Some(mapper_name) = luks_status.mapper_name {
                    let mapper_path = format!("/dev/mapper/{}", mapper_name);
                    let mapper_mounted = Command::new("findmnt")
                        .args(["-n", &mapper_path])
                        .output()
                        .await
                        .map(|output| output.status.success())
                        .unwrap_or(false);

                    if mapper_mounted {
                        unmount_partition(&mapper_name, &sender).await?;
                    }

                    Notification::send(
                        format!("Closing encrypted device {}...", mapper_name),
                        NotificationLevel::Info,
                        &sender,
                    )?;
                    lock_luks_device(&mapper_name, &sender).await?;
                }
            }
        }
    }

    let cmd = match fs_type {
        FilesystemType::Ext4 => "mkfs.ext4",
        FilesystemType::Fat32 => "mkfs.fat",
        FilesystemType::Ntfs => "mkfs.ntfs",
        FilesystemType::Exfat => "mkfs.exfat",
        FilesystemType::Btrfs => "mkfs.btrfs",
        FilesystemType::Xfs => "mkfs.xfs",
    };

    let check_cmd = Command::new("which").arg(cmd).output().await;

    if check_cmd.is_err() || !check_cmd.unwrap().status.success() {
        Notification::send(
            format!(
                "Formatting tool '{}' not found. Install the required package.",
                cmd
            ),
            NotificationLevel::Error,
            &sender,
        )?;
        return Err(anyhow!("Command not found: {}", cmd));
    }

    sender.send(Event::StartProgress(format!(
        "Formatting {} as whole disk...",
        disk
    )))?;

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

    let _ = Command::new("partprobe")
        .arg(&format!("/dev/{}", disk))
        .output()
        .await;

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

    let is_luks = is_luks_device(partition).await.unwrap_or(false);
    let actual_device = if is_luks {
        let luks_status = get_luks_status(partition).await?;
        if luks_status.is_active {
            if let Some(mapper_name) = luks_status.mapper_name {
                Notification::send(
                    format!(
                        "{} is encrypted and unlocked. Formatting the encrypted filesystem at {}...",
                        partition, mapper_name
                    ),
                    NotificationLevel::Info,
                    &sender,
                )?;
                mapper_name
            } else {
                return Err(anyhow!("LUKS device is active but mapper name not found"));
            }
        } else {
            Notification::send(
                format!(
                    "{} is encrypted and locked. Unlock it first (press 'l') to format the encrypted filesystem.",
                    partition
                ),
                NotificationLevel::Error,
                &sender,
            )?;
            return Err(anyhow!(
                "Cannot format locked LUKS device - unlock it first"
            ));
        }
    } else {
        partition.to_string()
    };

    if is_mounted(&actual_device).await? {
        Notification::send(
            format!("{} is mounted. Unmount it first (press 'm')", actual_device),
            NotificationLevel::Error,
            &sender,
        )?;
        return Err(anyhow!("Partition is mounted"));
    }

    let device_path = get_device_path(&actual_device);

    if !std::path::Path::new(&device_path).exists() {
        Notification::send(
            format!(
                "Device {} does not exist. If this is a LUKS device, ensure it is unlocked first.",
                device_path
            ),
            NotificationLevel::Error,
            &sender,
        )?;
        return Err(anyhow!("Device does not exist: {}", device_path));
    }

    let (cmd, args): (&str, Vec<&str>) = match fs_type {
        FilesystemType::Ext4 => (
            "mkfs.ext4",
            vec![
                "-F",
                "-E",
                "lazy_itable_init=1,lazy_journal_init=1",
                &device_path,
            ],
        ),
        FilesystemType::Fat32 => ("mkfs.fat", vec!["-F", "32", &device_path]),
        FilesystemType::Ntfs => ("mkfs.ntfs", vec!["-f", "-Q", &device_path]),
        FilesystemType::Exfat => ("mkfs.exfat", vec![&device_path]),
        FilesystemType::Btrfs => ("mkfs.btrfs", vec!["-f", &device_path]),
        FilesystemType::Xfs => ("mkfs.xfs", vec!["-f", &device_path]),
    };

    let check_cmd = Command::new("which").arg(cmd).output().await;

    if check_cmd.is_err() || !check_cmd.unwrap().status.success() {
        Notification::send(
            format!(
                "Formatting tool '{}' not found. Install the required package.",
                cmd
            ),
            NotificationLevel::Error,
            &sender,
        )?;
        return Err(anyhow!("Command not found: {}", cmd));
    }

    sender.send(Event::StartProgress(format!(
        "Formatting {} as {}...",
        actual_device,
        fs_type.as_str()
    )))?;

    let output = match Command::new(cmd).args(&args).output().await {
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
        format!("Formatted {} as {}", actual_device, fs_type.as_str()),
        NotificationLevel::Info,
        &sender,
    )?;

    Ok(())
}

pub async fn create_partition_table(
    disk: &str,
    table_type: &str,
    sender: &UnboundedSender<Event>,
) -> Result<()> {
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

async fn create_partition_raw(
    disk: &str,
    size_input: &str,
    sender: &UnboundedSender<Event>,
) -> Result<String> {
    validate_device_name(disk)?;

    let devices = list_block_devices().await?;
    let device = devices.iter().find(|d| d.name == disk);

    if device.is_none() {
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
        Notification::send(
            format!(
                "Requested size exceeds available space ({} > {})",
                format_bytes(requested_size),
                format_bytes(free_space)
            ),
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
        let err = String::from_utf8_lossy(&output.stderr);
        let error_msg =
            if err.contains("unrecognised disk label") || err.contains("unrecognized disk label") {
                format!(
                    "No partition table on {}. Press 'p' to create one first.",
                    disk
                )
            } else {
                format!("Create partition failed: {}", err.trim())
            };

        Notification::send(error_msg, NotificationLevel::Error, sender)?;
        return Err(anyhow!("Create partition failed"));
    }

    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    let devices = list_block_devices().await?;
    let device = devices.iter().find(|d| d.name == disk);

    if let Some(device) = device {
        if let Some(new_partition) = device.partitions.last() {
            return Ok(new_partition.name.clone());
        }
    }

    Err(anyhow!("Failed to find new partition"))
}

pub async fn create_partition_with_fs(
    disk: &str,
    size_input: &str,
    fs_type: FilesystemType,
    sender: &UnboundedSender<Event>,
) -> Result<()> {
    sender.send(Event::StartProgress(format!(
        "Creating partition on {}...",
        disk
    )))?;

    let part_name = create_partition_raw(disk, size_input, sender).await?;

    Notification::send(
        format!("Formatting {} as {}...", part_name, fs_type),
        NotificationLevel::Info,
        sender,
    )?;

    format_partition(&part_name, fs_type, sender.clone()).await?;

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

    let is_luks = is_luks_device(partition).await.unwrap_or(false);

    let luks_status = get_luks_status(partition).await?;
    if luks_status.is_active {
        if let Some(mapper_name) = luks_status.mapper_name {
            let mapper_path = format!("/dev/mapper/{}", mapper_name);
            let mapper_mounted = Command::new("findmnt")
                .args(["-n", &mapper_path])
                .output()
                .await
                .map(|output| output.status.success())
                .unwrap_or(false);

            if mapper_mounted {
                unmount_partition(&mapper_name, sender).await?;
            }

            Notification::send(
                format!("Closing encrypted device {}...", mapper_name),
                NotificationLevel::Info,
                sender,
            )?;
            lock_luks_device(&mapper_name, sender).await?;
        }
    }

    if is_mounted(partition).await? {
        unmount_partition(partition, sender).await?;
    }

    if is_luks {
        Notification::send(
            format!("Wiping LUKS header from {}...", partition),
            NotificationLevel::Info,
            sender,
        )?;

        let device_path = format!("/dev/{}", partition);
        let wipe_output = Command::new("wipefs")
            .args(["-a", &device_path])
            .output()
            .await;

        match wipe_output {
            Ok(output) if output.status.success() => {
                Notification::send(
                    format!("LUKS header wiped from {}", partition),
                    NotificationLevel::Info,
                    sender,
                )?;
            }
            Ok(output) => {
                let err = String::from_utf8_lossy(&output.stderr);
                Notification::send(
                    format!("Warning: Failed to wipe LUKS header: {}. Continuing with deletion...", err),
                    NotificationLevel::Warning,
                    sender,
                )?;
            }
            Err(_) => {
                Notification::send(
                    "Warning: wipefs not available. Continuing with deletion...".to_string(),
                    NotificationLevel::Warning,
                    sender,
                )?;
            }
        }
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

    Notification::send(
        format!("Deleting partition {}...", partition),
        NotificationLevel::Info,
        sender,
    )?;

    let output = Command::new("parted")
        .args(["-s", &format!("/dev/{}", disk), "rm", part_num])
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
        return Err(anyhow!("Delete partition failed: {}", err));
    }

    let partprobe_output = Command::new("partprobe")
        .arg(&format!("/dev/{}", disk))
        .output()
        .await;

    if let Ok(output) = partprobe_output {
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            Notification::send(
                format!("Warning: partprobe failed: {}. Partition deleted but you may need to reboot.", err),
                NotificationLevel::Warning,
                sender,
            )?;
        }
    }

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    Notification::send(
        format!("Successfully deleted partition {}", partition),
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

pub async fn resize_partition_and_filesystem(
    partition: &str,
    new_size_input: &str,
    sender: &UnboundedSender<Event>,
) -> Result<()> {
    validate_device_name(partition)?;

    let is_luks = is_luks_device(partition).await.unwrap_or(false);
    if is_luks {
        Notification::send(
            format!(
                "{} is an encrypted partition. Resizing encrypted partitions is not supported as it risks data corruption. To resize: backup data, delete partition, create larger partition, restore data.",
                partition
            ),
            NotificationLevel::Error,
            sender,
        )?;
        return Err(anyhow!(
            "Cannot resize encrypted partitions - too risky for data integrity"
        ));
    }

    if is_mounted(partition).await? {
        Notification::send(
            format!("{} is mounted. Unmount it first (press 'm')", partition),
            NotificationLevel::Error,
            sender,
        )?;
        return Err(anyhow!("Partition is mounted"));
    }

    sender.send(Event::StartProgress(format!("Resizing {}...", partition)))?;

    let (disk, part_num) = if partition.starts_with("nvme") || partition.starts_with("mmcblk") {
        let parts: Vec<&str> = partition.rsplitn(2, 'p').collect();
        if parts.len() == 2 {
            (parts[1].to_string(), parts[0].parse::<usize>()?)
        } else {
            sender.send(Event::EndProgress)?;
            return Err(anyhow!("Invalid partition name format: {}", partition));
        }
    } else {
        let disk = partition.trim_end_matches(|c: char| c.is_numeric());
        let part_num_str = partition.trim_start_matches(disk);
        (disk.to_string(), part_num_str.parse::<usize>()?)
    };

    let new_size_bytes = parse_size(new_size_input)?;

    let devices = list_block_devices().await?;
    let device = devices
        .iter()
        .find(|d| d.name == disk)
        .ok_or_else(|| anyhow!("Disk {} not found", disk))?;

    let current_partition = device
        .partitions
        .iter()
        .find(|p| p.name == partition)
        .ok_or_else(|| anyhow!("Partition {} not found", partition))?;

    let current_size = current_partition.size;
    let filesystem = current_partition.filesystem.clone();

    let is_growing = new_size_bytes > current_size;

    if !is_growing {
        Notification::send(
            "Shrinking filesystem...".to_string(),
            NotificationLevel::Info,
            sender,
        )?;
        resize_filesystem(partition, &filesystem, new_size_bytes, false, sender).await?;
    }

    Notification::send(
        "Resizing partition...".to_string(),
        NotificationLevel::Info,
        sender,
    )?;

    let output = Command::new("sfdisk")
        .args(["-d", &format!("/dev/{}", disk)])
        .output()
        .await
        .context("Failed to dump partition table")?;

    if !output.status.success() {
        sender.send(Event::EndProgress)?;
        let err = String::from_utf8_lossy(&output.stderr);
        Notification::send(
            format!("Failed to read partition table: {}", err),
            NotificationLevel::Error,
            sender,
        )?;
        return Err(anyhow!("Failed to read partition table"));
    }

    let table = String::from_utf8_lossy(&output.stdout);
    let mut new_table = String::new();
    let mut found = false;

    for line in table.lines() {
        if line.contains(&format!("/dev/{}{}", disk, part_num))
            || line.contains(&format!("/dev/{}p{}", disk, part_num))
        {
            found = true;

            let parts: Vec<&str> = line.split(&[':', ','][..]).collect();

            if parts.is_empty() {
                sender.send(Event::EndProgress)?;
                return Err(anyhow!("Invalid partition table format"));
            }

            let device_part = parts[0].trim();

            let expected_dev = format!("/dev/{}{}", disk, part_num);
            let expected_dev_p = format!("/dev/{}p{}", disk, part_num);
            if device_part != expected_dev && device_part != expected_dev_p {
                new_table.push_str(line);
                new_table.push('\n');
                continue;
            }

            let mut start_str = String::new();
            let mut other_attrs = Vec::new();

            for part in parts.iter().skip(1) {
                let trimmed = part.trim();
                if trimmed.starts_with("start") {
                    start_str = trimmed.to_string();
                } else if trimmed.starts_with("size") {
                    continue;
                } else if !trimmed.is_empty() {
                    other_attrs.push(trimmed.to_string());
                }
            }

            if start_str.is_empty() {
                sender.send(Event::EndProgress)?;
                Notification::send(
                    "Could not parse partition table".to_string(),
                    NotificationLevel::Error,
                    sender,
                )?;
                return Err(anyhow!("Could not find start sector"));
            }

            let size_sectors = (new_size_bytes + 511) / 512;

            let mut new_line = format!("{} : {}, size={}", device_part, start_str, size_sectors);
            for attr in other_attrs {
                new_line.push_str(", ");
                new_line.push_str(&attr);
            }

            new_table.push_str(&new_line);
            new_table.push('\n');
        } else {
            new_table.push_str(line);
            new_table.push('\n');
        }
    }

    if !found {
        sender.send(Event::EndProgress)?;
        Notification::send(
            format!("Partition {} not found in partition table", partition),
            NotificationLevel::Error,
            sender,
        )?;
        return Err(anyhow!("Partition not found in table"));
    }

    let mut child = Command::new("sfdisk")
        .args(["--force", "--no-reread", &format!("/dev/{}", disk)])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("Failed to spawn sfdisk")?;

    {
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin.write_all(new_table.as_bytes()).await?;
            stdin.flush().await?;
            drop(stdin);
        }
    }

    let output = child.wait_with_output().await?;

    if !output.status.success() {
        sender.send(Event::EndProgress)?;
        let err = String::from_utf8_lossy(&output.stderr);
        Notification::send(
            format!("Failed to resize partition: {}", err),
            NotificationLevel::Error,
            sender,
        )?;
        return Err(anyhow!("Failed to resize partition"));
    }

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    let _ = Command::new("partprobe")
        .arg(&format!("/dev/{}", disk))
        .output()
        .await;
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    if is_growing {
        Notification::send(
            "Expanding filesystem...".to_string(),
            NotificationLevel::Info,
            sender,
        )?;
        resize_filesystem(partition, &filesystem, new_size_bytes, true, sender).await?;
    }

    sender.send(Event::EndProgress)?;
    Notification::send(
        format!(
            "Successfully resized {} to {}",
            partition,
            format_bytes(new_size_bytes)
        ),
        NotificationLevel::Info,
        sender,
    )?;

    Ok(())
}

async fn resize_filesystem(
    partition: &str,
    filesystem: &Option<String>,
    new_size_bytes: u64,
    is_growing: bool,
    sender: &UnboundedSender<Event>,
) -> Result<()> {
    let fs = match filesystem {
        Some(fs) => fs.as_str(),
        None => {
            Notification::send(
                "No filesystem detected, skipping filesystem resize".to_string(),
                NotificationLevel::Warning,
                sender,
            )?;
            return Ok(());
        }
    };

    let device_path = format!("/dev/{}", partition);

    match fs {
        "ext4" | "ext3" | "ext2" => {
            let output = if is_growing {
                Command::new("resize2fs")
                    .arg(&device_path)
                    .output()
                    .await
                    .context("Failed to execute resize2fs")?
            } else {
                let size_k = format!("{}K", new_size_bytes / 1024);
                Command::new("resize2fs")
                    .args([&device_path, &size_k])
                    .output()
                    .await
                    .context("Failed to execute resize2fs")?
            };

            if !output.status.success() {
                let err = String::from_utf8_lossy(&output.stderr);
                Notification::send(
                    format!("Filesystem resize failed: {}", err),
                    NotificationLevel::Error,
                    sender,
                )?;
                return Err(anyhow!("resize2fs failed"));
            }
        }
        "xfs" => {
            if !is_growing {
                Notification::send(
                    "XFS does not support shrinking".to_string(),
                    NotificationLevel::Error,
                    sender,
                )?;
                return Err(anyhow!("XFS cannot be shrunk"));
            }

            let mount_point = format!("/tmp/disktui_resize_{}", partition.replace('/', "_"));
            Command::new("mkdir")
                .args(["-p", &mount_point])
                .output()
                .await?;

            let mount_output = Command::new("mount")
                .args([&device_path, &mount_point])
                .output()
                .await?;

            if mount_output.status.success() {
                let resize_output = Command::new("xfs_growfs").arg(&mount_point).output().await;

                let _ = Command::new("umount").arg(&mount_point).output().await;

                let _ = Command::new("rmdir").arg(&mount_point).output().await;

                if let Ok(output) = resize_output {
                    if !output.status.success() {
                        let err = String::from_utf8_lossy(&output.stderr);
                        Notification::send(
                            format!("XFS resize failed: {}", err),
                            NotificationLevel::Error,
                            sender,
                        )?;
                        return Err(anyhow!("xfs_growfs failed"));
                    }
                } else {
                    Notification::send(
                        "Failed to resize XFS filesystem".to_string(),
                        NotificationLevel::Error,
                        sender,
                    )?;
                    return Err(anyhow!("xfs_growfs failed"));
                }
            } else {
                let err = String::from_utf8_lossy(&mount_output.stderr);
                let _ = Command::new("rmdir").arg(&mount_point).output().await;
                Notification::send(
                    format!("Failed to mount for XFS resize: {}", err),
                    NotificationLevel::Error,
                    sender,
                )?;
                return Err(anyhow!("Mount failed"));
            }
        }
        "ntfs" => {
            let output = if is_growing {
                Command::new("ntfsresize")
                    .args(["-f", &device_path])
                    .output()
                    .await
            } else {
                let size_str = new_size_bytes.to_string();
                Command::new("ntfsresize")
                    .args(["-f", "-s", &size_str, &device_path])
                    .output()
                    .await
            };

            if let Ok(output) = output {
                if !output.status.success() {
                    let err = String::from_utf8_lossy(&output.stderr);
                    Notification::send(
                        format!("NTFS resize failed: {}", err),
                        NotificationLevel::Error,
                        sender,
                    )?;
                    return Err(anyhow!("ntfsresize failed"));
                }
            } else {
                Notification::send(
                    "ntfsresize not found. Install ntfs-3g package.".to_string(),
                    NotificationLevel::Error,
                    sender,
                )?;
                return Err(anyhow!("ntfsresize not found"));
            }
        }
        "btrfs" => {
            let mount_point = format!("/tmp/disktui_resize_{}", partition.replace('/', "_"));
            Command::new("mkdir")
                .args(["-p", &mount_point])
                .output()
                .await?;

            let mount_output = Command::new("mount")
                .args([&device_path, &mount_point])
                .output()
                .await?;

            if mount_output.status.success() {
                let size_arg = if is_growing {
                    "max".to_string()
                } else {
                    new_size_bytes.to_string()
                };

                let resize_output = Command::new("btrfs")
                    .args(["filesystem", "resize", &size_arg, &mount_point])
                    .output()
                    .await;

                let _ = Command::new("umount").arg(&mount_point).output().await;

                let _ = Command::new("rmdir").arg(&mount_point).output().await;

                if let Ok(output) = resize_output {
                    if !output.status.success() {
                        let err = String::from_utf8_lossy(&output.stderr);
                        Notification::send(
                            format!("Btrfs resize failed: {}", err),
                            NotificationLevel::Error,
                            sender,
                        )?;
                        return Err(anyhow!("btrfs resize failed"));
                    }
                } else {
                    Notification::send(
                        "Failed to resize Btrfs filesystem".to_string(),
                        NotificationLevel::Error,
                        sender,
                    )?;
                    return Err(anyhow!("btrfs resize failed"));
                }
            } else {
                let err = String::from_utf8_lossy(&mount_output.stderr);
                let _ = Command::new("rmdir").arg(&mount_point).output().await;
                Notification::send(
                    format!("Failed to mount for Btrfs resize: {}", err),
                    NotificationLevel::Error,
                    sender,
                )?;
                return Err(anyhow!("Mount failed"));
            }
        }
        "vfat" | "fat32" | "exfat" => {
            Notification::send(
                format!(
                    "{} filesystem cannot be easily resized. Consider reformatting.",
                    fs
                ),
                NotificationLevel::Warning,
                sender,
            )?;
        }
        _ => {
            Notification::send(
                format!(
                    "Filesystem '{}' resize not supported. Partition resized only.",
                    fs
                ),
                NotificationLevel::Warning,
                sender,
            )?;
        }
    }

    Ok(())
}

pub async fn is_luks_device(device: &str) -> Result<bool> {
    let output = Command::new("cryptsetup")
        .args(["isLuks", &format!("/dev/{}", device)])
        .output()
        .await;

    match output {
        Ok(output) => Ok(output.status.success()),
        Err(_) => Ok(false),
    }
}

pub async fn get_luks_info(device: &str) -> Result<LuksInfo> {
    let output = Command::new("cryptsetup")
        .args(["luksDump", &format!("/dev/{}", device)])
        .output()
        .await
        .context("Failed to execute cryptsetup luksDump")?;

    if !output.status.success() {
        return Err(anyhow!("Failed to get LUKS info"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut version = "LUKS2".to_string();
    let mut uuid = String::new();
    let mut cipher = String::new();
    let mut key_size = String::new();

    for line in stdout.lines() {
        if line.starts_with("Version:") {
            version = line.split_whitespace().nth(1).unwrap_or("2").to_string();
            version = format!("LUKS{}", version);
        } else if line.starts_with("UUID:") {
            uuid = line.split_whitespace().nth(1).unwrap_or("").to_string();
        } else if line.contains("Cipher:") {
            cipher = line.split(':').nth(1).unwrap_or("").trim().to_string();
        } else if line.contains("Key:") && line.contains("bits") {
            key_size = line
                .split_whitespace()
                .find(|s| s.chars().all(|c| c.is_numeric()))
                .unwrap_or("256")
                .to_string();
        }
    }

    Ok(LuksInfo {
        version,
        uuid,
        cipher,
        key_size,
    })
}

pub async fn get_luks_status(device: &str) -> Result<LuksStatus> {
    let mapper_entries = std::fs::read_dir("/dev/mapper");

    if mapper_entries.is_err() {
        return Ok(LuksStatus {
            is_active: false,
            mapper_name: None,
            device_path: None,
        });
    }

    for entry in mapper_entries.unwrap().flatten() {
        let mapper_name = entry.file_name().to_string_lossy().to_string();
        if mapper_name == "control" {
            continue;
        }

        let output = Command::new("cryptsetup")
            .args(["status", &mapper_name])
            .output()
            .await;

        if let Ok(output) = output {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);

                for line in stdout.lines() {
                    if line.trim().starts_with("device:") {
                        let dev_path = line.split(':').nth(1).unwrap_or("").trim();
                        if dev_path.ends_with(device) || dev_path == format!("/dev/{}", device) {
                            return Ok(LuksStatus {
                                is_active: true,
                                mapper_name: Some(mapper_name),
                                device_path: Some(dev_path.to_string()),
                            });
                        }
                    }
                }
            }
        }
    }

    Ok(LuksStatus {
        is_active: false,
        mapper_name: None,
        device_path: None,
    })
}

pub async fn unlock_luks_device(
    device: &str,
    passphrase: &str,
    mapper_name: &str,
    sender: &UnboundedSender<Event>,
) -> Result<()> {
    validate_device_name(device)?;
    validate_device_name(mapper_name)?;

    let status = get_luks_status(device).await?;
    if status.is_active {
        Notification::send(
            format!("{} is already unlocked", device),
            NotificationLevel::Warning,
            sender,
        )?;
        return Ok(());
    }

    sender.send(Event::StartProgress(format!("Unlocking {}...", device)))?;

    let device_path = format!("/dev/{}", device);

    let mut child = Command::new("cryptsetup")
        .args(["open", &device_path, mapper_name])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("Failed to spawn cryptsetup")?;

    {
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin.write_all(passphrase.as_bytes()).await?;
            stdin.write_all(b"\n").await?;
            stdin.flush().await?;
            drop(stdin);
        }
    }

    let output = child.wait_with_output().await?;

    if !output.status.success() {
        sender.send(Event::EndProgress)?;
        let err = String::from_utf8_lossy(&output.stderr);
        let error_msg = if err.contains("No key available") || err.contains("incorrect passphrase")
        {
            "Incorrect passphrase".to_string()
        } else {
            format!("Unlock failed: {}", err.trim())
        };

        Notification::send(error_msg, NotificationLevel::Error, sender)?;
        return Err(anyhow!("Unlock failed"));
    }

    let _ = Command::new("udevadm")
        .args(["settle", "--timeout=10"])
        .output()
        .await;

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    sender.send(Event::EndProgress)?;

    Notification::send(
        format!("Unlocked {} as /dev/mapper/{}", device, mapper_name),
        NotificationLevel::Info,
        sender,
    )?;

    Ok(())
}

pub async fn lock_luks_device(mapper_name: &str, sender: &UnboundedSender<Event>) -> Result<()> {
    validate_device_name(mapper_name)?;

    let mapper_path = format!("/dev/mapper/{}", mapper_name);

    let is_mounted = Command::new("findmnt")
        .args(["-n", &mapper_path])
        .output()
        .await
        .map(|output| output.status.success())
        .unwrap_or(false);

    if is_mounted {
        Notification::send(
            format!("{} is mounted. Unmount it first.", mapper_name),
            NotificationLevel::Error,
            sender,
        )?;
        return Err(anyhow!("Mapper device is mounted"));
    }

    sender.send(Event::StartProgress(format!("Locking {}...", mapper_name)))?;

    let close_future = Command::new("cryptsetup")
        .args(["close", mapper_name])
        .output();

    let output = match tokio::time::timeout(tokio::time::Duration::from_secs(10), close_future)
        .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            sender.send(Event::EndProgress)?;
            Notification::send(
                format!("Failed to lock: {}", e),
                NotificationLevel::Error,
                sender,
            )?;
            return Err(anyhow!("Failed to execute cryptsetup close"));
        }
        Err(_) => {
            sender.send(Event::EndProgress)?;
            Notification::send(
                format!(
                    "Lock operation timed out. Device may still be in use. Try closing any applications accessing the device."
                ),
                NotificationLevel::Error,
                sender,
            )?;
            return Err(anyhow!("Lock operation timed out"));
        }
    };

    if !output.status.success() {
        sender.send(Event::EndProgress)?;
        let err = String::from_utf8_lossy(&output.stderr);

        if err.contains("busy") || err.contains("in use") {
            Notification::send(
                format!(
                    "Device is busy. Close any applications using {} and try again.",
                    mapper_name
                ),
                NotificationLevel::Error,
                sender,
            )?;
        } else {
            Notification::send(
                format!("Lock failed: {}", err.trim()),
                NotificationLevel::Error,
                sender,
            )?;
        }
        return Err(anyhow!("Lock failed"));
    }

    let _ = Command::new("udevadm")
        .args(["settle", "--timeout=10"])
        .output()
        .await;

    sender.send(Event::EndProgress)?;

    Notification::send(
        format!("Locked {}", mapper_name),
        NotificationLevel::Info,
        sender,
    )?;

    Ok(())
}

pub async fn encrypt_partition(
    partition: &str,
    passphrase: &str,
    sender: &UnboundedSender<Event>,
) -> Result<()> {
    validate_device_name(partition)?;

    if is_mounted(partition).await? {
        Notification::send(
            format!("{} is mounted. Unmount it first.", partition),
            NotificationLevel::Error,
            sender,
        )?;
        return Err(anyhow!("Partition is mounted"));
    }

    sender.send(Event::StartProgress(format!("Encrypting {}...", partition)))?;

    let device_path = format!("/dev/{}", partition);

    let mut child = Command::new("cryptsetup")
        .args([
            "luksFormat",
            "--type",
            "luks2",
            "--batch-mode",
            &device_path,
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("Failed to spawn cryptsetup")?;

    {
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin.write_all(passphrase.as_bytes()).await?;
            stdin.write_all(b"\n").await?;
            stdin.flush().await?;
            drop(stdin);
        }
    }

    let output = child.wait_with_output().await?;

    sender.send(Event::EndProgress)?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        Notification::send(
            format!("Encryption failed: {}", err.trim()),
            NotificationLevel::Error,
            sender,
        )?;
        return Err(anyhow!("Encryption failed"));
    }

    Notification::send(
        format!("Encrypted {} with LUKS2", partition),
        NotificationLevel::Info,
        sender,
    )?;

    Ok(())
}

pub async fn encrypt_and_format_partition(
    partition: &str,
    passphrase: &str,
    fs_type: FilesystemType,
    sender: &UnboundedSender<Event>,
) -> Result<()> {
    validate_device_name(partition)?;

    sender.send(Event::StartProgress(format!("Encrypting {}...", partition)))?;

    encrypt_partition(partition, passphrase, sender).await?;

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let mapper_name = format!("luks-{}", partition);

    Notification::send(
        format!("Unlocking encrypted partition..."),
        NotificationLevel::Info,
        sender,
    )?;

    unlock_luks_device(partition, passphrase, &mapper_name, sender).await?;

    let mapper_path = format!("/dev/mapper/{}", mapper_name);
    wait_for_device(&mapper_path, 10).await?;

    let _ = Command::new("udevadm")
        .args(["settle", "--timeout=10"])
        .output()
        .await;

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    Notification::send(
        format!("Formatting with {}...", fs_type),
        NotificationLevel::Info,
        sender,
    )?;

    format_partition(&mapper_name, fs_type, sender.clone()).await?;

    sender.send(Event::EndProgress)?;

    Notification::send(
        format!(
            "Partition {} encrypted and formatted successfully",
            partition
        ),
        NotificationLevel::Info,
        sender,
    )?;

    Ok(())
}

pub async fn create_encrypted_partition_with_fs(
    disk: &str,
    size_input: &str,
    passphrase: &str,
    fs_type: FilesystemType,
    sender: &UnboundedSender<Event>,
) -> Result<()> {
    sender.send(Event::StartProgress(format!(
        "Creating encrypted partition on {}...",
        disk
    )))?;

    let part_name = create_partition_raw(disk, size_input, sender).await?;

    encrypt_and_format_partition(&part_name, passphrase, fs_type, sender).await?;

    Notification::send(
        format!("Created encrypted partition on {}", disk),
        NotificationLevel::Info,
        sender,
    )?;

    Ok(())
}
