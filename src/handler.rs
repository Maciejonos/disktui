use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::mpsc::UnboundedSender;
use tui_input::backend::crossterm::EventHandler;

use crate::app::{App, AppResult, FocusedBlock, PartitionDialogMode};
use crate::config::Config;
use crate::event::Event;
use crate::notification::{Notification, NotificationLevel};
use crate::operations::{
    create_partition_table, create_partition_with_fs, delete_partition, format_partition,
    format_whole_disk, mount_partition, unmount_partition,
};

fn check_operation_in_progress(app: &App, sender: &UnboundedSender<Event>) -> bool {
    if app.operation_in_progress.load(Ordering::Acquire) {
        let _ = Notification::send(
            "Operation already in progress".to_string(),
            NotificationLevel::Warning,
            sender,
        );
        true
    } else {
        false
    }
}

pub async fn handle_key_events(
    key_event: KeyEvent,
    app: &mut App,
    sender: UnboundedSender<Event>,
    config: Arc<Config>,
) -> AppResult<()> {
    if app.show_help {
        app.show_help = false;
        return Ok(());
    }

    if app.confirmation_dialog.show_dialog {
        return handle_confirmation_dialog(key_event, app, sender).await;
    }

    if app.format_dialog.show_dialog {
        return handle_format_dialog(key_event, app, sender).await;
    }

    if app.partition_dialog.show_dialog {
        return handle_partition_dialog(key_event, app, sender).await;
    }

    match key_event.code {
        KeyCode::Char('q') | KeyCode::Char('Q') => {
            if app.focused_block == FocusedBlock::DiskInfo {
                app.focused_block = FocusedBlock::Disks;
            } else {
                app.quit();
            }
        }
        KeyCode::Char('c') | KeyCode::Char('C') if key_event.modifiers == KeyModifiers::CONTROL => {
            app.quit();
        }
        KeyCode::Esc => {
            if app.focused_block == FocusedBlock::DiskInfo {
                app.focused_block = FocusedBlock::Disks;
            }
        }
        KeyCode::Char('?') => {
            app.show_help = true;
        }
        KeyCode::Char(c) if c == config.disk.info => {
            if app.focused_block == FocusedBlock::Disks
                || app.focused_block == FocusedBlock::Partitions
            {
                app.focused_block = FocusedBlock::DiskInfo;
            } else if app.focused_block == FocusedBlock::DiskInfo {
                app.focused_block = FocusedBlock::Disks;
            }
        }
        KeyCode::Tab | KeyCode::BackTab => {
            app.focused_block = match app.focused_block {
                FocusedBlock::Disks => FocusedBlock::Partitions,
                FocusedBlock::Partitions => FocusedBlock::Disks,
                _ => FocusedBlock::Disks,
            };
        }
        KeyCode::Char(c) if c == config.navigation.scroll_down => {
            handle_scroll_down(app);
        }
        KeyCode::Down => {
            handle_scroll_down(app);
        }
        KeyCode::Char(c) if c == config.navigation.scroll_up => {
            handle_scroll_up(app);
        }
        KeyCode::Up => {
            handle_scroll_up(app);
        }
        KeyCode::Char(c) if c == config.disk.format => {
            if app.focused_block == FocusedBlock::Partitions && app.selected_partition().is_some() {
                app.format_dialog.show_dialog = true;
                app.format_dialog.type_state.select(Some(0));
            } else if app.focused_block == FocusedBlock::Disks && app.selected_disk().is_some() {
                app.format_dialog.show_dialog = true;
                app.format_dialog.type_state.select(Some(0));
            }
        }
        KeyCode::Char('n') | KeyCode::Char('N') => {
            if app.focused_block == FocusedBlock::Disks {
                if let Some(disk) = app.selected_disk() {
                    if disk.device.partitions.len() == 1
                        && disk.device.partitions[0].name == disk.device.name
                    {
                        use crate::notification::{Notification, NotificationLevel};
                        let _ = Notification::send(
                            format!(
                                "No partition table on {}. Press 'p' to create one first.",
                                disk.device.name
                            ),
                            NotificationLevel::Error,
                            &sender,
                        );
                    } else {
                        let used_space: u64 = disk.device.partitions.iter().map(|p| p.size).sum();
                        let free_space = disk.device.size.saturating_sub(used_space);
                        if free_space > 0 {
                            app.partition_dialog.show_dialog = true;
                            app.partition_dialog.mode = PartitionDialogMode::CreatePartition;
                            app.partition_dialog.create_step =
                                crate::app::CreatePartitionStep::EnterSize;
                            app.partition_dialog.size_input = tui_input::Input::default();
                            app.partition_dialog.new_partition_fs_state.select(Some(0));
                        }
                    }
                }
            }
        }
        KeyCode::Char(c) if c == config.disk.partition => {
            if app.focused_block == FocusedBlock::Disks && app.selected_disk().is_some() {
                app.partition_dialog.show_dialog = true;
                app.partition_dialog.mode = PartitionDialogMode::SelectTableType;
            }
        }
        KeyCode::Char(c) if c == config.disk.mount => {
            if app.focused_block == FocusedBlock::Partitions {
                if let Some(partition) = app.selected_partition() {
                    // Check if another operation is in progress
                    if check_operation_in_progress(app, &sender) {
                        return Ok(());
                    }

                    app.operation_in_progress.store(true, Ordering::Release);
                    let part_name = partition.name.clone();
                    let is_mounted = partition.is_mounted;
                    let sender_clone = sender.clone();
                    let operation_flag = app.operation_in_progress.clone();
                    tokio::spawn(async move {
                        if is_mounted {
                            let _ = unmount_partition(&part_name, &sender_clone).await;
                        } else {
                            let _ = mount_partition(&part_name, &sender_clone).await;
                        }
                        let _ = sender_clone.send(Event::Refresh);
                        operation_flag.store(false, Ordering::Release);
                    });
                }
            }
        }
        KeyCode::Char(c) if c == config.disk.delete => {
            use crate::app::ConfirmationOperation;
            use crate::utils::format_bytes;

            if app.focused_block == FocusedBlock::Partitions {
                if let Some(partition) = app.selected_partition() {
                    let part_name = partition.name.clone();
                    let part_size = format_bytes(partition.size);
                    let filesystem = partition
                        .filesystem
                        .clone()
                        .unwrap_or_else(|| "none".to_string());
                    let mount_status = if partition.is_mounted {
                        format!(
                            "Yes ({})",
                            partition.mount_point.clone().unwrap_or_default()
                        )
                    } else {
                        "No".to_string()
                    };

                    app.confirmation_dialog = crate::app::ConfirmationDialog {
                        show_dialog: true,
                        title: "Confirm Delete Partition".to_string(),
                        message: "Are you sure you want to delete this partition?".to_string(),
                        details: vec![
                            ("Partition".to_string(), part_name.clone()),
                            ("Size".to_string(), part_size),
                            ("Filesystem".to_string(), filesystem),
                            ("Mounted".to_string(), mount_status),
                        ],
                        selected: 0,
                        operation: ConfirmationOperation::DeletePartition {
                            partition: part_name,
                        },
                    };
                }
            }
        }
        _ => {}
    }

    Ok(())
}

async fn handle_format_dialog(
    key_event: KeyEvent,
    app: &mut App,
    _sender: UnboundedSender<Event>,
) -> AppResult<()> {
    match key_event.code {
        KeyCode::Esc => {
            app.format_dialog.show_dialog = false;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if let Some(i) = app.format_dialog.type_state.selected() {
                if i < app.filesystem_types.len() - 1 {
                    app.format_dialog.type_state.select(Some(i + 1));
                }
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let Some(i) = app.format_dialog.type_state.selected() {
                if i > 0 {
                    app.format_dialog.type_state.select(Some(i - 1));
                }
            }
        }
        KeyCode::Enter => {
            use crate::app::ConfirmationOperation;
            use crate::utils::format_bytes;

            if let Some(fs_idx) = app.format_dialog.type_state.selected() {
                let fs_type = app.filesystem_types[fs_idx].clone();
                app.format_dialog.show_dialog = false;

                if app.focused_block == FocusedBlock::Partitions {
                    if let Some(partition) = app.selected_partition() {
                        let part_name = partition.name.clone();
                        let part_size = format_bytes(partition.size);
                        let current_fs = partition
                            .filesystem
                            .clone()
                            .unwrap_or_else(|| "none".to_string());

                        app.confirmation_dialog = crate::app::ConfirmationDialog {
                            show_dialog: true,
                            title: "Confirm Format Partition".to_string(),
                            message: "Are you sure you want to format this partition?".to_string(),
                            details: vec![
                                ("Partition".to_string(), part_name.clone()),
                                ("Size".to_string(), part_size),
                                ("Current Filesystem".to_string(), current_fs),
                                ("New Filesystem".to_string(), fs_type.to_string()),
                            ],
                            selected: 0,
                            operation: ConfirmationOperation::FormatPartition {
                                partition: part_name,
                                fs_type,
                            },
                        };
                    }
                } else if app.focused_block == FocusedBlock::Disks {
                    if let Some(disk) = app.selected_disk() {
                        let disk_name = disk.device.name.clone();
                        let disk_size = format_bytes(disk.device.size);
                        let disk_model = disk
                            .device
                            .model
                            .clone()
                            .unwrap_or_else(|| "N/A".to_string());

                        app.confirmation_dialog = crate::app::ConfirmationDialog {
                            show_dialog: true,
                            title: "Confirm Format Entire Disk".to_string(),
                            message: "Are you sure you want to format this ENTIRE DISK?"
                                .to_string(),
                            details: vec![
                                ("Disk".to_string(), disk_name.clone()),
                                ("Size".to_string(), disk_size),
                                ("Model".to_string(), disk_model),
                                ("New Filesystem".to_string(), fs_type.to_string()),
                            ],
                            selected: 0,
                            operation: ConfirmationOperation::FormatDisk {
                                disk: disk_name,
                                fs_type,
                            },
                        };
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

async fn handle_partition_dialog(
    key_event: KeyEvent,
    app: &mut App,
    _sender: UnboundedSender<Event>,
) -> AppResult<()> {
    use crate::app::{CreatePartitionStep, PartitionDialogMode};

    match key_event.code {
        KeyCode::Esc => {
            app.partition_dialog.show_dialog = false;
        }
        KeyCode::Tab => {
            if app.partition_dialog.mode == PartitionDialogMode::SelectTableType {
                app.partition_dialog.mode = PartitionDialogMode::CreatePartition;
                app.partition_dialog.create_step = CreatePartitionStep::EnterSize;
            }
        }
        KeyCode::Backspace => {
            if app.partition_dialog.mode == PartitionDialogMode::CreatePartition
                && app.partition_dialog.create_step == CreatePartitionStep::SelectFilesystem
            {
                app.partition_dialog.create_step = CreatePartitionStep::EnterSize;
            }
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if app.partition_dialog.mode == PartitionDialogMode::SelectTableType {
                if let Some(i) = app.partition_dialog.table_type_state.selected() {
                    if i < app.partition_dialog.table_types.len() - 1 {
                        app.partition_dialog.table_type_state.select(Some(i + 1));
                    }
                }
            } else if app.partition_dialog.create_step == CreatePartitionStep::SelectFilesystem {
                if let Some(i) = app.partition_dialog.new_partition_fs_state.selected() {
                    if i < app.filesystem_types.len() - 1 {
                        app.partition_dialog
                            .new_partition_fs_state
                            .select(Some(i + 1));
                    }
                }
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if app.partition_dialog.mode == PartitionDialogMode::SelectTableType {
                if let Some(i) = app.partition_dialog.table_type_state.selected() {
                    if i > 0 {
                        app.partition_dialog.table_type_state.select(Some(i - 1));
                    }
                }
            } else if app.partition_dialog.create_step == CreatePartitionStep::SelectFilesystem {
                if let Some(i) = app.partition_dialog.new_partition_fs_state.selected() {
                    if i > 0 {
                        app.partition_dialog
                            .new_partition_fs_state
                            .select(Some(i - 1));
                    }
                }
            }
        }
        KeyCode::Enter => {
            use crate::app::ConfirmationOperation;
            use crate::utils::format_bytes;

            if app.partition_dialog.mode == PartitionDialogMode::SelectTableType {
                if let Some(disk) = app.selected_disk() {
                    if let Some(table_idx) = app.partition_dialog.table_type_state.selected() {
                        let disk_name = disk.device.name.clone();
                        let disk_size = format_bytes(disk.device.size);
                        let disk_model = disk
                            .device
                            .model
                            .clone()
                            .unwrap_or_else(|| "N/A".to_string());
                        let table_type = app.partition_dialog.table_types[table_idx].clone();
                        let partitions_count = disk.device.partitions.len();

                        app.partition_dialog.show_dialog = false;

                        app.confirmation_dialog = crate::app::ConfirmationDialog {
                            show_dialog: true,
                            title: "Confirm Create Partition Table".to_string(),
                            message: "This will ERASE ALL DATA and create a new partition table!"
                                .to_string(),
                            details: vec![
                                ("Disk".to_string(), disk_name.clone()),
                                ("Size".to_string(), disk_size),
                                ("Model".to_string(), disk_model),
                                ("Table Type".to_string(), table_type.clone().to_uppercase()),
                                (
                                    "Current Partitions".to_string(),
                                    format!("{} (will be deleted)", partitions_count),
                                ),
                            ],
                            selected: 0,
                            operation: ConfirmationOperation::CreatePartitionTable {
                                disk: disk_name,
                                table_type,
                            },
                        };
                    }
                }
            } else if app.partition_dialog.mode == PartitionDialogMode::CreatePartition {
                if app.partition_dialog.create_step == CreatePartitionStep::EnterSize {
                    app.partition_dialog.create_step = CreatePartitionStep::SelectFilesystem;
                } else if let (Some(disk), Some(fs_idx)) = (
                    app.selected_disk(),
                    app.partition_dialog.new_partition_fs_state.selected(),
                ) {
                    let disk_name = disk.device.name.clone();
                    let disk_size = format_bytes(disk.device.size);
                    let size_str = app.partition_dialog.size_input.value().to_string();
                    let fs_type = app.filesystem_types[fs_idx].clone();

                    let used_space: u64 = disk.device.partitions.iter().map(|p| p.size).sum();
                    let free_space = disk.device.size.saturating_sub(used_space);
                    let free_space_str = format_bytes(free_space);

                    let display_size = if size_str.trim().is_empty() {
                        format!("{} (all available)", free_space_str)
                    } else {
                        size_str.clone()
                    };

                    app.partition_dialog.show_dialog = false;

                    app.confirmation_dialog = crate::app::ConfirmationDialog {
                        show_dialog: true,
                        title: "Confirm Create Partition".to_string(),
                        message: "Create new partition with the following settings?".to_string(),
                        details: vec![
                            ("Disk".to_string(), disk_name.clone()),
                            ("Disk Size".to_string(), disk_size),
                            ("Available Space".to_string(), free_space_str),
                            ("New Partition Size".to_string(), display_size),
                            ("Filesystem".to_string(), fs_type.to_string()),
                        ],
                        selected: 0,
                        operation: ConfirmationOperation::CreatePartition {
                            disk: disk_name,
                            size: size_str,
                            fs_type,
                        },
                    };
                }
            }
        }
        _ => {
            if app.partition_dialog.mode == PartitionDialogMode::CreatePartition
                && app.partition_dialog.create_step == CreatePartitionStep::EnterSize
            {
                app.partition_dialog
                    .size_input
                    .handle_event(&crossterm::event::Event::Key(key_event));
            }
        }
    }
    Ok(())
}

fn handle_scroll_down(app: &mut App) {
    match app.focused_block {
        FocusedBlock::Disks => {
            if !app.disks.is_empty() {
                let i = match app.disks_state.selected() {
                    Some(i) => {
                        if i < app.disks.len() - 1 {
                            i + 1
                        } else {
                            i
                        }
                    }
                    None => 0,
                };
                app.disks_state.select(Some(i));
                if !app.disks[i].device.partitions.is_empty() {
                    app.partitions_state.select(Some(0));
                } else {
                    app.partitions_state.select(None);
                }
            }
        }
        FocusedBlock::Partitions => {
            if let Some(disk) = app.selected_disk() {
                if !disk.device.partitions.is_empty() {
                    let i = match app.partitions_state.selected() {
                        Some(i) => {
                            if i < disk.device.partitions.len() - 1 {
                                i + 1
                            } else {
                                i
                            }
                        }
                        None => 0,
                    };
                    app.partitions_state.select(Some(i));
                }
            }
        }
        _ => {}
    }
}

fn handle_scroll_up(app: &mut App) {
    match app.focused_block {
        FocusedBlock::Disks => {
            if !app.disks.is_empty() {
                let i = match app.disks_state.selected() {
                    Some(i) => i.saturating_sub(1),
                    None => 0,
                };
                app.disks_state.select(Some(i));
                if !app.disks[i].device.partitions.is_empty() {
                    app.partitions_state.select(Some(0));
                } else {
                    app.partitions_state.select(None);
                }
            }
        }
        FocusedBlock::Partitions => {
            if let Some(disk) = app.selected_disk() {
                if !disk.device.partitions.is_empty() {
                    let i = match app.partitions_state.selected() {
                        Some(i) => i.saturating_sub(1),
                        None => 0,
                    };
                    app.partitions_state.select(Some(i));
                }
            }
        }
        _ => {}
    }
}

async fn handle_confirmation_dialog(
    key_event: KeyEvent,
    app: &mut App,
    sender: UnboundedSender<Event>,
) -> AppResult<()> {
    use crate::app::ConfirmationOperation;

    match key_event.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
            app.confirmation_dialog.show_dialog = false;
            app.confirmation_dialog.operation = ConfirmationOperation::None;
        }
        KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l') => {
            app.confirmation_dialog.selected = 1 - app.confirmation_dialog.selected;
        }
        KeyCode::Enter => {
            if app.confirmation_dialog.selected == 1 {
                let operation = app.confirmation_dialog.operation.clone();
                app.confirmation_dialog.show_dialog = false;
                app.confirmation_dialog.operation = ConfirmationOperation::None;

                match operation {
                    ConfirmationOperation::FormatPartition { partition, fs_type } => {
                        if check_operation_in_progress(app, &sender) {
                            return Ok(());
                        }
                        app.operation_in_progress.store(true, Ordering::Release);
                        let sender_clone = sender.clone();
                        let operation_flag = app.operation_in_progress.clone();
                        tokio::spawn(async move {
                            let _ =
                                format_partition(&partition, fs_type, sender_clone.clone()).await;
                            let _ = sender_clone.send(Event::Refresh);
                            operation_flag.store(false, Ordering::Release);
                        });
                    }
                    ConfirmationOperation::FormatDisk { disk, fs_type } => {
                        if check_operation_in_progress(app, &sender) {
                            return Ok(());
                        }
                        app.operation_in_progress.store(true, Ordering::Release);
                        let sender_clone = sender.clone();
                        let operation_flag = app.operation_in_progress.clone();
                        tokio::spawn(async move {
                            let _ = format_whole_disk(&disk, fs_type, sender_clone.clone()).await;
                            let _ = sender_clone.send(Event::Refresh);
                            operation_flag.store(false, Ordering::Release);
                        });
                    }
                    ConfirmationOperation::DeletePartition { partition } => {
                        if check_operation_in_progress(app, &sender) {
                            return Ok(());
                        }
                        app.operation_in_progress.store(true, Ordering::Release);
                        let sender_clone = sender.clone();
                        let operation_flag = app.operation_in_progress.clone();
                        tokio::spawn(async move {
                            let _ = delete_partition(&partition, &sender_clone).await;
                            let _ = sender_clone.send(Event::Refresh);
                            operation_flag.store(false, Ordering::Release);
                        });
                    }
                    ConfirmationOperation::CreatePartitionTable { disk, table_type } => {
                        if check_operation_in_progress(app, &sender) {
                            return Ok(());
                        }
                        app.operation_in_progress.store(true, Ordering::Release);
                        let sender_clone = sender.clone();
                        let operation_flag = app.operation_in_progress.clone();
                        tokio::spawn(async move {
                            let _ = create_partition_table(&disk, &table_type, &sender_clone).await;
                            let _ = sender_clone.send(Event::Refresh);
                            operation_flag.store(false, Ordering::Release);
                        });
                    }
                    ConfirmationOperation::CreatePartition {
                        disk,
                        size,
                        fs_type,
                    } => {
                        if check_operation_in_progress(app, &sender) {
                            return Ok(());
                        }
                        app.operation_in_progress.store(true, Ordering::Release);
                        let sender_clone = sender.clone();
                        let operation_flag = app.operation_in_progress.clone();
                        tokio::spawn(async move {
                            let _ = create_partition_with_fs(&disk, &size, fs_type, &sender_clone)
                                .await;
                            let _ = sender_clone.send(Event::Refresh);
                            operation_flag.store(false, Ordering::Release);
                        });
                    }
                    ConfirmationOperation::None => {}
                }
            } else {
                app.confirmation_dialog.show_dialog = false;
                app.confirmation_dialog.operation = ConfirmationOperation::None;
            }
        }
        _ => {}
    }
    Ok(())
}
