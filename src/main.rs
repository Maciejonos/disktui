use std::io;
use std::sync::Arc;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use disktui::app::{App, AppResult};
use disktui::config::Config;
use disktui::event::{Event, EventHandler};
use disktui::handler::handle_key_events;
use disktui::tui::Tui;

#[tokio::main]
async fn main() -> AppResult<()> {
	let config = Arc::new(Config::new());

	let backend = CrosstermBackend::new(io::stdout());
	let terminal = Terminal::new(backend)?;
	let events = EventHandler::new(100);
	let mut tui = Tui::new(terminal, events);
	tui.init()?;

	let mut app = App::new(None).await?;

    while app.running {
        tui.draw(&mut app)?;

        match tui.events.next().await? {
            Event::Tick => {
                app.tick().await?;
            }
            Event::Key(key_event) => {
                handle_key_events(key_event, &mut app, tui.events.sender.clone(), config.clone()).await?;
            }
            Event::Notification(notification) => {
                app.notifications.push(notification);
            }
            Event::Refresh => {
                app.refresh().await?;
            }
            Event::StartProgress(message) => {
                app.progress.show_dialog = true;
                app.progress.message = message;
                app.progress.spinner_index = 0;
            }
            Event::EndProgress => {
                app.progress.show_dialog = false;
                app.progress.message.clear();
                app.progress.disk_name.clear();
                app.progress.disk_model.clear();
                app.operation_in_progress.store(false, std::sync::atomic::Ordering::Release);
            }
        }
    }

    tui.exit()?;
    Ok(())
}
