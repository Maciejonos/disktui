use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
	Mount {
		device: String,
	},
	Unmount {
		device: String,
	},
	Format {
		device: String,
		fs_type: String,
	},
	FormatWholeDisk {
		disk: String,
		fs_type: String,
	},
	CreatePartitionTable {
		disk: String,
		table_type: String,
	},
	CreatePartition {
		disk: String,
		size: String,
		fs_type: Option<String>,
	},
	CreateEncryptedPartition {
		disk: String,
		size: String,
		passphrase: String,
		fs_type: String,
	},
	DeletePartition {
		partition: String,
	},
	ResizePartition {
		partition: String,
		new_size: String,
	},
	UnlockLuks {
		device: String,
		passphrase: String,
		mapper_name: String,
	},
	LockLuks {
		mapper_name: String,
	},
	EncryptPartition {
		partition: String,
		passphrase: String,
	},
	EncryptAndFormat {
		partition: String,
		passphrase: String,
		fs_type: String,
	},
	Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
	Ok {
		data: Option<String>,
	},
	Error {
		message: String,
	},
	Notification {
		level: String,
		message: String,
	},
	Progress {
		action: String,
		message: Option<String>,
	},
}

impl Response {
	pub fn ok() -> Self {
		Self::Ok { data: None }
	}

	pub fn error(message: impl Into<String>) -> Self {
		Self::Error {
			message: message.into(),
		}
	}

	pub fn notification(level: &str, message: impl Into<String>) -> Self {
		Self::Notification {
			level: level.to_string(),
			message: message.into(),
		}
	}

	pub fn progress_start(message: impl Into<String>) -> Self {
		Self::Progress {
			action: "start".to_string(),
			message: Some(message.into()),
		}
	}

	pub fn progress_end() -> Self {
		Self::Progress {
			action: "end".to_string(),
			message: None,
		}
	}
}
