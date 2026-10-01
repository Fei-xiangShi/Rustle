//! iced Program trait 实现
//!
//! 实现 iced 的 `Program` trait，用于 shader widget。
//! `draw()` 方法返回 `LyricsEnginePrimitive`，由 iced 框架传递给 Pipeline。

use crate::features::lyrics::engine::pipeline::LyricsEnginePrimitive;
use iced::widget::shader::{Action, Program};
use iced::{Event, Rectangle, mouse};

/// 歌词渲染 Program
///
/// 持有预构建的 `LyricsEnginePrimitive`，在 `draw()` 时返回。
/// 这种设计避免了 `Program::draw(&self)` 的不可变借用限制。
pub struct LyricsEngineProgram<Message> {
    /// 预构建的渲染数据
    primitive: LyricsEnginePrimitive,
    on_line_press: Option<fn(u64) -> Message>,
}

impl<Message> LyricsEngineProgram<Message> {
    /// 使用预构建的 primitive 创建 Program
    pub fn new(primitive: LyricsEnginePrimitive) -> Self {
        Self {
            primitive,
            on_line_press: None,
        }
    }

    pub fn on_line_press(mut self, on_line_press: fn(u64) -> Message) -> Self {
        self.on_line_press = Some(on_line_press);
        self
    }
}

impl<Message> Program<Message> for LyricsEngineProgram<Message>
where
    Message: 'static,
{
    type State = ();
    type Primitive = LyricsEnginePrimitive;

    fn update(
        &self,
        _state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<Message>> {
        if matches!(
            event,
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
        ) {
            let on_line_press = self.on_line_press?;
            let index = self.primitive.hit_test(bounds, cursor)?;
            return Some(
                Action::publish(on_line_press(self.primitive.lines[index].start_ms)).and_capture(),
            );
        }

        None
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if self.on_line_press.is_some() && self.primitive.hit_test(bounds, cursor).is_some() {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }

    fn draw(
        &self,
        _state: &Self::State,
        _cursor: mouse::Cursor,
        _bounds: Rectangle,
    ) -> Self::Primitive {
        self.primitive.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::lyrics::engine::{
        CachedShapedLine, LyricLineData, LyricsEngineConfig, text_shaper::ShapedLine,
        types::LyricsLineTraits,
    };
    use iced::{Point, event};
    use std::sync::Arc;

    fn program() -> LyricsEngineProgram<u64> {
        let shape = |height| ShapedLine {
            glyphs: Vec::new(),
            width: 240.0,
            height,
            word_bounds: Vec::new(),
        };
        let lines = vec![
            LyricLineData {
                text: "Wrapped lyric".into(),
                translated: Some("Translation".into()),
                romanized: Some("Romanization".into()),
                start_ms: 1234,
                ..Default::default()
            },
            LyricLineData {
                text: "Duet lyric".into(),
                start_ms: 5678,
                is_duet: true,
                ..Default::default()
            },
        ];
        LyricsEngineProgram::new(LyricsEnginePrimitive {
            line_traits: LyricsLineTraits::from_lines(&lines),
            lines: Arc::new(lines),
            shaped_lines: Arc::new(vec![
                CachedShapedLine {
                    main: shape(80.0),
                    main_font_size: 48.0,
                    translation: Some(shape(40.0)),
                    translation_font_size: 24.0,
                    romanized: Some(shape(40.0)),
                    romanized_font_size: 24.0,
                    total_height: 160.0,
                },
                CachedShapedLine {
                    main: shape(60.0),
                    main_font_size: 48.0,
                    translation: None,
                    translation_font_size: 24.0,
                    romanized: None,
                    romanized_font_size: 24.0,
                    total_height: 60.0,
                },
            ]),
            scroll_position: 0.0,
            is_manually_scrolling: true,
            buffered_lines: Default::default(),
            scroll_to_index: 0,
            current_time_ms: 0.0,
            font_size: 48.0,
            config: LyricsEngineConfig::default(),
            is_playing: true,
            interlude_dots: None,
            cached_line_heights: Arc::new(vec![160.0, 60.0]),
            line_positions: Arc::new(vec![100.0, 280.0]),
            line_scales: Arc::new(vec![0.8, 1.0]),
            line_blur_levels: Arc::new(vec![0.0; 2]),
            line_opacities: Arc::new(vec![1.0; 2]),
            line_bg_slide_y: Arc::new(vec![0.0; 2]),
            line_bright_mask_alpha: Arc::new(vec![0.2; 2]),
            line_dark_mask_alpha: Arc::new(vec![0.2; 2]),
        })
        .on_line_press(|time_ms| time_ms)
    }

    fn bounds() -> Rectangle {
        Rectangle {
            x: 500.0,
            y: 40.0,
            width: 400.0,
            height: 600.0,
        }
    }

    fn cursor(x: f32, y: f32) -> mouse::Cursor {
        mouse::Cursor::Available(Point::new(x, y))
    }

    #[test]
    fn auto_following_lyrics_have_no_pointer_or_click_action() {
        let mut program = program();
        program.primitive.is_manually_scrolling = false;
        let cursor = cursor(700.0, 170.0);
        assert_eq!(program.primitive.hit_test(bounds(), cursor), None);
        assert_eq!(
            program.mouse_interaction(&(), bounds(), cursor),
            mouse::Interaction::default()
        );
        assert!(
            program
                .update(
                    &mut (),
                    &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                    bounds(),
                    cursor,
                )
                .is_none()
        );
    }

    #[test]
    fn clicking_wrapped_translation_and_romanization_seeks_to_line_start() {
        let program = program();
        for y in [170.0, 240.0, 270.0] {
            let cursor = cursor(700.0, y);
            assert_eq!(
                program.mouse_interaction(&(), bounds(), cursor),
                mouse::Interaction::Pointer
            );
            let action = program
                .update(
                    &mut (),
                    &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                    bounds(),
                    cursor,
                )
                .expect("visible lyric should be clickable");
            let (message, _, status) = action.into_inner();
            assert_eq!(message, Some(1234));
            assert_eq!(status, event::Status::Captured);
        }
    }

    #[test]
    fn hit_test_uses_current_animation_and_duet_row_positions() {
        let mut program = program();
        assert_eq!(
            program.primitive.hit_test(bounds(), cursor(850.0, 340.0)),
            Some(1)
        );
        Arc::make_mut(&mut program.primitive.line_positions)[1] = 400.0;
        assert_eq!(
            program.primitive.hit_test(bounds(), cursor(850.0, 340.0)),
            None
        );
        assert_eq!(
            program.primitive.hit_test(bounds(), cursor(850.0, 460.0)),
            Some(1)
        );

        Arc::make_mut(&mut program.primitive.lines)[1].is_bg = true;
        Arc::make_mut(&mut program.primitive.line_bg_slide_y)[1] = -80.0;
        assert_eq!(
            program.primitive.hit_test(bounds(), cursor(850.0, 410.0)),
            Some(1)
        );
        assert_eq!(
            program.primitive.hit_test(bounds(), cursor(850.0, 460.0)),
            None
        );
    }

    #[test]
    fn blank_hidden_unshaped_and_outside_rows_are_not_clickable() {
        let mut program = program();
        for cursor in [
            cursor(700.0, 310.0),       // Gap between rows.
            cursor(700.0, 145.0),       // Outside the scaled first row.
            cursor(505.0, 170.0),       // Renderer padding.
            cursor(950.0, 170.0),       // Outside the renderer.
            mouse::Cursor::Unavailable, // Covered by an overlay.
        ] {
            assert_eq!(program.primitive.hit_test(bounds(), cursor), None);
            assert_eq!(
                program.mouse_interaction(&(), bounds(), cursor),
                mouse::Interaction::default()
            );
        }

        Arc::make_mut(&mut program.primitive.line_opacities)[0] = 0.0001;
        assert_eq!(
            program.primitive.hit_test(bounds(), cursor(700.0, 170.0)),
            None
        );
        Arc::make_mut(&mut program.primitive.lines)[1].text = "  ".into();
        assert_eq!(
            program.primitive.hit_test(bounds(), cursor(700.0, 340.0)),
            None
        );
        Arc::make_mut(&mut program.primitive.line_opacities)[0] = 1.0;
        Arc::make_mut(&mut program.primitive.shaped_lines).clear();
        assert_eq!(
            program.primitive.hit_test(bounds(), cursor(700.0, 170.0)),
            None
        );
    }

    #[test]
    fn wheel_and_secondary_buttons_do_not_seek_or_capture() {
        let program = program();
        for event in [
            mouse::Event::WheelScrolled {
                delta: mouse::ScrollDelta::Lines { x: 0.0, y: 1.0 },
            },
            mouse::Event::ButtonPressed(mouse::Button::Right),
            mouse::Event::ButtonReleased(mouse::Button::Left),
        ] {
            assert!(
                program
                    .update(
                        &mut (),
                        &Event::Mouse(event),
                        bounds(),
                        cursor(700.0, 170.0)
                    )
                    .is_none()
            );
        }
    }
}
