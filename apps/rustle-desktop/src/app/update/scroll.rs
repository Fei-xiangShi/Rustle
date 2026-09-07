//! Shared smooth-scroll message handling and per-frame application.

use iced::Rectangle;
use iced::Task;
use iced::advanced::widget::operation::{self as widget_op, Operation, Outcome, Scrollable};
use iced::time::Instant;
use iced::widget::Id;
use iced::widget::scrollable::AbsoluteOffset;

use crate::app::message::Message;
use crate::app::state::App;
use crate::ui::animation::{SmoothScrollEvent, SmoothScrollTarget};

fn read_native_scroll_offset(target: &'static str) -> impl Operation<f32> {
    struct ReadScrollOffset {
        target: &'static str,
        offset: Option<f32>,
    }

    impl Operation<f32> for ReadScrollOffset {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<f32>)) {
            operate(self);
        }

        fn scrollable(
            &mut self,
            id: Option<&Id>,
            _bounds: Rectangle,
            _content_bounds: Rectangle,
            translation: iced::Vector,
            _state: &mut dyn Scrollable,
        ) {
            if id == Some(&Id::new(self.target)) {
                self.offset = Some(translation.y);
            }
        }

        fn finish(&self) -> Outcome<f32> {
            self.offset.map_or(Outcome::None, Outcome::Some)
        }
    }

    ReadScrollOffset {
        target,
        offset: None,
    }
}

impl App {
    pub(super) fn handle_scroll(&mut self, message: &Message) -> Option<Task<Message>> {
        let Message::SmoothScroll(event) = message else {
            return None;
        };

        match *event {
            SmoothScrollEvent::Requested { target, delta } => {
                if self.core.settings.display.power_saving_mode {
                    Some(self.apply_smooth_scroll_delta(target, delta))
                } else {
                    self.ui
                        .smooth_scroll
                        .request_wheel(target, delta, Instant::now());
                    Some(Task::none())
                }
            }
            SmoothScrollEvent::Cancelled { target } => {
                self.ui.smooth_scroll.cancel(target);
                Some(Task::none())
            }
        }
    }

    pub(super) fn advance_smooth_scroll(&mut self, now: Instant) -> Task<Message> {
        let Some((target, delta)) = self.ui.smooth_scroll.tick(now) else {
            return Task::none();
        };

        self.apply_smooth_scroll_delta(target, delta)
    }

    pub(super) fn settle_smooth_scroll(&mut self) -> Task<Message> {
        let Some((target, delta)) = self.ui.smooth_scroll.take_remaining() else {
            return Task::none();
        };

        self.apply_smooth_scroll_delta(target, delta)
    }

    pub(super) fn apply_smooth_scroll_delta(
        &mut self,
        target: SmoothScrollTarget,
        delta: f32,
    ) -> Task<Message> {
        match target {
            SmoothScrollTarget::Native(id) => {
                let scroll = iced::widget::operation::scroll_by(
                    iced::widget::Id::new(id),
                    AbsoluteOffset { x: 0.0, y: delta },
                );

                if id == "settings_scroll" {
                    scroll.chain(self.settings_scroll_offset_task())
                } else {
                    scroll
                }
            }
            SmoothScrollTarget::PlaylistSongs => {
                self.ui
                    .playlist_page
                    .scroll_state
                    .borrow_mut()
                    .scroll_by_immediate(delta);
                Task::none()
            }
            SmoothScrollTarget::SearchSongs => {
                self.ui
                    .search
                    .scroll_state
                    .borrow_mut()
                    .scroll_by_immediate(delta);
                Task::none()
            }
        }
    }

    pub(super) fn settings_scroll_offset_task(&self) -> Task<Message> {
        iced_runtime::task::widget(widget_op::map(
            read_native_scroll_offset("settings_scroll"),
            Message::SettingsScrolled,
        ))
    }
}
