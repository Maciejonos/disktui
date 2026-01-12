use anyhow::{anyhow, Context, Result};
use disktui::protocol::{Request, Response};
use std::io::{BufRead, Write};
use tokio::process::Command;

struct ResponseWriter {
	stdout: std::io::Stdout,
}

impl ResponseWriter {
	fn new() -> Self {
		Self { stdout: std::io::stdout() }
	}

	fn send(&mut self, response: Response) -> Result<()> {
		let json = serde_json::to_string(&response)?;
		writeln!(self.stdout, "{}", json)?;
		self.stdout.flush()?;
		Ok(())
	}

	fn notify(&mut self, level: &str, message: impl Into<String>) -> Result<()> {
		self.send(Response::notification(level, message))
	}

	fn progress_start(&mut self, message: impl Into<String>) -> Result<()> {
		self.send(Response::progress_start(message))
	}

	fn progress_end(&mut self) -> Result<()> {
		self.send(Response::progress_end())
	}
}

fn validate_device_name(name: &str) -> Result<()> {
	if name.is_empty() {
		return Err(anyhow!("Invalid device name: empty"));
	}
	if name.contains("..") || name.contains('/') {
		return Err(anyhow!("Invalid device name: contains path traversal characters"));
	}
	if !name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
		return Err(anyhow!("Invalid device name: contains illegal characters"));
	}
	if name.len() > 32 {
		return Err(anyhow!("Invalid device name: too long"));
	}
	Ok(())
}

fn get_device_path(device_name: &str) -> String {
	if device_name.starts_with("luks-") {
		format!("/dev/mapper/{}", device_name)
	} else {
		let mapper_path = format!("/dev/mapper/{}", device_name);
		if std::path::Path::new(&mapper_path).exists() {
			mapper_path
		} else {
			format!("/dev/{}", device_name)
		}
	}
}

async fn is_mounted(partition: &str) -> Result<bool> {
	let device_path = get_device_path(partition);
	let output = Command::new("findmnt")
		.args(["-n", &device_path])
		.output()
		.await
		.context("Failed to check mount status")?;
	Ok(output.status.success())
}

async fn get_device_mount_point(device_path: &str) -> Option<String> {
	let output = Command::new("findmnt")
		.args(["-n", "-o", "TARGET", device_path])
		.output()
		.await
		.ok()?;
	if output.status.success() {
		let mount_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
		if !mount_str.is_empty() {
			return Some(mount_str);
		}
	}
	None
}

async fn mount_partition(device: &str, writer: &mut ResponseWriter) -> Result<()> {
	validate_device_name(device)?;

	if is_mounted(device).await? {
		writer.notify("warning", format!("{} already mounted", device))?;
		return Ok(());
	}

	let device_path = get_device_path(device);
	if !std::path::Path::new(&device_path).exists() {
		return Err(anyhow!("Device {} does not exist", device_path));
	}

	let mount_point = format!("/mnt/{}", device);
	Command::new("mkdir").args(["-p", &mount_point]).output().await?;

	writer.progress_start(format!("Mounting {}...", device))?;

	let output = Command::new("mount")
		.args([&device_path, &mount_point])
		.output()
		.await
		.context("Failed to execute mount")?;

	writer.progress_end()?;

	if !output.status.success() {
		let err = String::from_utf8_lossy(&output.stderr);
		let _ = Command::new("rmdir").arg(&mount_point).output().await;
		return Err(anyhow!("Mount failed: {}", err));
	}

	writer.notify("info", format!("Mounted {} at {}", device, mount_point))?;
	Ok(())
}

async fn unmount_partition(device: &str, writer: &mut ResponseWriter) -> Result<()> {
	validate_device_name(device)?;

	if !is_mounted(device).await? {
		writer.notify("warning", format!("{} not mounted", device))?;
		return Ok(());
	}

	let device_path = get_device_path(device);
	let actual_mount_point = get_device_mount_point(&device_path).await;

	writer.progress_start(format!("Unmounting {}...", device))?;

	let output = Command::new("umount")
		.arg(&device_path)
		.output()
		.await
		.context("Failed to execute umount")?;

	writer.progress_end()?;

	if !output.status.success() {
		let err = String::from_utf8_lossy(&output.stderr);
		if err.contains("target is busy") || err.contains("device is busy") {
			writer.notify("warning", format!("Device {} is busy. Attempting lazy unmount...", device))?;
			let lazy_output = Command::new("umount").args(["-l", &device_path]).output().await?;
			if !lazy_output.status.success() {
				return Err(anyhow!("Lazy unmount failed"));
			}
			if let Some(ref mp) = actual_mount_point
				&& mp.starts_with("/mnt/") {
					let _ = Command::new("rmdir").arg(mp).output().await;
				}
			writer.notify("warning", format!("Lazy unmount initiated for {}. Device still in use.", device))?;
			return Ok(());
		}
		return Err(anyhow!("Unmount failed: {}", err));
	}

	if let Some(ref mp) = actual_mount_point
		&& mp.starts_with("/mnt/") {
			let _ = Command::new("rmdir").arg(mp).output().await;
		}

	writer.notify("info", format!("Unmounted {}", device))?;
	Ok(())
}

async fn format_partition(device: &str, fs_type: &str, writer: &mut ResponseWriter) -> Result<()> {
	validate_device_name(device)?;

	let device_path = get_device_path(device);
	if !std::path::Path::new(&device_path).exists() {
		return Err(anyhow!("Device {} does not exist", device_path));
	}

	let cmd = match fs_type {
		"ext4" => "mkfs.ext4",
		"fat32" | "vfat" => "mkfs.fat",
		"ntfs" => "mkfs.ntfs",
		"exfat" => "mkfs.exfat",
		"btrfs" => "mkfs.btrfs",
		"xfs" => "mkfs.xfs",
		_ => return Err(anyhow!("Unsupported filesystem type: {}", fs_type)),
	};

	let which_output = Command::new("which").arg(cmd).output().await?;
	if !which_output.status.success() {
		return Err(anyhow!("{} not found. Install the appropriate package.", cmd));
	}

	writer.progress_start(format!("Formatting {} as {}...", device, fs_type))?;

	let mut command = Command::new(cmd);
	match fs_type {
		"fat32" | "vfat" => {
			command.args(["-F", "32", &device_path]);
		}
		"ntfs" => {
			command.args(["-f", "-Q", &device_path]);
		}
		"btrfs" | "xfs" => {
			command.args(["-f", &device_path]);
		}
		_ => {
			command.arg(&device_path);
		}
	}

	let output = command.output().await.context("Failed to execute mkfs")?;

	writer.progress_end()?;

	if !output.status.success() {
		let err = String::from_utf8_lossy(&output.stderr);
		return Err(anyhow!("Format failed: {}", err));
	}

	writer.notify("info", format!("Formatted {} as {}", device, fs_type))?;
	Ok(())
}

async fn create_partition_table(disk: &str, table_type: &str, writer: &mut ResponseWriter) -> Result<()> {
	validate_device_name(disk)?;

	let label = match table_type {
		"gpt" => "gpt",
		"mbr" | "msdos" => "msdos",
		_ => return Err(anyhow!("Unsupported partition table type: {}", table_type)),
	};

	writer.progress_start(format!("Creating {} partition table on {}...", table_type, disk))?;

	let output = Command::new("parted")
		.args(["-s", &format!("/dev/{}", disk), "mklabel", label])
		.output()
		.await
		.context("Failed to execute parted")?;

	writer.progress_end()?;

	if !output.status.success() {
		let err = String::from_utf8_lossy(&output.stderr);
		return Err(anyhow!("Failed to create partition table: {}", err));
	}

	writer.notify("info", format!("Created {} partition table on {}", table_type, disk))?;
	Ok(())
}

async fn get_last_partition_end_bytes(disk: &str) -> Result<u64> {
	let output = Command::new("parted")
		.args(["-s", "-m", &format!("/dev/{}", disk), "unit", "B", "print"])
		.output()
		.await
		.context("Failed to execute parted")?;

	if !output.status.success() {
		return Ok(1_048_576);
	}

	let stdout = String::from_utf8_lossy(&output.stdout);
	let mut last_end: u64 = 1_048_576;

	for line in stdout.lines() {
		let parts: Vec<&str> = line.split(':').collect();
		if parts.len() >= 3
			&& let Ok(_part_num) = parts[0].parse::<u32>()
			&& let Some(end_str) = parts[2].strip_suffix('B')
			&& let Ok(end) = end_str.parse::<u64>()
			&& end > last_end
		{
			last_end = end;
		}
	}

	let aligned = ((last_end + 1_048_576) / 1_048_576) * 1_048_576;
	Ok(aligned)
}

fn parse_size(input: &str) -> Result<u64> {
	let input = input.trim().to_uppercase();
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

	let num: f64 = num_str.parse().map_err(|_| anyhow!("Invalid size format"))?;
	Ok((num * unit as f64).round() as u64)
}

async fn create_partition(disk: &str, size: &str, fs_type: Option<&str>, writer: &mut ResponseWriter) -> Result<String> {
	validate_device_name(disk)?;

	let start_offset = get_last_partition_end_bytes(disk).await?;

	let lsblk_output = Command::new("lsblk")
		.args(["-b", "-d", "-n", "-o", "SIZE", &format!("/dev/{}", disk)])
		.output()
		.await?;
	let disk_size: u64 = String::from_utf8_lossy(&lsblk_output.stdout)
		.trim()
		.parse()
		.unwrap_or(0);

	let free_space = disk_size.saturating_sub(start_offset);
	if free_space == 0 {
		return Err(anyhow!("No free space available"));
	}

	let requested_size = if size.trim().is_empty() {
		free_space
	} else {
		parse_size(size)?
	};

	if requested_size > free_space {
		return Err(anyhow!("Requested size exceeds available space"));
	}

	let start_mb = start_offset / 1_000_000;
	let end_offset = start_offset + requested_size;
	let end_mb = end_offset / 1_000_000;

	writer.progress_start(format!("Creating partition on {}...", disk))?;

	let output = Command::new("parted")
		.args(["-s", &format!("/dev/{}", disk), "mkpart", "primary", &format!("{}MB", start_mb), &format!("{}MB", end_mb)])
		.output()
		.await
		.context("Failed to execute parted")?;

	if !output.status.success() {
		writer.progress_end()?;
		let err = String::from_utf8_lossy(&output.stderr);
		return Err(anyhow!("Create partition failed: {}", err));
	}

	tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

	let lsblk_output = Command::new("lsblk")
		.args(["-J", "-o", "NAME", &format!("/dev/{}", disk)])
		.output()
		.await?;
	let lsblk_str = String::from_utf8_lossy(&lsblk_output.stdout);

	let new_partition = if let Ok(json) = serde_json::from_str::<serde_json::Value>(&lsblk_str) {
		json["blockdevices"][0]["children"]
			.as_array()
			.and_then(|arr| arr.last())
			.and_then(|p| p["name"].as_str())
			.map(|s| s.to_string())
	} else {
		None
	};

	let partition_name = new_partition.ok_or_else(|| anyhow!("Failed to find new partition"))?;

	if let Some(fs) = fs_type {
		format_partition(&partition_name, fs, writer).await?;
	}

	writer.progress_end()?;
	writer.notify("info", format!("Created partition {}", partition_name))?;

	Ok(partition_name)
}

async fn delete_partition(partition: &str, writer: &mut ResponseWriter) -> Result<()> {
	validate_device_name(partition)?;

	if is_mounted(partition).await? {
		unmount_partition(partition, writer).await?;
	}

	let (disk, part_num) = if partition.starts_with("nvme") || partition.starts_with("mmcblk") {
		let parts: Vec<&str> = partition.rsplitn(2, 'p').collect();
		if parts.len() == 2 && !parts[0].is_empty() && parts[0].chars().all(|c| c.is_numeric()) {
			(parts[1], parts[0])
		} else {
			return Err(anyhow!("Invalid partition name format: {}", partition));
		}
	} else {
		let disk = partition.trim_end_matches(|c: char| c.is_numeric());
		let part_num = partition.trim_start_matches(disk);
		if part_num.is_empty() || !part_num.chars().all(|c| c.is_numeric()) {
			return Err(anyhow!("Invalid partition name format: {}", partition));
		}
		(disk, part_num)
	};

	writer.progress_start(format!("Deleting partition {}...", partition))?;

	let output = Command::new("parted")
		.args(["-s", &format!("/dev/{}", disk), "rm", part_num])
		.output()
		.await
		.context("Failed to execute parted")?;

	writer.progress_end()?;

	if !output.status.success() {
		let err = String::from_utf8_lossy(&output.stderr);
		return Err(anyhow!("Delete partition failed: {}", err));
	}

	let _ = Command::new("partprobe").arg(format!("/dev/{}", disk)).output().await;

	writer.notify("info", format!("Deleted partition {}", partition))?;
	Ok(())
}

async fn unlock_luks(device: &str, passphrase: &str, mapper_name: &str, writer: &mut ResponseWriter) -> Result<()> {
	validate_device_name(device)?;
	validate_device_name(mapper_name)?;

	let device_path = format!("/dev/{}", device);

	writer.progress_start(format!("Unlocking {}...", device))?;

	let mut child = Command::new("cryptsetup")
		.args(["open", &device_path, mapper_name])
		.stdin(std::process::Stdio::piped())
		.stdout(std::process::Stdio::piped())
		.stderr(std::process::Stdio::piped())
		.spawn()?;

	if let Some(mut stdin) = child.stdin.take() {
		use tokio::io::AsyncWriteExt;
		stdin.write_all(passphrase.as_bytes()).await?;
		stdin.write_all(b"\n").await?;
		stdin.flush().await?;
		drop(stdin);
	}

	let output = child.wait_with_output().await?;

	writer.progress_end()?;

	if !output.status.success() {
		let err = String::from_utf8_lossy(&output.stderr);
		return Err(anyhow!("Failed to unlock: {}", err));
	}

	writer.notify("info", format!("Unlocked {} as {}", device, mapper_name))?;
	Ok(())
}

async fn lock_luks(mapper_name: &str, writer: &mut ResponseWriter) -> Result<()> {
	validate_device_name(mapper_name)?;

	let mapper_path = format!("/dev/mapper/{}", mapper_name);

	if std::path::Path::new(&mapper_path).exists()
		&& let Some(mount_point) = get_device_mount_point(&mapper_path).await {
			writer.notify("info", format!("Unmounting {} first...", mapper_name))?;
			let _ = Command::new("umount").arg(&mapper_path).output().await;
			if mount_point.starts_with("/mnt/") {
				let _ = Command::new("rmdir").arg(&mount_point).output().await;
			}
		}

	writer.progress_start(format!("Locking {}...", mapper_name))?;

	let output = Command::new("cryptsetup")
		.args(["close", mapper_name])
		.output()
		.await
		.context("Failed to execute cryptsetup close")?;

	writer.progress_end()?;

	if !output.status.success() {
		let err = String::from_utf8_lossy(&output.stderr);
		return Err(anyhow!("Failed to lock: {}", err));
	}

	writer.notify("info", format!("Locked {}", mapper_name))?;
	Ok(())
}

async fn encrypt_partition(partition: &str, passphrase: &str, writer: &mut ResponseWriter) -> Result<()> {
	validate_device_name(partition)?;

	let device_path = format!("/dev/{}", partition);

	writer.progress_start(format!("Encrypting {}...", partition))?;

	let mut child = Command::new("cryptsetup")
		.args(["luksFormat", "--type", "luks2", "-q", &device_path])
		.stdin(std::process::Stdio::piped())
		.stdout(std::process::Stdio::piped())
		.stderr(std::process::Stdio::piped())
		.spawn()?;

	if let Some(mut stdin) = child.stdin.take() {
		use tokio::io::AsyncWriteExt;
		stdin.write_all(passphrase.as_bytes()).await?;
		stdin.write_all(b"\n").await?;
		stdin.flush().await?;
		drop(stdin);
	}

	let output = child.wait_with_output().await?;

	writer.progress_end()?;

	if !output.status.success() {
		let err = String::from_utf8_lossy(&output.stderr);
		return Err(anyhow!("Encryption failed: {}", err));
	}

	writer.notify("info", format!("Encrypted {} with LUKS2", partition))?;
	Ok(())
}

async fn encrypt_and_format(partition: &str, passphrase: &str, fs_type: &str, writer: &mut ResponseWriter) -> Result<()> {
	encrypt_partition(partition, passphrase, writer).await?;

	tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

	let mapper_name = format!("luks-{}", partition);
	unlock_luks(partition, passphrase, &mapper_name, writer).await?;

	let mapper_path = format!("/dev/mapper/{}", mapper_name);
	for _ in 0..10 {
		if std::path::Path::new(&mapper_path).exists() {
			break;
		}
		tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
	}

	format_partition(&mapper_name, fs_type, writer).await?;

	writer.notify("info", format!("Partition {} encrypted and formatted", partition))?;
	Ok(())
}

async fn format_whole_disk(disk: &str, fs_type: &str, writer: &mut ResponseWriter) -> Result<()> {
	validate_device_name(disk)?;

	writer.progress_start(format!("Formatting entire disk {}...", disk))?;

	let output = Command::new("parted")
		.args(["-s", &format!("/dev/{}", disk), "mklabel", "gpt"])
		.output()
		.await?;

	if !output.status.success() {
		writer.progress_end()?;
		let err = String::from_utf8_lossy(&output.stderr);
		return Err(anyhow!("Failed to create partition table: {}", err));
	}

	let output = Command::new("parted")
		.args(["-s", &format!("/dev/{}", disk), "mkpart", "primary", "1MiB", "100%"])
		.output()
		.await?;

	if !output.status.success() {
		writer.progress_end()?;
		let err = String::from_utf8_lossy(&output.stderr);
		return Err(anyhow!("Failed to create partition: {}", err));
	}

	let _ = Command::new("partprobe").arg(format!("/dev/{}", disk)).output().await;
	tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

	let partition = if disk.starts_with("nvme") || disk.starts_with("mmcblk") {
		format!("{}p1", disk)
	} else {
		format!("{}1", disk)
	};

	format_partition(&partition, fs_type, writer).await?;

	writer.progress_end()?;
	writer.notify("info", format!("Formatted {} as whole disk with {}", disk, fs_type))?;
	Ok(())
}

async fn resize_partition(partition: &str, _new_size: &str, _writer: &mut ResponseWriter) -> Result<()> {
	validate_device_name(partition)?;
	Err(anyhow!("Partition resize not implemented yet"))
}

async fn create_encrypted_partition(disk: &str, size: &str, passphrase: &str, fs_type: &str, writer: &mut ResponseWriter) -> Result<()> {
	let partition = create_partition(disk, size, None, writer).await?;
	encrypt_and_format(&partition, passphrase, fs_type, writer).await?;
	Ok(())
}

async fn handle_request(request: Request, writer: &mut ResponseWriter) -> Result<()> {
	match request {
		Request::Mount { device } => mount_partition(&device, writer).await,
		Request::Unmount { device } => unmount_partition(&device, writer).await,
		Request::Format { device, fs_type } => format_partition(&device, &fs_type, writer).await,
		Request::FormatWholeDisk { disk, fs_type } => format_whole_disk(&disk, &fs_type, writer).await,
		Request::CreatePartitionTable { disk, table_type } => create_partition_table(&disk, &table_type, writer).await,
		Request::CreatePartition { disk, size, fs_type } => {
			create_partition(&disk, &size, fs_type.as_deref(), writer).await?;
			Ok(())
		}
		Request::CreateEncryptedPartition { disk, size, passphrase, fs_type } => {
			create_encrypted_partition(&disk, &size, &passphrase, &fs_type, writer).await
		}
		Request::DeletePartition { partition } => delete_partition(&partition, writer).await,
		Request::ResizePartition { partition, new_size } => resize_partition(&partition, &new_size, writer).await,
		Request::UnlockLuks { device, passphrase, mapper_name } => unlock_luks(&device, &passphrase, &mapper_name, writer).await,
		Request::LockLuks { mapper_name } => lock_luks(&mapper_name, writer).await,
		Request::EncryptPartition { partition, passphrase } => encrypt_partition(&partition, &passphrase, writer).await,
		Request::EncryptAndFormat { partition, passphrase, fs_type } => {
			encrypt_and_format(&partition, &passphrase, &fs_type, writer).await
		}
		Request::Shutdown => std::process::exit(0),
	}
}

#[tokio::main]
async fn main() -> Result<()> {
	let stdin = std::io::stdin();
	let mut writer = ResponseWriter::new();

	for line in stdin.lock().lines() {
		let line = match line {
			Ok(l) => l,
			Err(_) => break,
		};

		if line.trim().is_empty() {
			continue;
		}

		let request: Request = match serde_json::from_str(&line) {
			Ok(r) => r,
			Err(e) => {
				let _ = writer.send(Response::error(format!("Invalid request: {}", e)));
				continue;
			}
		};

		match handle_request(request, &mut writer).await {
			Ok(()) => {
				let _ = writer.send(Response::ok());
			}
			Err(e) => {
				let _ = writer.send(Response::error(e.to_string()));
			}
		}
	}

	Ok(())
}
