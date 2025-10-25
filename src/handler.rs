use std::sync::Arc;
use std::sync::atomic::Ordering;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc::UnboundedSender;
use tui_input::backend::crossterm::EventHandler;

use crate::app::{App, AppResult, FocusedBlock, PartitionDialogMode};
use crate::config::Config;
use crate::event::Event;
use crate::notification::{Notification, NotificationLevel};
use crate::operations::{
    format_partition, format_whole_disk, create_partition_table, create_partition_with_fs, delete_partition,
    mount_partition, unmount_partition,
};

/// Check if an operation is in progress and send a warning notification if so.
/// Returns true if operation is in progress (and caller should return early).
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
            if app.focused_block == FocusedBlock::Disks || app.focused_block == FocusedBlock::Partitions {
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
            if app.focused_block == FocusedBlock::Partitions
                && app.selected_partition().is_some() {
                app.format_dialog.show_dialog = true;
                app.format_dialog.type_state.select(Some(0));
            } else if app.focused_block == FocusedBlock::Disks
                && app.selected_disk().is_some() {
                app.format_dialog.show_dialog = true;
                app.format_dialog.type_state.select(Some(0));
            }
        }
        KeyCode::Char('n') | KeyCode::Char('N') => {
            if app.focused_block == FocusedBlock::Disks {
                if let Some(disk) = app.selected_disk() {
                    // Check if disk has no partition table (showing whole disk as single partition)
                    if disk.device.partitions.len() == 1 && disk.device.partitions[0].name == disk.device.name {
                        use crate::notification::{Notification, NotificationLevel};
                        let _ = Notification::send(
                            format!("No partition table on {}. Press 'p' to create one first.", disk.device.name),
                            NotificationLevel::Error,
                            &sender,
                        );
                    } else {
                        let used_space: u64 = disk.device.partitions.iter().map(|p| p.size).sum();
                        let free_space = disk.device.size.saturating_sub(used_space);
                        if free_space > 0 {
                            app.partition_dialog.show_dialog = true;
                            app.partition_dialog.mode = PartitionDialogMode::CreatePartition;
                            app.partition_dialog.create_step = crate::app::CreatePartitionStep::EnterSize;
                            app.partition_dialog.size_input = tui_input::Input::default();
                            app.partition_dialog.new_partition_fs_state.select(Some(0));
                        }
                    }
                }
            }
        }
        KeyCode::Char(c) if c == config.disk.partition => {
            if app.focused_block == FocusedBlock::Disks
                && app.selected_disk().is_some() {
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
            if app.focused_block == FocusedBlock::Partitions {
                if let Some(partition) = app.selected_partition() {
                    // Check if another operation is in progress
                    if check_operation_in_progress(app, &sender) {
                        return Ok(());
                    }

                    app.operation_in_progress.store(true, Ordering::Release);
                    let part_name = partition.name.clone();
                    let sender_clone = sender.clone();
                    let operation_flag = app.operation_in_progress.clone();
                    tokio::spawn(async move {
                        let _ = delete_partition(&part_name, &sender_clone).await;
                        let _ = sender_clone.send(Event::Refresh);
                        operation_flag.store(false, Ordering::Release);
                    });
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
    sender: UnboundedSender<Event>,
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
            if let Some(fs_idx) = app.format_dialog.type_state.selected() {
                let fs_type = app.filesystem_types[fs_idx].clone();
                let sender_clone = sender.clone();
                let operation_flag = app.operation_in_progress.clone();
                app.operation_in_progress.store(true, Ordering::Release);
                app.format_dialog.show_dialog = false;

                if app.focused_block == FocusedBlock::Partitions {
                    if let Some(partition) = app.selected_partition() {
                        let part_name = partition.name.clone();
                        tokio::spawn(async move {
                            let _ = format_partition(&part_name, fs_type, sender_clone.clone()).await;
                            let _ = sender_clone.send(Event::Refresh);
                            operation_flag.store(false, Ordering::Release);
                        });
                    }
                } else if app.focused_block == FocusedBlock::Disks {
                    if let Some(disk) = app.selected_disk() {
                        let disk_name = disk.device.name.clone();
                        tokio::spawn(async move {
                            let _ = format_whole_disk(&disk_name, fs_type, sender_clone.clone()).await;
                            let _ = sender_clone.send(Event::Refresh);
                            operation_flag.store(false, Ordering::Release);
                        });
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
    sender: UnboundedSender<Event>,
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
                && app.partition_dialog.create_step == CreatePartitionStep::SelectFilesystem {
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
                        app.partition_dialog.new_partition_fs_state.select(Some(i + 1));
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
                        app.partition_dialog.new_partition_fs_state.select(Some(i - 1));
                    }
                }
            }
        }
        KeyCode::Enter => {
            if app.partition_dialog.mode == PartitionDialogMode::SelectTableType {
                if let Some(disk) = app.selected_disk() {
                    if let Some(table_idx) = app.partition_dialog.table_type_state.selected() {
                        // Check if another operation is in progress
                        if check_operation_in_progress(app, &sender) {
                            return Ok(());
                        }

                        app.operation_in_progress.store(true, Ordering::Release);
                        let disk_name = disk.device.name.clone();
                        let table_type = app.partition_dialog.table_types[table_idx].clone();
                        let sender_clone = sender.clone();
                        let operation_flag = app.operation_in_progress.clone();
                        app.partition_dialog.show_dialog = false;
                        tokio::spawn(async move {
                            let _ = create_partition_table(&disk_name, &table_type, &sender_clone).await;
                            let _ = sender_clone.send(Event::Refresh);
                            operation_flag.store(false, Ordering::Release);
                        });
                    }
                }
            } else if app.partition_dialog.mode == PartitionDialogMode::CreatePartition {
                if app.partition_dialog.create_step == CreatePartitionStep::EnterSize {
                    app.partition_dialog.create_step = CreatePartitionStep::SelectFilesystem;
                } else if let (Some(disk), Some(fs_idx)) = (app.selected_disk(), app.partition_dialog.new_partition_fs_state.selected()) {
                    // Check if another operation is in progress
                    if check_operation_in_progress(app, &sender) {
                        return Ok(());
                    }

                    app.operation_in_progress.store(true, Ordering::Release);
                    let disk_name = disk.device.name.clone();
                    let size_str = app.partition_dialog.size_input.value().to_string();
                    let fs_type = app.filesystem_types[fs_idx].clone();
                    let sender_clone = sender.clone();
                    let operation_flag = app.operation_in_progress.clone();
                    app.partition_dialog.show_dialog = false;

                    // Don't send notification - progress dialog will show the message

                    tokio::spawn(async move {
                        let _ = create_partition_with_fs(&disk_name, &size_str, fs_type, &sender_clone).await;
                        let _ = sender_clone.send(Event::Refresh);
                        operation_flag.store(false, Ordering::Release);
                    });
                }
            }
        }
        _ => {
            if app.partition_dialog.mode == PartitionDialogMode::CreatePartition
                && app.partition_dialog.create_step == CreatePartitionStep::EnterSize {
                app.partition_dialog.size_input.handle_event(&crossterm::event::Event::Key(key_event));
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

