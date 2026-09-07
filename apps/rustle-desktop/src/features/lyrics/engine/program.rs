//! iced Program trait 实现
//!
//! 实现 iced 的 `Program` trait，用于 shader widget。
//! `draw()` 方法返回 `LyricsEnginePrimitive`，由 iced 框架传递给 Pipeline。

use crate::features::lyrics::engine::pipeline::LyricsEnginePrimitive;
use iced::widget::shader::Program;
use iced::{Rectangle, mouse};

/// 歌词渲染 Program
///
/// 持有预构建的 `LyricsEnginePrimitive`，在 `draw()` 时返回。
/// 这种设计避免了 `Program::draw(&self)` 的不可变借用限制。
pub struct LyricsEngineProgram<Message> {
    /// 预构建的渲染数据
    primitive: LyricsEnginePrimitive,
    _phantom: std::marker::PhantomData<Message>,
}

impl<Message> LyricsEngineProgram<Message> {
    /// 使用预构建的 primitive 创建 Program
    pub fn new(primitive: LyricsEnginePrimitive) -> Self {
        Self {
            primitive,
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<Message> Program<Message> for LyricsEngineProgram<Message>
where
    Message: 'static,
{
    type State = ();
    type Primitive = LyricsEnginePrimitive;

    fn draw(
        &self,
        _state: &Self::State,
        _cursor: mouse::Cursor,
        _bounds: Rectangle,
    ) -> Self::Primitive {
        self.primitive.clone()
    }
}
