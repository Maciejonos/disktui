use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Flex, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Clear, List, ListItem, Paragraph, Row, Table},
};

use crate::app::{App, FocusedBlock, PartitionDialogMode};
use crate::utils::format_bytes;
use ratatui::widgets::Wrap;

pub fn render(app: &mut App, frame: &mut Frame) {
    if app.show_help {
        render_help_dialog(frame);
    } else if app.progress.show_dialog {
        render_main(app, frame);
        render_progress_dialog(app, frame);
    } else if app.passphrase_dialog.show_dialog {
        render_main(app, frame);
        render_passphrase_dialog(app, frame);
    } else if app.confirmation_dialog.show_dialog {
        render_main(app, frame);
        render_confirmation_dialog(app, frame);
    } else if app.format_dialog.show_dialog {
        render_main(app, frame);
        render_format_dialog(app, frame);
    } else if app.partition_dialog.show_dialog {
        render_main(app, frame);
        render_partition_dialog(app, frame);
    } else if app.resize_dialog.show_dialog {
        render_main(app, frame);
        render_resize_dialog(app, frame);
    } else if app.focused_block == FocusedBlock::DiskInfo {
        render_main(app, frame);
        render_disk_info(app, frame);
    } else {
        render_main(app, frame);
    }

    for (index, notification) in app.notifications.iter().enumerate() {
        notification.render(index, frame);
    }
}

fn render_main(app: &mut App, frame: &mut Frame) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(8),
            Constraint::Min(8),
            Constraint::Length(6),
            Constraint::Length(1),
        ])
        .split(frame.area());

    render_disks_table(app, frame, chunks[0]);
    render_partitions_table(app, frame, chunks[1]);
    render_disk_summary(app, frame, chunks[2]);
    render_context_help(app, frame, chunks[3]);
}

fn render_disks_table(app: &mut App, frame: &mut Frame, area: Rect) {
    let header_color = if app.focused_block == FocusedBlock::Disks {
        app.theme.header
    } else {
        Color::Reset
    };
    let header = Row::new(vec![
        Cell::from("Name").style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(header_color),
        ),
        Cell::from("Size").style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(header_color),
        ),
        Cell::from("Type").style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(header_color),
        ),
        Cell::from("Model").style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(header_color),
        ),
        Cell::from("Serial").style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(header_color),
        ),
    ])
    .bottom_margin(1);

    let rows: Vec<Row> = app
        .disks
        .iter()
        .map(|disk| {
            Row::new(vec![
                Cell::from(disk.device.name.clone()),
                Cell::from(disk.size_str()),
                Cell::from(disk.device_type()),
                Cell::from(
                    disk.device
                        .model
                        .clone()
                        .unwrap_or_else(|| "N/A".to_string()),
                ),
                Cell::from(
                    disk.device
                        .serial
                        .clone()
                        .unwrap_or_else(|| "N/A".to_string()),
                ),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(app.theme.disk_name_width),
        Constraint::Length(app.theme.disk_size_width),
        Constraint::Length(app.theme.disk_type_width),
        Constraint::Length(app.theme.disk_model_width),
        Constraint::Length(app.theme.disk_serial_width),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .title(" Disks ")
                .borders(Borders::ALL)
                .border_style(if app.focused_block == FocusedBlock::Disks {
                    Style::default().fg(app.theme.focus_border)
                } else {
                    Style::default().fg(app.theme.normal_border)
                })
                .border_type(if app.focused_block == FocusedBlock::Disks {
                    BorderType::Thick
                } else {
                    BorderType::default()
                }),
        )
        .column_spacing(2)
        .style(Style::default())
        .row_highlight_style(if app.focused_block == FocusedBlock::Disks {
            Style::default()
                .bg(app.theme.highlight_bg)
                .fg(app.theme.highlight_fg)
        } else {
            Style::default()
        });

    frame.render_stateful_widget(table, area, &mut app.disks_state);
}

fn render_partitions_table(app: &mut App, frame: &mut Frame, area: Rect) {
    let header_color = if app.focused_block == FocusedBlock::Partitions {
        app.theme.header
    } else {
        Color::Reset
    };
    let header = Row::new(vec![
        Cell::from("Name").style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(header_color),
        ),
        Cell::from("Size").style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(header_color),
        ),
        Cell::from("Filesystem").style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(header_color),
        ),
        Cell::from("Mount Point").style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(header_color),
        ),
        Cell::from("Label").style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(header_color),
        ),
        Cell::from("Usage").style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(header_color),
        ),
    ])
    .bottom_margin(1);

    let rows: Vec<Row> = if let Some(disk) = app.selected_disk() {
        disk.device
            .partitions
            .iter()
            .map(|part| {
                let name_display = if part.is_encrypted {
                    if part.mapper_device.is_some() {
                        format!("🔓 {}", part.name)
                    } else {
                        format!("🔒 {}", part.name)
                    }
                } else {
                    part.name.clone()
                };

                let filesystem_display = if part.is_encrypted && part.mapper_device.is_none() {
                    part.encryption_type
                        .clone()
                        .unwrap_or_else(|| "LUKS".to_string())
                } else {
                    part.filesystem.clone().unwrap_or_else(|| "N/A".to_string())
                };

                Row::new(vec![
                    Cell::from(name_display),
                    Cell::from(part.size_str()),
                    Cell::from(filesystem_display),
                    Cell::from(part.mount_point.clone().unwrap_or_else(|| "-".to_string())),
                    Cell::from(part.label.clone().unwrap_or_else(|| "-".to_string())),
                    Cell::from(part.usage_str(
                        app.theme.usage_bar_filled,
                        app.theme.usage_bar_empty,
                        app.theme.usage_bar_length,
                    )),
                ])
            })
            .collect()
    } else {
        vec![]
    };

    let title = if let Some(disk) = app.selected_disk() {
        if disk.device.partitions.len() == 1 && disk.device.partitions[0].name == disk.device.name {
            format!(" {} (whole disk - no partition table) ", disk.device.name)
        } else {
            format!(" Partitions of {} ", disk.device.name)
        }
    } else {
        " Partitions ".to_string()
    };

    let widths = [
        Constraint::Length(app.theme.partition_name_width),
        Constraint::Length(app.theme.partition_size_width),
        Constraint::Length(app.theme.partition_fs_width),
        Constraint::Length(app.theme.partition_mount_width),
        Constraint::Length(app.theme.partition_label_width),
        Constraint::Min(app.theme.partition_usage_min_width),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(if app.focused_block == FocusedBlock::Partitions {
                    Style::default().fg(app.theme.focus_border)
                } else {
                    Style::default().fg(app.theme.normal_border)
                })
                .border_type(if app.focused_block == FocusedBlock::Partitions {
                    BorderType::Thick
                } else {
                    BorderType::default()
                }),
        )
        .column_spacing(2)
        .style(Style::default())
        .row_highlight_style(if app.focused_block == FocusedBlock::Partitions {
            Style::default()
                .bg(app.theme.highlight_bg)
                .fg(app.theme.highlight_fg)
        } else {
            Style::default()
        });

    frame.render_stateful_widget(table, area, &mut app.partitions_state);
}

fn render_disk_summary(app: &App, frame: &mut Frame, area: Rect) {
    let text = if let Some(disk) = app.selected_disk() {
        let model = disk
            .device
            .model
            .clone()
            .unwrap_or_else(|| "N/A".to_string());
        let size = disk.size_str();
        let dtype = disk.device_type();
        let smart = disk
            .smart_data
            .as_ref()
            .map(|s| s.health.clone())
            .unwrap_or_else(|| "N/A".to_string());
        let temp = disk
            .smart_data
            .as_ref()
            .and_then(|s| s.temperature)
            .map(|t| format!("{}°C", t))
            .unwrap_or_else(|| "N/A".to_string());

        let layout_bar = generate_layout_bar(disk);

        format!(
            "Model: {} | Size: {} | Type: {} | SMART: {} | Temp: {}\nLayout: {}",
            model, size, dtype, smart, temp, layout_bar
        )
    } else {
        "No disk selected".to_string()
    };

    let paragraph = Paragraph::new(text)
        .block(Block::default().title(" Disk Info ").borders(Borders::ALL))
        .wrap(Wrap { trim: true })
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::White));

    frame.render_widget(paragraph, area);
}

fn generate_layout_bar(disk: &crate::disk::Disk) -> String {
    let total_size = disk.device.size;
    if total_size == 0 {
        return "[ EMPTY ]".to_string();
    }

    let mut parts = Vec::new();

    for partition in &disk.device.partitions {
        parts.push((partition.name.clone(), partition.size));
    }

    let used_space: u64 = parts.iter().map(|(_, size)| size).sum();
    let free_space = total_size.saturating_sub(used_space);

    let mut layout = String::from("[ ");

    for (i, (name, size)) in parts.iter().enumerate() {
        if i > 0 {
            layout.push_str(" | ");
        }
        layout.push_str(&format!("{} ({})", name, format_bytes(*size)));
    }

    if free_space > 0 {
        if !parts.is_empty() {
            layout.push_str(" | ");
        }
        layout.push_str(&format!("FREE ({})", format_bytes(free_space)));
    }

    layout.push_str(" ]");
    layout
}

fn render_context_help(app: &App, frame: &mut Frame, area: Rect) {
    let help_text = match app.focused_block {
        FocusedBlock::Disks => {
            let disk_opt = app.selected_disk();
            let has_selection = disk_opt.is_some();

            let has_free_space = disk_opt
                .map(|d| {
                    let used_space: u64 = d.device.partitions.iter().map(|p| p.size).sum();
                    d.device.size > used_space
                })
                .unwrap_or(false);

            // Check if disk has a partition table (not showing whole disk as single partition)
            let has_partition_table = disk_opt
                .map(|d| {
                    !(d.device.partitions.len() == 1
                        && d.device.partitions[0].name == d.device.name)
                })
                .unwrap_or(false);

            if has_selection {
                let mut spans = vec![
                    Span::from("Tab ").bold().yellow(),
                    Span::from("Switch | "),
                    Span::from("j/k ").bold().yellow(),
                    Span::from("Scroll | "),
                ];

                if has_free_space && has_partition_table {
                    spans.extend_from_slice(&[
                        Span::from("n ").bold().yellow(),
                        Span::from("New Partition | "),
                    ]);
                }

                spans.extend_from_slice(&[
                    Span::from("f ").bold().yellow(),
                    Span::from("Format Disk | "),
                    Span::from("p ").bold().yellow(),
                    Span::from("Partition Table | "),
                    Span::from("i ").bold().yellow(),
                    Span::from("Info | "),
                    Span::from("? ").bold().yellow(),
                    Span::from("Help | "),
                    Span::from("q ").bold().yellow(),
                    Span::from("Quit"),
                ]);

                Line::from(spans)
            } else {
                Line::from(vec![
                    Span::from("Tab ").bold().yellow(),
                    Span::from("Switch | "),
                    Span::from("j/k ").bold().yellow(),
                    Span::from("Select disk | "),
                    Span::from("? ").bold().yellow(),
                    Span::from("Help | "),
                    Span::from("q ").bold().yellow(),
                    Span::from("Quit"),
                ])
            }
        }
        FocusedBlock::Partitions => {
            let has_selection = app.selected_partition().is_some();
            if has_selection {
                let partition = app.selected_partition();
                let is_mounted = partition.as_ref().map(|p| p.is_mounted).unwrap_or(false);
                let is_encrypted = partition.as_ref().map(|p| p.is_encrypted).unwrap_or(false);
                let is_unlocked = partition
                    .as_ref()
                    .and_then(|p| p.mapper_device.as_ref())
                    .is_some();

                let mount_text = if is_mounted { "Unmount" } else { "Mount" };

                let mut spans = vec![
                    Span::from("Tab ").bold().yellow(),
                    Span::from("Switch | "),
                    Span::from("j/k ").bold().yellow(),
                    Span::from("Scroll | "),
                ];

                if is_encrypted {
                    let lock_text = if is_unlocked { "Lock" } else { "Unlock" };
                    spans.extend_from_slice(&[
                        Span::from("l ").bold().yellow(),
                        Span::from(format!("{} | ", lock_text)),
                    ]);
                } else {
                    spans.extend_from_slice(&[
                        Span::from("e ").bold().yellow(),
                        Span::from("Encrypt | "),
                    ]);
                }

                if !is_encrypted || is_unlocked {
                    spans.extend_from_slice(&[
                        Span::from("f ").bold().yellow(),
                        Span::from("Format | "),
                    ]);
                }

                if is_unlocked || !is_encrypted {
                    spans.extend_from_slice(&[
                        Span::from("m ").bold().yellow(),
                        Span::from(format!("{} | ", mount_text)),
                    ]);
                }

                if !is_mounted && !is_encrypted {
                    spans.extend_from_slice(&[
                        Span::from("r ").bold().yellow(),
                        Span::from("Resize | "),
                    ]);
                }

                spans.extend_from_slice(&[
                    Span::from("d ").bold().yellow(),
                    Span::from("Delete | "),
                    Span::from("? ").bold().yellow(),
                    Span::from("Help | "),
                    Span::from("q ").bold().yellow(),
                    Span::from("Quit"),
                ]);

                Line::from(spans)
            } else {
                Line::from(vec![
                    Span::from("Tab ").bold().yellow(),
                    Span::from("Switch | "),
                    Span::from("j/k ").bold().yellow(),
                    Span::from("Select partition | "),
                    Span::from("? ").bold().yellow(),
                    Span::from("Help | "),
                    Span::from("q ").bold().yellow(),
                    Span::from("Quit"),
                ])
            }
        }
        _ => Line::from(vec![
            Span::from("? ").bold().yellow(),
            Span::from("Help | "),
            Span::from("q ").bold().yellow(),
            Span::from("Quit"),
        ]),
    };

    frame.render_widget(help_text.centered(), area);
}

fn render_help_dialog(frame: &mut Frame) {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(35),
            Constraint::Fill(1),
        ])
        .flex(Flex::SpaceBetween)
        .split(frame.area());

    let area = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(70),
            Constraint::Fill(1),
        ])
        .flex(Flex::SpaceBetween)
        .split(popup_layout[1])[1];

    let help_text = vec![
        Line::from("Navigation:").bold().yellow(),
        Line::from("  Tab/Shift+Tab  - Navigate between blocks"),
        Line::from("  j/Down         - Scroll down"),
        Line::from("  k/Up           - Scroll up"),
        Line::from(""),
        Line::from("Disk Operations (focus on Partitions):")
            .bold()
            .yellow(),
        Line::from("  f  - Format partition/disk"),
        Line::from("  m  - Mount/unmount"),
        Line::from("  r  - Resize partition (unmounted only)"),
        Line::from("  d  - Delete partition"),
        Line::from(""),
        Line::from("Disk Operations (focus on Disks):")
            .bold()
            .yellow(),
        Line::from("  p  - Partition (create table/partition)"),
        Line::from("  i  - Show disk SMART info"),
        Line::from(""),
        Line::from("Workflow for USB with ISO:").bold().yellow(),
        Line::from("  1. Tab to Partitions, press 'm' to unmount"),
        Line::from("  2. Tab to Disks, press 'p' to create partition table"),
        Line::from("  3. Select GPT/MBR, press Enter"),
        Line::from("  4. Tab to create partition, adjust size, Enter"),
        Line::from("  5. Tab to Partitions, select partition, press 'f'"),
        Line::from(""),
        Line::from("Other:").bold().yellow(),
        Line::from("  ?  - Toggle this help | q  - Quit"),
        Line::from(""),
        Line::from("Press any key to close").centered().italic(),
    ];

    let block = Paragraph::new(help_text)
        .block(
            Block::default()
                .title(" Disk Utility TUI - Help ")
                .title_alignment(Alignment::Center)
                .borders(Borders::ALL)
                .border_type(BorderType::Thick)
                .border_style(Style::default().fg(Color::Green)),
        )
        .style(Style::default().fg(Color::White));

    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
}

fn render_format_dialog(app: &mut App, frame: &mut Frame) {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(30),
            Constraint::Length(15),
            Constraint::Percentage(30),
        ])
        .split(frame.area());

    let area = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(40),
            Constraint::Fill(1),
        ])
        .split(popup_layout[1])[1];

    let part_name = app
        .selected_partition()
        .map(|p| p.name.clone())
        .unwrap_or_default();

    let title = if app.format_dialog.encrypt_mode {
        format!(" Encrypt {} - Select Filesystem ", part_name)
    } else {
        format!(" Format {} - Select Filesystem ", part_name)
    };

    let items: Vec<ListItem> = app
        .filesystem_types
        .iter()
        .map(|fs| ListItem::new(fs.as_str()))
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title(title)
                .title_alignment(Alignment::Center)
                .borders(Borders::ALL)
                .border_type(BorderType::Thick)
                .border_style(Style::default().fg(Color::Green)),
        )
        .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White));

    let warning = Paragraph::new("WARNING: All data will be lost!\n\nEnter: Confirm | Esc: Cancel")
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Fill(1), Constraint::Length(3)])
        .split(area);

    frame.render_widget(Clear, area);
    frame.render_stateful_widget(list, chunks[0], &mut app.format_dialog.type_state);
    frame.render_widget(warning, chunks[1]);
}

fn render_partition_dialog(app: &mut App, frame: &mut Frame) {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Length(18),
            Constraint::Percentage(25),
        ])
        .split(frame.area());

    let area = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(50),
            Constraint::Fill(1),
        ])
        .split(popup_layout[1])[1];

    let disk_name = app
        .selected_disk()
        .map(|d| d.device.name.clone())
        .unwrap_or_default();

    frame.render_widget(Clear, area);

    if app.partition_dialog.mode == PartitionDialogMode::SelectTableType {
        let items: Vec<ListItem> = app
            .partition_dialog
            .table_types
            .iter()
            .map(|t| ListItem::new(t.clone()))
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .title(format!(" Create Partition Table on {} ", disk_name))
                    .title_alignment(Alignment::Center)
                    .borders(Borders::ALL)
                    .border_type(BorderType::Thick)
                    .border_style(Style::default().fg(Color::Green)),
            )
            .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White));

        let info = Paragraph::new(
            "Tab: Switch to create partition mode\nEnter: Create table | Esc: Cancel",
        )
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::Yellow));

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Fill(1), Constraint::Length(3)])
            .split(area);

        frame.render_stateful_widget(list, chunks[0], &mut app.partition_dialog.table_type_state);
        frame.render_widget(info, chunks[1]);
    } else {
        use crate::app::CreatePartitionStep;

        let free_space = if let Some(disk) = app.selected_disk() {
            let used_space: u64 = disk.device.partitions.iter().map(|p| p.size).sum();
            disk.device.size.saturating_sub(used_space)
        } else {
            0
        };

        let free_space_str = format_bytes(free_space);

        if app.partition_dialog.create_step == CreatePartitionStep::EnterSize {
            let border_block = Block::default()
                .title(format!(" Create New Partition on {} ", disk_name))
                .title_alignment(Alignment::Center)
                .borders(Borders::ALL)
                .border_type(BorderType::Thick)
                .border_style(Style::default().fg(Color::Green));

            let inner_area = border_block.inner(area);

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1), // Label line
                    Constraint::Length(3), // Input box with border
                    Constraint::Length(1), // Spacing
                    Constraint::Length(2), // Info text (2 lines)
                    Constraint::Fill(1),   // Remaining space
                ])
                .split(inner_area);

            frame.render_widget(Clear, area);
            frame.render_widget(border_block, area);

            let size_label =
                Paragraph::new(format!("Partition Size (Available: {}):", free_space_str));

            let size_input = Paragraph::new(app.partition_dialog.size_input.value())
                .block(Block::default().borders(Borders::ALL));

            let info = Paragraph::new(
                "Examples: 100M, 2.5G, 1T (leave empty for max)\n\
                 Enter: Next | Esc: Cancel",
            )
            .alignment(Alignment::Center);

            frame.render_widget(size_label, chunks[0]);
            frame.render_widget(size_input, chunks[1]);
            frame.render_widget(info, chunks[3]);
        } else {
            let items: Vec<ListItem> = app
                .filesystem_types
                .iter()
                .map(|fs| ListItem::new(fs.to_string()))
                .collect();

            let list = List::new(items)
                .block(
                    Block::default()
                        .title(format!(
                            " Select Filesystem for New Partition (Size: {}) ",
                            app.partition_dialog.size_input.value()
                        ))
                        .title_alignment(Alignment::Center)
                        .borders(Borders::ALL)
                        .border_type(BorderType::Thick)
                        .border_style(Style::default().fg(Color::Green)),
                )
                .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White));

            let info = Paragraph::new(
                "j/k: Navigate | Enter: Create Partition | Backspace: Go Back | Esc: Cancel",
            )
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::Yellow));

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Fill(1), Constraint::Length(3)])
                .split(area);

            frame.render_stateful_widget(
                list,
                chunks[0],
                &mut app.partition_dialog.new_partition_fs_state,
            );
            frame.render_widget(info, chunks[1]);
        }
    }
}

fn render_disk_info(app: &App, frame: &mut Frame) {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(12),
            Constraint::Fill(1),
        ])
        .flex(Flex::Start)
        .split(frame.area());

    let area = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Fill(1),
            Constraint::Min(70),
            Constraint::Fill(1),
        ])
        .split(popup_layout[1])[1];

    if let Some(disk) = app.selected_disk() {
        let rows = vec![
            Row::new(vec![
                Cell::from("Name").style(Style::default().bold().yellow()),
                Cell::from(disk.device.name.clone()),
            ]),
            Row::new(vec![
                Cell::from("Size").style(Style::default().bold().yellow()),
                Cell::from(disk.size_str()),
            ]),
            Row::new(vec![
                Cell::from("Type").style(Style::default().bold().yellow()),
                Cell::from(disk.device_type()),
            ]),
            Row::new(vec![
                Cell::from("Model").style(Style::default().bold().yellow()),
                Cell::from(
                    disk.device
                        .model
                        .clone()
                        .unwrap_or_else(|| "N/A".to_string()),
                ),
            ]),
            Row::new(vec![
                Cell::from("Serial").style(Style::default().bold().yellow()),
                Cell::from(
                    disk.device
                        .serial
                        .clone()
                        .unwrap_or_else(|| "N/A".to_string()),
                ),
            ]),
            Row::new(vec![
                Cell::from("SMART Health").style(Style::default().bold().yellow()),
                Cell::from(
                    disk.smart_data
                        .as_ref()
                        .map(|s| s.health.clone())
                        .unwrap_or_else(|| "N/A".to_string()),
                ),
            ]),
            Row::new(vec![
                Cell::from("Temperature").style(Style::default().bold().yellow()),
                Cell::from(
                    disk.smart_data
                        .as_ref()
                        .and_then(|s| s.temperature)
                        .map(|t| format!("{}°C", t))
                        .unwrap_or_else(|| "N/A".to_string()),
                ),
            ]),
            Row::new(vec![
                Cell::from("Power On Hours").style(Style::default().bold().yellow()),
                Cell::from(
                    disk.smart_data
                        .as_ref()
                        .and_then(|s| s.power_on_hours)
                        .map(|h| format!("{}", h))
                        .unwrap_or_else(|| "N/A".to_string()),
                ),
            ]),
        ];

        let table = Table::new(rows, [Constraint::Length(20), Constraint::Fill(1)]).block(
            Block::default()
                .title(" Disk Information ")
                .title_alignment(Alignment::Center)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green))
                .border_type(BorderType::Thick),
        );

        frame.render_widget(Clear, area);
        frame.render_widget(table, area);
    }
}

fn render_progress_dialog(app: &App, frame: &mut Frame) {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(10),
            Constraint::Fill(1),
        ])
        .split(frame.area());

    let area = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(60),
            Constraint::Fill(1),
        ])
        .split(popup_layout[1])[1];

    let spinner_chars = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    let spinner = spinner_chars[app.progress.spinner_index % spinner_chars.len()];

    let title = if !app.progress.disk_name.is_empty() && !app.progress.disk_model.is_empty() {
        format!(
            " {} /dev/{} ({}) ",
            app.progress.message, app.progress.disk_name, app.progress.disk_model
        )
    } else if !app.progress.disk_name.is_empty() {
        format!(" {} /dev/{} ", app.progress.message, app.progress.disk_name)
    } else {
        format!(" {} ", app.progress.message)
    };

    let border_block = Block::default()
        .title(title)
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(Style::default().fg(Color::Cyan));

    let inner_area = border_block.inner(area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Fill(1),
        ])
        .split(inner_area);

    frame.render_widget(Clear, area);
    frame.render_widget(border_block, area);

    // Centered spinner
    let spinner_text = Paragraph::new(format!("{}", spinner))
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .alignment(Alignment::Center);

    // Status message
    let status_text = Paragraph::new("Please wait while the operation completes...")
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);

    frame.render_widget(spinner_text, chunks[1]);
    frame.render_widget(status_text, chunks[2]);
}

fn render_confirmation_dialog(app: &mut App, frame: &mut Frame) {
    // Calculate dialog height based on content
    let details_count = app.confirmation_dialog.details.len();
    let dialog_height = 10 + details_count as u16 * 1;

    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(dialog_height),
            Constraint::Fill(1),
        ])
        .split(frame.area());

    let area = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(60),
            Constraint::Fill(1),
        ])
        .split(popup_layout[1])[1];

    let border_block = Block::default()
        .title(format!(" {} ", app.confirmation_dialog.title))
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(Style::default().fg(Color::Yellow));

    let inner_area = border_block.inner(area);

    // Build content
    let mut text_lines = vec![
        Line::from(""),
        Line::from(app.confirmation_dialog.message.clone()).style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Line::from(""),
    ];

    // Add details
    if !app.confirmation_dialog.details.is_empty() {
        for (key, value) in &app.confirmation_dialog.details {
            text_lines.push(Line::from(vec![
                Span::styled(
                    format!("{}: ", key),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(value.clone(), Style::default().fg(Color::White)),
            ]));
        }
        text_lines.push(Line::from(""));
    }

    // Warning message
    text_lines.push(
        Line::from("⚠ WARNING: This operation cannot be undone! ⚠")
            .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
            .centered(),
    );
    text_lines.push(Line::from(""));

    // Buttons
    let no_style = if app.confirmation_dialog.selected == 0 {
        Style::default()
            .bg(Color::DarkGray)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    let yes_style = if app.confirmation_dialog.selected == 1 {
        Style::default()
            .bg(Color::Red)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    text_lines.push(
        Line::from(vec![
            Span::raw("  "),
            Span::styled(" No ", no_style),
            Span::raw("    "),
            Span::styled(" Yes ", yes_style),
        ])
        .centered(),
    );

    text_lines.push(Line::from(""));
    text_lines.push(
        Line::from("← → or h/l to select  |  Enter to confirm  |  Esc to cancel")
            .style(Style::default().fg(Color::DarkGray))
            .centered(),
    );

    let paragraph = Paragraph::new(text_lines).alignment(Alignment::Left);

    frame.render_widget(Clear, area);
    frame.render_widget(border_block, area);
    frame.render_widget(paragraph, inner_area);
}

fn render_resize_dialog(app: &mut App, frame: &mut Frame) {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(30),
            Constraint::Length(14),
            Constraint::Percentage(30),
        ])
        .split(frame.area());

    let area = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(60),
            Constraint::Fill(1),
        ])
        .split(popup_layout[1])[1];

    if let Some(partition) = app.selected_partition() {
        let current_size_str = format_bytes(partition.size);
        let filesystem = partition
            .filesystem
            .clone()
            .unwrap_or_else(|| "none".to_string());

        let border_block = Block::default()
            .title(format!(" Resize {} ", partition.name))
            .title_alignment(Alignment::Center)
            .borders(Borders::ALL)
            .border_type(BorderType::Thick)
            .border_style(Style::default().fg(Color::Green));

        let inner_area = border_block.inner(area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2), // Current size info
                Constraint::Length(1), // Spacing
                Constraint::Length(1), // Label line
                Constraint::Length(3), // Input box with border
                Constraint::Length(1), // Spacing
                Constraint::Length(4), // Info text (4 lines)
                Constraint::Fill(1),   // Remaining space
            ])
            .split(inner_area);

        frame.render_widget(Clear, area);
        frame.render_widget(border_block, area);

        let info_text = Paragraph::new(format!(
            "Current Size: {}\nFilesystem: {}",
            current_size_str, filesystem
        ))
        .style(Style::default().fg(Color::White));

        let size_label = Paragraph::new("New Size:");

        let size_input = Paragraph::new(app.resize_dialog.size_input.value())
            .block(Block::default().borders(Borders::ALL));

        let help_text = Paragraph::new(
            "Examples: 100M, 2.5G, 1T\n\
             Supports both growing and shrinking.\n\
             \n\
             Enter: Confirm | Esc: Cancel",
        )
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::Yellow));

        frame.render_widget(info_text, chunks[0]);
        frame.render_widget(size_label, chunks[2]);
        frame.render_widget(size_input, chunks[3]);
        frame.render_widget(help_text, chunks[5]);
    }
}

fn render_passphrase_dialog(app: &App, frame: &mut Frame) {
    use crate::app::PassphraseOperation;

    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(30),
            Constraint::Length(14),
            Constraint::Percentage(30),
        ])
        .split(frame.area());

    let area = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(60),
            Constraint::Fill(1),
        ])
        .split(popup_layout[1])[1];

    let (title, warning_text, help_text) = match app.passphrase_dialog.operation {
        PassphraseOperation::Unlock => (
            format!(" Unlock {} ", app.passphrase_dialog.target_device),
            "Enter passphrase to unlock encrypted device",
            "Enter: Unlock | Esc: Cancel",
        ),
        PassphraseOperation::Encrypt => (
            format!(" Encrypt {} ", app.passphrase_dialog.target_device),
            "⚠ WARNING: All data will be lost! ⚠\nEnter passphrase for encryption",
            "Enter: Next | Esc: Cancel",
        ),
        PassphraseOperation::EncryptConfirm => (
            format!(" Encrypt {} ", app.passphrase_dialog.target_device),
            "Confirm passphrase",
            "Enter: Encrypt | Esc: Cancel",
        ),
    };

    let border_block = Block::default()
        .title(title)
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(Style::default().fg(Color::Cyan));

    let inner_area = border_block.inner(area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // Warning text
            Constraint::Length(1), // Spacing
            Constraint::Length(1), // Label
            Constraint::Length(3), // Input box
            Constraint::Length(1), // Spacing
            Constraint::Length(2), // Help text
            Constraint::Fill(1),   // Remaining
        ])
        .split(inner_area);

    frame.render_widget(Clear, area);
    frame.render_widget(border_block, area);

    let warning_color = match app.passphrase_dialog.operation {
        PassphraseOperation::Encrypt => Color::Red,
        _ => Color::Yellow,
    };

    let warning = Paragraph::new(warning_text)
        .alignment(Alignment::Center)
        .style(
            Style::default()
                .fg(warning_color)
                .add_modifier(Modifier::BOLD),
        );

    let label = Paragraph::new("Passphrase:");

    let masked_value: String = "*".repeat(app.passphrase_dialog.input.value().len());
    let passphrase_input = Paragraph::new(masked_value)
        .block(Block::default().borders(Borders::ALL))
        .style(Style::default().fg(Color::White));

    let help = Paragraph::new(help_text)
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::DarkGray));

    frame.render_widget(warning, chunks[0]);
    frame.render_widget(label, chunks[2]);
    frame.render_widget(passphrase_input, chunks[3]);
    frame.render_widget(help, chunks[5]);
}
