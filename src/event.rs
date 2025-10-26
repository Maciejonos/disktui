use std::time::Duration;

use crossterm::event::{Event as CrosstermEvent, KeyEvent};
use futures::{FutureExt, StreamExt};
use tokio::sync::mpsc;

use crate::{app::AppResult, notification::Notification};

#[derive(Clone, Debug)]
pub enum Event {
    Tick,
    Key(KeyEvent),
    Notification(Notification),
    Refresh,
    StartProgress(String),
    EndProgress,
}

#[derive(Debug)]
pub struct EventHandler {
    pub sender: mpsc::UnboundedSender<Event>,
    pub receiver: mpsc::UnboundedReceiver<Event>,
    _handler: tokio::task::JoinHandle<()>,
}

impl EventHandler {
    pub fn new(tick_rate: u64) -> Self {
        let tick_rate = Duration::from_millis(tick_rate);
        let (sender, receiver) = mpsc::unbounded_channel();
        let sender_cloned = sender.clone();
        let handler = tokio::spawn(async move {
            let mut reader = crossterm::event::EventStream::new();
            let mut tick = tokio::time::interval(tick_rate);
            loop {
                let tick_delay = tick.tick();
                let crossterm_event = reader.next().fuse();
                tokio::select! {
                  () = sender_cloned.closed() => {
                    break;
                  }
                  _ = tick_delay => {
                    if sender_cloned.send(Event::Tick).is_err() {
                      break;
                    }
                  }
                  Some(Ok(evt)) = crossterm_event => {
                    if let CrosstermEvent::Key(key) = evt {
                      if key.kind == crossterm::event::KeyEventKind::Press
                        && sender_cloned.send(Event::Key(key)).is_err() {
                        break;
                      }
                    }
                  }
                };
            }
        });
        Self {
            sender,
            receiver,
            _handler: handler,
        }
    }

    pub async fn next(&mut self) -> AppResult<Event> {
        self.receiver
            .recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("Event channel closed"))
    }
}
