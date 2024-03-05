// SPDX-License-Identifier: GPL-3.0-only

use std::{
    any::TypeId,
    collections::{HashMap, HashSet},
    future::Future,
    hash::{Hash, Hasher},
    num::{NonZeroU32, NonZeroU64},
    pin::{pin, Pin},
    task::{Context, Poll},
    time::Duration,
};
use tokio::time;

use cosmic::{
    iced_futures::{
        futures::{
            channel::mpsc::{self, channel, TryRecvError},
            future::{select, select_all, Either, SelectAll},
            pin_mut, FutureExt, SinkExt, StreamExt,
        },
        subscription, Subscription,
    },
    widget::segmented_button::Entity,
};

use crate::Message;

pub fn auto_save_subscription() -> Subscription<AutoSaveEvent> {
    struct AutoSave;

    subscription::channel(TypeId::of::<AutoSave>(), 100, |mut output| async move {
        let mut state = State::Init;
        let (sender, mut recv) = channel(100);
        let mut timeouts: HashSet<AutoSaveUpdate> = HashSet::new();

        loop {
            match state {
                State::Init => {
                    state = output
                        .send(AutoSaveEvent::Ready(sender.clone()))
                        .await
                        .inspect_err(|e| {
                            log::error!("Auto saver failed to send message to app on init: {e}")
                        })
                        .map(|_| State::Select)
                        .unwrap_or(State::Exit);
                }
                State::Select => {
                    // select_all panics on empty iterators hence the check
                    if timeouts.is_empty() {
                        state = recv.next().await.map_or(State::Exit, State::UpdateTimeouts);
                    } else {
                        // select_all requires IntoIter, so `timeouts` is drained here then the
                        // HashSet is rebuilt from the remaining timeouts
                        let futures: Vec<_> = timeouts.drain().collect();
                        match select(recv.next(), select_all(futures)).await {
                            Either::Left((message, unfinished)) => {
                                // Add the unfinished futures back into the hash set
                                // The futures may have made progress which is why they are moved
                                // between collections
                                timeouts.extend(unfinished.into_inner());

                                // Update timeouts or exit (None means the channel is closed)
                                state = message.map(State::UpdateTimeouts).unwrap_or(State::Exit);
                            }
                            Either::Right(((entity, _, remaining), _)) => {
                                state = match output.send(AutoSaveEvent::Save(entity)).await {
                                    Ok(_) => {
                                        // `timeouts` was drained earlier and should be empty so
                                        // `entity` doesn't need to be removed
                                        timeouts.extend(remaining);
                                        State::Select
                                    }
                                    Err(e) => {
                                        log::error!(
                                            "Auto saver failed to send save message to app: {e}"
                                        );
                                        State::Exit
                                    }
                                }
                            }
                        }
                    }
                }
                State::UpdateTimeouts(update) => {
                    match update {
                        AutoSaveEvent::Update(timeout) => {
                            timeouts.replace(timeout);
                        }
                        AutoSaveEvent::Cancel(entity) => {
                            // TODO: Borrow
                            timeouts.remove(&AutoSaveUpdate::new(entity, 1.try_into().unwrap()));
                        }
                        _ => unreachable!(),
                    }

                    state = State::Select;
                }
                State::Exit => {
                    // TODO: Is there anything else to do here?
                    std::future::pending().await
                }
            }
        }
    })
}

pub enum AutoSaveEvent {
    // Messages to send to application:
    /// Auto saver is ready to register timeouts.
    Ready(mpsc::Sender<AutoSaveEvent>),
    /// Sent when timeout is reached (file is ready to be saved).
    Save(Entity),

    // Messages from application:
    /// Update or insert a new entity to be saved.
    ///
    /// Tabs that are not registered are added to be saved after the timeout expires.
    /// Updating a tab that's already being tracked refreshes the timeout.
    Update(AutoSaveUpdate),
    /// Cancel an [`Entity`]'s timeout.
    Cancel(Entity),
    // TODO: This can probably handle Session save timeouts too
    // Session(..)
}

pub struct AutoSaveUpdate {
    entity: Entity,
    save_in: Pin<Box<time::Sleep>>,
}

impl AutoSaveUpdate {
    pub fn new(entity: Entity, secs: NonZeroU64) -> Self {
        Self {
            entity,
            // `Sleep` doesn't implement Unpin. Box pinning is the most straightforward
            // way to store Sleep and advance each of the timeouts with SelectAll.
            save_in: Box::pin(time::sleep(Duration::from_secs(secs.get()))),
        }
    }
}

impl Hash for AutoSaveUpdate {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.entity.hash(state)
    }
}

impl Eq for AutoSaveUpdate {}

impl PartialEq for AutoSaveUpdate {
    fn eq(&self, other: &Self) -> bool {
        self.entity == other.entity
    }
}

impl Future for AutoSaveUpdate {
    type Output = Entity;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // let mut save_at = pin!(self.save_at);
        match self.as_mut().save_in.poll_unpin(cx) {
            Poll::Ready(_) => Poll::Ready(self.entity),
            Poll::Pending => Poll::Pending,
        }
    }
}

// State machine for auto saver
enum State {
    Init,
    Select,
    UpdateTimeouts(AutoSaveEvent),
    Exit,
}

impl From<Result<Option<AutoSaveEvent>, TryRecvError>> for State {
    fn from(value: Result<Option<AutoSaveEvent>, TryRecvError>) -> Self {
        match value {
            Ok(Some(event)) => State::UpdateTimeouts(event),
            Ok(None) => State::Exit,
            Err(e) => {
                // TODO: Retry or exit?
                log::error!("Auto saver failed to receive message from app: {e}");
                State::Exit
            }
        }
    }
}
