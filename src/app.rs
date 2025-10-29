use crate::disk::Disk;
use crate::notification::Notification;
use crate::operations::{FilesystemType, get_smart_data, list_block_devices};
use crate::theme::Theme;
use anyhow::Result;
use ratatui::widgets::{ListState, TableState};
use std::sync::{Arc, atomic::AtomicBool};
use tui_input::Input;

pub type AppResult<T> = Result<T>;

#[derive(Debug, Clone)]
pub enum ConfirmationOperation {
    None,
    FormatPartition {
        partition: String,
        fs_type: FilesystemType,
    },
    FormatDisk {
        disk: String,
        fs_type: FilesystemType,
    },
    DeletePartition {
        partition: String,
    },
    CreatePartitionTable {
        disk: String,
        table_type: String,
    },
    CreatePartition {
        disk: String,
        size: String,
        fs_type: FilesystemType,
    },
    ResizePartition {
        partition: String,
        new_size: String,
    },
    UnlockLuksDevice {
        device: String,
        mapper_name: String,
    },
    LockLuksDevice {
        mapper_name: String,
    },
    EncryptPartition {
        partition: String,
        fs_type: crate::operations::FilesystemType,
    },
}

#[derive(Debug)]
pub struct ConfirmationDialog {
    pub show_dialog: bool,
    pub title: String,
    pub message: String,
    pub details: Vec<(String, String)>,
    pub selected: usize,
    pub operation: ConfirmationOperation,
}

impl Default for ConfirmationDialog {
    fn default() -> Self {
        Self {
            show_dialog: false,
            title: String::new(),
            message: String::new(),
            details: Vec::new(),
            selected: 0,
            operation: ConfirmationOperation::None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FocusedBlock {
    Disks,
    Partitions,
    DiskInfo,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PartitionDialogMode {
    SelectTableType,
    CreatePartition,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CreatePartitionStep {
    EnterSize,
    SelectFilesystem,
}

#[derive(Debug, Default)]
pub struct ProgressState {
    pub show_dialog: bool,
    pub message: String,
    pub disk_name: String,
    pub disk_model: String,
    pub spinner_index: usize,
}

#[derive(Debug)]
pub struct FormatDialogState {
    pub show_dialog: bool,
    pub type_state: ListState,
    pub encrypt_mode: bool,
}

impl Default for FormatDialogState {
    fn default() -> Self {
        let mut type_state = ListState::default();
        type_state.select(Some(0));
        Self {
            show_dialog: false,
            encrypt_mode: false,
            type_state,
        }
    }
}

#[derive(Debug)]
pub struct PartitionDialogState {
    pub show_dialog: bool,
    pub mode: PartitionDialogMode,
    pub create_step: CreatePartitionStep,
    pub table_type_state: ListState,
    pub table_types: Vec<String>,
    pub size_input: Input,
    pub new_partition_fs_state: ListState,
}

impl Default for PartitionDialogState {
    fn default() -> Self {
        let mut table_type_state = ListState::default();
        table_type_state.select(Some(0));
        let mut new_partition_fs_state = ListState::default();
        new_partition_fs_state.select(Some(0));

        Self {
            show_dialog: false,
            mode: PartitionDialogMode::SelectTableType,
            create_step: CreatePartitionStep::EnterSize,
            table_type_state,
            table_types: vec!["gpt".to_string(), "msdos".to_string()],
            size_input: Input::default(),
            new_partition_fs_state,
        }
    }
}

#[derive(Debug)]
pub struct ResizeDialogState {
    pub show_dialog: bool,
    pub size_input: Input,
}

impl Default for ResizeDialogState {
    fn default() -> Self {
        Self {
            show_dialog: false,
            size_input: Input::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PassphraseOperation {
    Unlock,
    Encrypt,
    EncryptConfirm,
}

#[derive(Debug)]
pub struct PassphraseDialogState {
    pub show_dialog: bool,
    pub input: Input,
    pub operation: PassphraseOperation,
    pub target_device: String,
    pub confirm_mode: bool,
    pub first_passphrase: String,
    pub filesystem_type: Option<crate::operations::FilesystemType>,
}

impl Default for PassphraseDialogState {
    fn default() -> Self {
        Self {
            show_dialog: false,
            input: Input::default(),
            operation: PassphraseOperation::Unlock,
            target_device: String::new(),
            confirm_mode: false,
            first_passphrase: String::new(),
            filesystem_type: None,
        }
    }
}

pub struct App {
    pub running: bool,
    pub focused_block: FocusedBlock,
    pub disks: Vec<Disk>,
    pub disks_state: TableState,
    pub partitions_state: TableState,
    pub notifications: Vec<Notification>,
    pub show_help: bool,
    pub filesystem_types: Vec<FilesystemType>,
    pub operation_in_progress: Arc<AtomicBool>,
    pub progress: ProgressState,
    pub format_dialog: FormatDialogState,
    pub partition_dialog: PartitionDialogState,
    pub resize_dialog: ResizeDialogState,
    pub passphrase_dialog: PassphraseDialogState,
    pub confirmation_dialog: ConfirmationDialog,
    pub theme: Theme,
}

impl App {
    pub async fn new() -> AppResult<Self> {
        let devices = list_block_devices().await?;
        let mut disks = Vec::new();

        for device in devices {
            let smart_data = get_smart_data(&device.name).await.ok();
            disks.push(Disk::new(device, smart_data));
        }

        let mut disks_state = TableState::default();
        if !disks.is_empty() {
            disks_state.select(Some(0));
        }

        let mut partitions_state = TableState::default();
        if !disks.is_empty() && !disks[0].device.partitions.is_empty() {
            partitions_state.select(Some(0));
        }

        let filesystem_types = FilesystemType::all();

        Ok(Self {
            running: true,
            focused_block: FocusedBlock::Disks,
            disks,
            disks_state,
            partitions_state,
            notifications: Vec::new(),
            show_help: false,
            filesystem_types,
            operation_in_progress: Arc::new(AtomicBool::new(false)),
            progress: ProgressState::default(),
            format_dialog: FormatDialogState::default(),
            partition_dialog: PartitionDialogState::default(),
            resize_dialog: ResizeDialogState::default(),
            passphrase_dialog: PassphraseDialogState::default(),
            confirmation_dialog: ConfirmationDialog::default(),
            theme: Theme::new(),
        })
    }

    pub async fn refresh(&mut self) -> AppResult<()> {
        let devices = list_block_devices().await?;
        let selected_disk_index = self.disks_state.selected();
        let selected_partition_index = self.partitions_state.selected();

        let mut disks = Vec::new();
        for device in devices {
            let smart_data = get_smart_data(&device.name).await.ok();
            disks.push(Disk::new(device, smart_data));
        }

        self.disks = disks;

        if let Some(idx) = selected_disk_index {
            if idx < self.disks.len() {
                self.disks_state.select(Some(idx));
            } else if !self.disks.is_empty() {
                self.disks_state.select(Some(0));
            } else {
                self.disks_state.select(None);
            }
        }

        if let Some(disk_idx) = self.disks_state.selected() {
            if disk_idx < self.disks.len() {
                let partitions_len = self.disks[disk_idx].device.partitions.len();
                if let Some(part_idx) = selected_partition_index {
                    if part_idx < partitions_len {
                        self.partitions_state.select(Some(part_idx));
                    } else if partitions_len > 0 {
                        self.partitions_state.select(Some(0));
                    } else {
                        self.partitions_state.select(None);
                    }
                } else if partitions_len > 0 {
                    self.partitions_state.select(Some(0));
                }
            }
        }

        Ok(())
    }

    pub async fn tick(&mut self) -> AppResult<()> {
        self.notifications.retain(|n| n.ttl > 0);
        self.notifications.iter_mut().for_each(|n| n.ttl -= 1);

        if self.progress.show_dialog {
            self.progress.spinner_index = (self.progress.spinner_index + 1) % 10;
        }

        Ok(())
    }

    pub fn selected_disk(&self) -> Option<&Disk> {
        self.disks_state.selected().and_then(|i| self.disks.get(i))
    }

    pub fn selected_partition(&self) -> Option<&crate::partition::Partition> {
        if let Some(disk_idx) = self.disks_state.selected() {
            if let Some(disk) = self.disks.get(disk_idx) {
                if let Some(part_idx) = self.partitions_state.selected() {
                    return disk.device.partitions.get(part_idx);
                }
            }
        }
        None
    }

    pub fn quit(&mut self) {
        self.running = false;
    }
}
