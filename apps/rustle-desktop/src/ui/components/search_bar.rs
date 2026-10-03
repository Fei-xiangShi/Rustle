//! Search bar component
//! Rounded search input with icon and placeholder text

use iced::widget::{Space, container, row, svg, text_input};
use iced::{Alignment, Element, Fill, Length, Padding};

use crate::app::Message;
use crate::i18n::{Key, Locale};
use crate::ui::responsive::{IconRole, ResponsiveContext, TextRole};
use crate::ui::theme;

pub const TOP_BAR_SEARCH_INPUT_ID: &str = "top_bar_search_input";

#[derive(Debug, Clone, Copy)]
pub struct SearchBarStyle {
    pub width: f32,
    pub height: f32,
    pub icon_size: f32,
    pub horizontal_padding: f32,
    pub icon_spacing: f32,
    pub input_padding: f32,
    pub text_size: f32,
    pub radius: f32,
    /// Maximum visual width for fluid top-bar usage. The outer lane remains
    /// flexible so the field can shrink instead of compressing the chrome.
    pub max_width: f32,
}

impl SearchBarStyle {
    /// Build a top-bar style from the shared responsive-rem tokens.
    pub fn top_bar(context: &ResponsiveContext, width: f32) -> Self {
        let tokens = &context.tokens;
        Self {
            width,
            height: tokens.size(42.0),
            icon_size: tokens.icon(IconRole::TopBarSearch),
            horizontal_padding: tokens.space(14.0),
            icon_spacing: tokens.space(10.0),
            input_padding: tokens.space(10.0),
            text_size: tokens.text(TextRole::BodyLarge),
            radius: tokens.size(21.0),
            max_width: tokens.size(360.0),
        }
    }
}

/// Build the search bar component with a custom style
fn field(search_query: &str, locale: Locale, style: SearchBarStyle) -> Element<'_, Message> {
    let search_icon = svg(svg::Handle::from_memory(
        crate::ui::icons::SEARCH.as_bytes(),
    ))
    .width(style.icon_size)
    .height(style.icon_size)
    .style(|_theme, _status| svg::Style {
        color: Some(theme::text_muted(_theme)),
    });

    let input = text_input(locale.get(Key::SearchPlaceholder), search_query)
        .id(iced::widget::Id::new(TOP_BAR_SEARCH_INPUT_ID))
        .on_input(Message::SearchChanged)
        .on_submit(Message::SearchSubmit)
        .padding(Padding::new(style.input_padding).left(0.0))
        .size(style.text_size)
        .style(|theme, _status| iced::widget::text_input::Style {
            background: iced::Background::Color(iced::Color::TRANSPARENT),
            border: iced::Border::default(),
            placeholder: theme::text_muted(theme),
            value: theme::text_primary(theme),
            selection: theme::accent(theme),
        });

    let content = row![
        Space::new().width(style.horizontal_padding),
        search_icon,
        Space::new().width(style.icon_spacing),
        input,
        Space::new().width(style.horizontal_padding),
    ]
    .align_y(Alignment::Center);

    container(content)
        .width(if style.width > 0.0 {
            Length::Fixed(style.width)
        } else {
            Length::Fill.max(style.max_width)
        })
        .height(style.height)
        .align_y(Alignment::Center)
        .style(move |theme| container::Style {
            background: Some(iced::Background::Color(theme::hover_bg_alpha(theme, 0.08))),
            border: iced::Border {
                radius: style.radius.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// The popup is a true overlay: it never pushes the page or top bar down.
pub fn view<'a>(
    search_query: &'a str,
    locale: Locale,
    style: SearchBarStyle,
    suggestions: &'a crate::app::SearchSuggestionsState,
    context: ResponsiveContext,
) -> Element<'a, Message> {
    use iced::widget::text::{Ellipsis, Wrapping};
    use iced::widget::{button, column, scrollable, text};
    let tokens = context.tokens;
    let mut content = column![
        text("搜索建议")
            .size(tokens.text(TextRole::Label))
            .style(|theme| text::Style {
                color: Some(theme::text_muted(theme))
            })
    ]
    .spacing(tokens.space(6.0));
    let mut previous_kind = None;
    for (index, item) in suggestions.items.iter().enumerate() {
        if previous_kind != Some(item.kind) {
            let label = match item.kind {
                crate::api::SearchType::Songs => Key::SearchTabSongs,
                crate::api::SearchType::Artists => Key::SearchTabArtists,
                crate::api::SearchType::Albums => Key::SearchTabAlbums,
                _ => Key::SearchTabPlaylists,
            };
            content = content.push(
                container(text(locale.get(label)).size(tokens.text(TextRole::Caption)))
                    .style(|theme| container::Style {
                        text_color: Some(theme::text_muted(theme)),
                        ..Default::default()
                    })
                    .padding(Padding::new(tokens.space(4.0)).top(tokens.space(10.0))),
            );
            previous_kind = Some(item.kind);
        }
        let selected = suggestions.selected == Some(index);
        let mut lines = column![
            text(&item.title)
                .size(tokens.text(TextRole::Body))
                .width(Fill)
                .wrapping(Wrapping::None)
                .ellipsis(Ellipsis::End)
        ];
        if !item.subtitle.is_empty() {
            lines = lines.push(
                text(&item.subtitle)
                    .size(tokens.text(TextRole::Caption))
                    .width(Fill)
                    .wrapping(Wrapping::None)
                    .ellipsis(Ellipsis::End)
                    .style(|theme| text::Style {
                        color: Some(theme::text_muted(theme)),
                    }),
            );
        }
        content = content.push(
            container(
                button(lines)
                    .width(Fill)
                    .padding(tokens.space(10.0))
                    .on_press(Message::SearchSuggestPick(index))
                    .style(move |theme, status| iced::widget::button::Style {
                        background: Some(iced::Background::Color(if selected {
                            theme::accent(theme).scale_alpha(0.16)
                        } else if matches!(status, iced::widget::button::Status::Hovered) {
                            theme::hover_bg_alpha(theme, 0.10)
                        } else {
                            iced::Color::TRANSPARENT
                        })),
                        text_color: theme::text_primary(theme),
                        border: iced::Border {
                            radius: tokens.size(9.0).into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
            )
            .id(iced::widget::Id::from(format!("search_suggestion_{index}"))),
        );
    }
    if suggestions.items.is_empty() {
        content = content.push(
            container(
                text(if suggestions.loading {
                    "正在查找…"
                } else {
                    "暂无建议，按 Enter 搜索全部结果"
                })
                .size(tokens.text(TextRole::Label)),
            )
            .padding(tokens.space(12.0)),
        );
    }
    let popup = container(
        scrollable(content)
            .id(iced::widget::Id::new("search_suggestions_scroll"))
            .direction(crate::ui::widgets::vertical_scrollbar(tokens))
            .height(Length::Shrink.max(tokens.size(420.0))),
    )
    .width(Length::Fixed(tokens.size(400.0)))
    .padding(tokens.space(12.0))
    .style(move |theme| container::Style {
        background: Some(iced::Background::Color(theme::surface(theme))),
        text_color: Some(theme::text_primary(theme)),
        border: iced::Border {
            color: theme::border_color(theme),
            width: tokens.size(1.0),
            radius: tokens.size(16.0).into(),
        },
        shadow: iced::Shadow {
            color: iced::Color::BLACK.scale_alpha(if theme::is_dark_theme(theme) {
                0.32
            } else {
                0.12
            }),
            offset: iced::Vector::new(0.0, tokens.size(8.0)),
            blur_radius: tokens.size(24.0),
        },
        ..Default::default()
    });
    container(crate::ui::widgets::AnchoredPopup::new(
        field(search_query, locale, style),
        popup,
        tokens.space(8.0),
        suggestions.open,
        [
            Message::SearchSuggestDismiss,
            Message::SearchSuggestMove(-1),
            Message::SearchSuggestMove(1),
            Message::SearchSubmit,
        ],
    ))
    .width(Fill)
    .align_x(Alignment::Start)
    .into()
}

/// Reveal keyboard selection using measured row bounds, including group headings.
pub fn reveal_suggestion(index: usize) -> iced::Task<Message> {
    use iced::advanced::widget::operation::{Operation, Outcome, Scrollable};
    use iced::widget::{Id, scrollable::AbsoluteOffset};
    use iced::{Rectangle, Vector};
    struct Reveal {
        target: Id,
        scroll: Option<(f32, f32, f32)>,
        target_bounds: Option<Rectangle>,
    }
    impl Operation<f32> for Reveal {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<f32>)) {
            operate(self);
        }
        fn scrollable(
            &mut self,
            id: Option<&Id>,
            bounds: Rectangle,
            content: Rectangle,
            translation: Vector,
            _state: &mut dyn Scrollable,
        ) {
            if id == Some(&Id::new("search_suggestions_scroll")) {
                self.scroll = Some((content.y, bounds.height, translation.y));
            }
        }
        fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
            if id == Some(&self.target) {
                self.target_bounds = Some(bounds);
            }
        }
        fn finish(&self) -> Outcome<f32> {
            let (Some((origin, height, offset)), Some(bounds)) = (self.scroll, self.target_bounds)
            else {
                return Outcome::None;
            };
            let top = bounds.y - origin;
            let bottom = top + bounds.height;
            if top < offset {
                Outcome::Some(top.max(0.0))
            } else if bottom > offset + height {
                Outcome::Some((bottom - height).max(0.0))
            } else {
                Outcome::None
            }
        }
    }
    iced_runtime::task::widget(Reveal {
        target: Id::from(format!("search_suggestion_{index}")),
        scroll: None,
        target_bounds: None,
    })
    .then(|offset| {
        iced::widget::operation::scroll_to(
            Id::new("search_suggestions_scroll"),
            AbsoluteOffset {
                x: None,
                y: Some(offset),
            },
        )
    })
}
