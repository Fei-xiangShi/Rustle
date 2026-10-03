//! Settings page component
//!
//! All settings on one page with tab navigation (like markdown TOC)
//! Tab bar: continuous bottom line, active tab highlighted
//! Clicking tab scrolls to corresponding section

use iced::widget::{
    Space, button, column, container, pick_list, row, scrollable, svg, text, text_input, toggler,
    tooltip,
};
use iced::{Alignment, Background, Border, ContentFit, Element, Fill, Length, Padding};

use crate::app::{ImageState, Message, SettingsSection};
use crate::features::{Action, KeyBindings, Settings, ShortcutScope};
use crate::i18n::{Key, Language, Locale};
use crate::image::ImageKind;
use crate::ui::animation::SmoothScrollTarget;
use crate::ui::responsive::{
    RadiusRole, ResponsiveContext, ShortcutTablesLayout, TextRole, shortcut_tables_layout,
    top_bar_height,
};
use crate::ui::{theme, widgets};

/// Settings page view with fixed header and all sections on one scrollable page
pub struct SettingsPageView<'a> {
    pub settings: &'a Settings,
    pub audio_devices: Vec<(String, String)>,
    pub font_families: Vec<String>,
    pub active_section: SettingsSection,
    pub locale: Locale,
    pub editing_keybinding: Option<(Action, ShortcutScope)>,
    pub is_logged_in: bool,
    pub user_info: Option<&'a crate::app::UserInfo>,
    pub image_state: &'a ImageState,
    pub cache_stats: Option<&'a crate::cache::CacheStats>,
    pub context: ResponsiveContext,
}

pub fn view<'a>(view: SettingsPageView<'a>) -> Element<'a, Message> {
    let SettingsPageView {
        settings,
        audio_devices,
        font_families,
        active_section,
        locale,
        editing_keybinding,
        is_logged_in,
        user_info,
        image_state,
        cache_stats,
        context,
    } = view;
    let tokens = context.tokens;
    // Fixed header: title + tabs
    let header = column![
        text(locale.get(Key::SettingsTitle).to_string())
            .size(tokens.text(TextRole::Hero))
            .style(|theme| text::Style {
                color: Some(theme::settings_title(theme))
            }),
        Space::new().height(tokens.space(24.0)),
        tab_bar(active_section, locale, context),
    ]
    .width(Fill);

    let header_container = container(header)
        .width(Fill)
        .padding(
            Padding::new(tokens.space(40.0))
                .top(top_bar_height(&context))
                .right(tokens.space(32.0))
                .bottom(tokens.space(20.0))
                .left(tokens.space(32.0)),
        )
        .style(move |theme| container::Style {
            background: Some(Background::Color(theme::background(theme))),
            ..Default::default()
        });

    // All sections on one page
    let all_sections = all_sections_content(SettingsSections {
        settings,
        audio_devices: &audio_devices,
        font_families: &font_families,
        locale,
        editing_keybinding,
        is_logged_in,
        user_info,
        image_state,
        cache_stats,
        context,
    });

    let scrollable_content = crate::ui::widgets::smooth_scroll(
        scrollable(
            container(all_sections).width(Fill).padding(
                Padding::new(tokens.space(20.0))
                    .right(tokens.space(32.0))
                    .bottom(tokens.space(60.0))
                    .left(tokens.space(32.0)),
            ),
        )
        .width(Fill)
        .height(Fill)
        .direction(crate::ui::widgets::vertical_scrollbar(tokens))
        .id(iced::widget::Id::new("settings_scroll"))
        .on_scroll(|viewport| {
            let offset = viewport.absolute_offset();
            Message::SettingsScrolled {
                offset: offset.y,
                max_offset: (viewport.content_bounds().height - viewport.bounds().height).max(0.0),
            }
        }),
        SmoothScrollTarget::Native("settings_scroll"),
        tokens,
        Message::SmoothScroll,
    );

    // Combine fixed header + scrollable content
    container(
        column![header_container, scrollable_content,]
            .width(Fill)
            .height(Fill),
    )
    .width(Fill)
    .height(Fill)
    .style(theme::main_content)
    .into()
}

/// Tab bar with continuous bottom line - active tab portion highlighted
fn tab_bar(
    active_section: SettingsSection,
    locale: Locale,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    let tabs = [
        (
            SettingsSection::Account,
            locale.get(Key::SettingsTabAccount),
        ),
        (
            SettingsSection::Playback,
            locale.get(Key::SettingsTabPlayback),
        ),
        (
            SettingsSection::Display,
            locale.get(Key::SettingsTabDisplay),
        ),
        (SettingsSection::System, locale.get(Key::SettingsTabSystem)),
        (
            SettingsSection::Network,
            locale.get(Key::SettingsTabNetwork),
        ),
        (
            SettingsSection::Storage,
            locale.get(Key::SettingsTabStorage),
        ),
        (
            SettingsSection::Shortcuts,
            locale.get(Key::SettingsTabShortcuts),
        ),
        (SettingsSection::About, locale.get(Key::SettingsTabAbout)),
    ];

    // Build tab items (button + underline stacked vertically)
    let tab_items: Vec<Element<'static, Message>> = tabs
        .iter()
        .map(|(section, label)| {
            let is_active = *section == active_section;

            let tab_button = button(
                container(
                    text(label.to_string())
                        .size(tokens.text(TextRole::Body))
                        .style(move |theme| text::Style {
                            color: Some(if is_active {
                                theme::accent(theme)
                            } else {
                                theme::settings_inactive_tab(theme)
                            }),
                        }),
                )
                .width(Fill)
                .center_x(Fill),
            )
            .style(move |theme, status| {
                let hover_bg = match status {
                    button::Status::Hovered => {
                        Some(Background::Color(theme::hover_bg_alpha(theme, 0.05)))
                    }
                    _ => None,
                };
                button::Style {
                    background: hover_bg,
                    text_color: theme::text_primary(theme),
                    border: Border::default(),
                    ..Default::default()
                }
            })
            .on_press(Message::ScrollToSection(*section))
            .padding([tokens.space(12.0), 0.0])
            .width(Fill);

            let underline = container(Space::new().height(tokens.size(2.0)))
                .width(Fill)
                .style(move |theme| container::Style {
                    background: Some(Background::Color(if is_active {
                        theme::accent(theme)
                    } else {
                        theme::settings_inactive_underline(theme)
                    })),
                    ..Default::default()
                });

            container(column![tab_button, underline].spacing(0).width(Fill))
                .width(tokens.size(90.0))
                .into()
        })
        .collect();

    // All tabs in a row with horizontal scroll for narrow screens
    crate::ui::widgets::scaled_scroll(
        scrollable(row(tab_items).spacing(0))
            .direction(crate::ui::widgets::hidden_horizontal_scrollbar())
            .id(iced::widget::Id::new("settings_tabs_scroll"))
            .width(Fill),
        tokens,
    )
    .into()
}

/// All settings sections on one page
struct SettingsSections<'a> {
    settings: &'a Settings,
    audio_devices: &'a [(String, String)],
    font_families: &'a [String],
    locale: Locale,
    editing_keybinding: Option<(Action, ShortcutScope)>,
    is_logged_in: bool,
    user_info: Option<&'a crate::app::UserInfo>,
    image_state: &'a ImageState,
    cache_stats: Option<&'a crate::cache::CacheStats>,
    context: ResponsiveContext,
}

fn all_sections_content(view: SettingsSections<'_>) -> Element<'static, Message> {
    let SettingsSections {
        settings,
        audio_devices,
        font_families,
        locale,
        editing_keybinding,
        is_logged_in,
        user_info,
        image_state,
        cache_stats,
        context,
    } = view;
    use crate::app::SettingsSection;
    column![
        // Account section
        container(column![
            section_header(locale.get(Key::SettingsAccountTitle), context),
            Space::new().height(context.tokens.space(16.0)),
            account_section(is_logged_in, user_info, image_state, locale, context),
        ])
        .id(SettingsSection::Account.widget_id()),
        Space::new().height(context.tokens.space(40.0)),
        // Playback section
        container(column![
            section_header(locale.get(Key::SettingsPlaybackTitle), context),
            Space::new().height(context.tokens.space(16.0)),
            playback_section(settings, locale, context),
        ])
        .id(SettingsSection::Playback.widget_id()),
        Space::new().height(context.tokens.space(40.0)),
        // Display section
        container(column![
            section_header(locale.get(Key::SettingsDisplayTitle), context),
            Space::new().height(context.tokens.space(16.0)),
            display_section(settings, font_families, locale, context),
        ])
        .id(SettingsSection::Display.widget_id()),
        Space::new().height(context.tokens.space(40.0)),
        // System section
        container(column![
            section_header(locale.get(Key::SettingsSystemTitle), context),
            Space::new().height(context.tokens.space(16.0)),
            system_section(settings, audio_devices, locale, context),
        ])
        .id(SettingsSection::System.widget_id()),
        Space::new().height(context.tokens.space(40.0)),
        // Network section
        container(column![
            section_header(locale.get(Key::SettingsNetworkTitle), context),
            Space::new().height(context.tokens.space(16.0)),
            network_section(settings, locale, context),
        ])
        .id(SettingsSection::Network.widget_id()),
        Space::new().height(context.tokens.space(40.0)),
        // Storage section
        container(column![
            section_header(locale.get(Key::SettingsStorageTitle), context),
            Space::new().height(context.tokens.space(16.0)),
            storage_section(settings, locale, cache_stats, context),
        ])
        .id(SettingsSection::Storage.widget_id()),
        Space::new().height(context.tokens.space(40.0)),
        // Shortcuts section
        container(column![
            section_header(locale.get(Key::SettingsShortcutsTitle), context),
            Space::new().height(context.tokens.space(16.0)),
            shortcuts_section(&settings.keybindings, locale, editing_keybinding, context),
        ])
        .id(SettingsSection::Shortcuts.widget_id()),
        Space::new().height(context.tokens.space(40.0)),
        // About section
        container(column![
            section_header(locale.get(Key::SettingsAboutTitle), context),
            Space::new().height(context.tokens.space(16.0)),
            about_section(locale, context),
        ])
        .id(SettingsSection::About.widget_id()),
    ]
    .spacing(0)
    .width(Fill)
    .into()
}

fn account_section(
    is_logged_in: bool,
    user_info: Option<&crate::app::UserInfo>,
    image_state: &ImageState,
    locale: Locale,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    // Account section
    if is_logged_in {
        if let Some(info) = user_info {
            let avatar_handle = image_state
                .get(ImageKind::UserAvatar, info.user_id)
                .cloned();
            let avatar = if let Some(handle) = avatar_handle {
                container(
                    widgets::crossfade_image(Some(handle))
                        .width(Fill)
                        .height(Fill)
                        .content_fit(iced::ContentFit::Cover)
                        .border_radius(tokens.radius(RadiusRole::Large)),
                )
                .width(tokens.target(crate::ui::responsive::TargetRole::Icon))
                .height(tokens.target(crate::ui::responsive::TargetRole::Icon))
            } else {
                container(
                    svg(iced::widget::svg::Handle::from_memory(
                        crate::ui::icons::USER.as_bytes(),
                    ))
                    .width(tokens.icon(crate::ui::responsive::IconRole::Medium))
                    .height(tokens.icon(crate::ui::responsive::IconRole::Medium))
                    .style(|_theme, _status| iced::widget::svg::Style {
                        color: Some(theme::text_secondary(_theme)),
                    }),
                )
                .width(tokens.target(crate::ui::responsive::TargetRole::Icon))
                .height(tokens.target(crate::ui::responsive::TargetRole::Icon))
                .center_x(tokens.target(crate::ui::responsive::TargetRole::Icon))
                .center_y(tokens.target(crate::ui::responsive::TargetRole::Icon))
                .style(move |_theme| iced::widget::container::Style {
                    background: Some(iced::Background::Color(theme::border_color(_theme))),
                    border: iced::Border {
                        radius: tokens.radius(RadiusRole::Large).into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
            };

            let vip_text: Element<'static, Message> = info
                .vip
                .badge_url()
                .and_then(|icon_url| {
                    let vip_key =
                        crate::image::vip_badge_key(info.user_id, info.vip.tier(), icon_url);
                    image_state.get(ImageKind::VipBadge, vip_key).cloned()
                })
                .map(|handle| -> Element<'static, Message> {
                    widgets::crossfade_image(Some(handle))
                        // Slightly taller than the account text while staying
                        // subordinate to the 48px avatar. Intrinsic width keeps
                        // Black Vinyl VIP and SVIP at their own proportions.
                        .height(tokens.text(TextRole::BodyLarge) + tokens.space(2.0))
                        .content_fit(ContentFit::Contain)
                        .into()
                })
                .unwrap_or_else(|| Space::new().width(0).into());

            column![
                setting_row(
                    context,
                    locale.get(Key::SettingsAccountLoggedInAs),
                    None,
                    row![
                        avatar,
                        Space::new().width(tokens.space(12.0)),
                        column![
                            text(info.nickname.clone())
                                .size(tokens.text(TextRole::BodyLarge))
                                .style(|theme| text::Style {
                                    color: Some(theme::text_primary(theme))
                                }),
                            vip_text,
                        ]
                    ]
                    .align_y(Alignment::Center)
                    .into()
                ),
                divider(context),
                setting_row(
                    context,
                    locale.get(Key::SettingsAccountLogout),
                    None,
                    button(
                        text(locale.get(Key::SettingsAccountLogout).to_string())
                            .size(tokens.text(TextRole::Body))
                    )
                    .style(move |theme, status| {
                        theme::button_danger(theme, status, tokens.theme_metrics())
                    })
                    .padding([tokens.space(8.0), tokens.space(16.0)])
                    .on_press(Message::Logout)
                    .into()
                ),
            ]
            .spacing(0)
            .into()
        } else {
            // Logged in but no info yet
            column![
                setting_row(
                    context,
                    locale.get(Key::SettingsAccountLoggedInAs),
                    None,
                    text("Loading...")
                        .size(tokens.text(TextRole::Body))
                        .style(|theme| text::Style {
                            color: Some(theme::text_primary(theme))
                        })
                        .into()
                ),
                divider(context),
                setting_row(
                    context,
                    locale.get(Key::SettingsAccountLogout),
                    None,
                    button(
                        text(locale.get(Key::SettingsAccountLogout).to_string())
                            .size(tokens.text(TextRole::Body))
                    )
                    .style(move |theme, status| {
                        theme::button_danger(theme, status, tokens.theme_metrics())
                    })
                    .padding([tokens.space(8.0), tokens.space(16.0)])
                    .on_press(Message::Logout)
                    .into()
                ),
            ]
            .spacing(0)
            .into()
        }
    } else {
        column![setting_row(
            context,
            locale.get(Key::SettingsAccountNotLoggedIn),
            None,
            button(
                text(locale.get(Key::ClickToLogin).to_string()).size(tokens.text(TextRole::Body))
            )
            .style(move |theme, status| {
                theme::primary_button(theme, status, tokens.theme_metrics())
            })
            .padding([tokens.space(8.0), tokens.space(16.0)])
            .on_press(Message::ToggleLoginPopup)
            .into()
        ),]
        .spacing(0)
        .into()
    }
}

fn section_header(title: &str, context: ResponsiveContext) -> Element<'static, Message> {
    text(title.to_string())
        .size(context.tokens.text(TextRole::Subtitle))
        .style(|theme| text::Style {
            color: Some(theme::settings_section_title(theme)),
        })
        .into()
}

/// Setting row with label on left and control on right
fn setting_row<'a>(
    context: ResponsiveContext,
    label: &str,
    description: Option<&str>,
    control: Element<'a, Message>,
) -> Element<'a, Message> {
    let tokens = context.tokens;
    let label_text = label.to_string();
    let desc_text = description.map(|d| d.to_string());

    let label_section: Element<'a, Message> = if let Some(desc) = desc_text {
        column![
            text(label_text)
                .size(tokens.text(TextRole::BodyLarge))
                .style(|theme| text::Style {
                    color: Some(theme::settings_label(theme))
                }),
            text(desc)
                .size(tokens.text(TextRole::Caption))
                .style(|theme| text::Style {
                    color: Some(theme::settings_desc(theme))
                }),
        ]
        .spacing(tokens.space(4.0))
        .width(Fill)
        .into()
    } else {
        column![
            text(label_text)
                .size(tokens.text(TextRole::BodyLarge))
                .style(|theme| text::Style {
                    color: Some(theme::settings_label(theme))
                }),
        ]
        .width(Fill)
        .into()
    };

    // Iced resolves intrinsic/Shrink children before distributing remaining
    // row width to Fill children. Keeping the control slot compressed and the
    // label slot flexible creates actual trailing alignment without a
    // profile-selected second row.
    let control_slot = container(control)
        .width(Length::Shrink)
        .align_x(Alignment::End);
    let content = row![label_section, control_slot]
        .spacing(tokens.space(16.0))
        .align_y(Alignment::Center)
        .width(Fill);

    container(content)
        .padding(
            Padding::new(tokens.space(16.0))
                .top(tokens.space(16.0))
                .bottom(tokens.space(16.0)),
        )
        .width(Fill)
        .into()
}

fn playback_section(
    settings: &Settings,
    locale: Locale,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    use crate::features::MusicQuality;

    let tokens = context.tokens;

    // Build music quality options
    let quality_options: Vec<String> = MusicQuality::all()
        .iter()
        .map(|q| q.display_name().to_string())
        .collect();

    let current_quality = settings.playback.music_quality.display_name().to_string();

    column![
        setting_row(
            context,
            locale.get(Key::SettingsMusicQuality),
            Some(locale.get(Key::SettingsMusicQualityDesc)),
            styled_pick_list(context, quality_options, Some(current_quality), |value| {
                let quality = MusicQuality::from_display_name(&value).unwrap_or(MusicQuality::High);
                Message::UpdateMusicQuality(quality)
            },)
        ),
        divider(context),
        setting_row(
            context,
            locale.get(Key::SettingsFadeInOut),
            Some(locale.get(Key::SettingsFadeInOutDesc)),
            toggler(settings.playback.fade_in_out)
                .on_toggle(Message::UpdateFadeInOut)
                .size(tokens.text(TextRole::Title))
                .into()
        ),
        divider(context),
        setting_row(
            context,
            locale.get(Key::SettingsAutomix),
            Some(locale.get(Key::SettingsAutomixDesc)),
            toggler(settings.playback.automix_enabled)
                .on_toggle(Message::UpdateAutomixEnabled)
                .size(tokens.text(TextRole::Title))
                .into()
        ),
        divider(context),
        setting_row(
            context,
            locale.get(Key::SettingsVolumeNormalization),
            Some(locale.get(Key::SettingsVolumeNormalizationDesc)),
            toggler(settings.playback.volume_normalization)
                .on_toggle(Message::UpdateVolumeNormalization)
                .size(tokens.text(TextRole::Title))
                .into()
        ),
        divider(context),
        // Audio Engine entry - clickable row to navigate to audio engine page
        audio_engine_entry_row(locale, context),
    ]
    .spacing(0)
    .into()
}

/// Audio engine entry row - clickable to navigate to audio engine page
fn audio_engine_entry_row(locale: Locale, context: ResponsiveContext) -> Element<'static, Message> {
    let tokens = context.tokens;
    let content = setting_row(
        context,
        locale.get(Key::AudioEngineTitle),
        None,
        svg(svg::Handle::from_memory(
            crate::ui::icons::CHEVRON_RIGHT.as_bytes(),
        ))
        .width(tokens.icon(crate::ui::responsive::IconRole::Medium))
        .height(tokens.icon(crate::ui::responsive::IconRole::Medium))
        .style(|theme, _status| svg::Style {
            color: Some(theme::settings_desc(theme)),
        })
        .into(),
    );

    button(content)
        .width(Fill)
        .padding(0)
        .style(|theme, status| {
            let bg = match status {
                button::Status::Hovered => Some(Background::Color(theme::hover_bg(theme))),
                button::Status::Pressed => Some(Background::Color(theme::hover_bg(theme))),
                _ => None,
            };
            button::Style {
                background: bg,
                border: Border::default(),
                text_color: theme::text_primary(theme),
                ..Default::default()
            }
        })
        .on_press(Message::OpenAudioEngine)
        .into()
}

fn display_section(
    settings: &Settings,
    font_families: &[String],
    locale: Locale,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    use crate::features::CloseBehavior;

    let tokens = context.tokens;

    let close_behavior_options = vec![
        locale.get(Key::SettingsCloseBehaviorAsk).to_string(),
        locale.get(Key::SettingsCloseBehaviorExit).to_string(),
        locale.get(Key::SettingsCloseBehaviorMinimize).to_string(),
    ];

    let current_close_behavior = match settings.close_behavior {
        CloseBehavior::Ask => locale.get(Key::SettingsCloseBehaviorAsk).to_string(),
        CloseBehavior::Exit => locale.get(Key::SettingsCloseBehaviorExit).to_string(),
        CloseBehavior::MinimizeToTray => locale.get(Key::SettingsCloseBehaviorMinimize).to_string(),
    };

    let ask_label = locale.get(Key::SettingsCloseBehaviorAsk).to_string();
    let exit_label = locale.get(Key::SettingsCloseBehaviorExit).to_string();
    let language_options: Vec<String> = Language::all()
        .iter()
        .map(|language| language.display_name().to_string())
        .collect();
    let current_language = Language::from_code(&settings.display.language).unwrap_or_default();

    column![
        setting_row(
            context,
            locale.get(Key::SettingsDarkMode),
            None,
            toggler(settings.display.dark_mode)
                .on_toggle(Message::UpdateDarkMode)
                .size(tokens.text(TextRole::Title))
                .into()
        ),
        divider(context),
        setting_row(
            context,
            locale.get(Key::SettingsLanguage),
            None,
            styled_pick_list(
                context,
                language_options,
                Some(current_language.display_name().to_string()),
                |value| {
                    let language = Language::all()
                        .iter()
                        .copied()
                        .find(|language| language.display_name() == value)
                        .unwrap_or_default();
                    Message::UpdateAppLanguage(language.code().to_string())
                },
            )
        ),
        divider(context),
        setting_row(
            context,
            locale.get(Key::SettingsPowerSavingMode),
            Some(locale.get(Key::SettingsPowerSavingModeDesc)),
            toggler(settings.display.power_saving_mode)
                .on_toggle(Message::UpdatePowerSavingMode)
                .size(tokens.text(TextRole::Title))
                .into()
        ),
        divider(context),
        setting_row(
            context,
            locale.get(Key::SettingsLyricsFontFamily),
            None,
            styled_pick_list(
                context,
                {
                    let auto_label = locale.get(Key::SettingsLyricsFontFamilyAuto).to_string();
                    let mut opts = vec![auto_label];
                    opts.extend(font_families.iter().cloned());
                    opts
                },
                Some(
                    settings
                        .lyrics
                        .lyrics_font_family
                        .clone()
                        .unwrap_or_else(|| locale
                            .get(Key::SettingsLyricsFontFamilyAuto)
                            .to_string()),
                ),
                {
                    let auto_label = locale.get(Key::SettingsLyricsFontFamilyAuto).to_string();
                    move |value| {
                        if value == auto_label {
                            Message::UpdateLyricsFontFamily(None)
                        } else {
                            Message::UpdateLyricsFontFamily(Some(value))
                        }
                    }
                },
            )
        ),
        divider(context),
        setting_row(
            context,
            locale.get(Key::SettingsCloseBehavior),
            None,
            styled_pick_list(
                context,
                close_behavior_options,
                Some(current_close_behavior),
                move |value| {
                    let behavior = if value == ask_label {
                        CloseBehavior::Ask
                    } else if value == exit_label {
                        CloseBehavior::Exit
                    } else {
                        CloseBehavior::MinimizeToTray
                    };
                    Message::UpdateCloseBehavior(behavior)
                },
            )
        ),
    ]
    .spacing(0)
    .into()
}

fn system_section(
    settings: &Settings,
    audio_devices: &[(String, String)],
    locale: Locale,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let tokens = context.tokens;

    let default_device_label = locale.get(Key::SettingsDefaultDevice).to_string();

    // Build display names list (descriptions) and keep track of internal names
    let mut display_names: Vec<String> = vec![default_device_label.clone()];
    for device in audio_devices {
        display_names.push(device.1.clone());
    }

    // Find current device's display name
    let current_display = if let Some(ref device_name) = settings.system.audio_output_device {
        audio_devices
            .iter()
            .find(|(name, _)| name == device_name)
            .map(|(_, description)| description.clone())
            .unwrap_or_else(|| default_device_label.clone())
    } else {
        default_device_label.clone()
    };

    // Clone for closure
    let devices_for_closure = audio_devices.to_vec();
    let default_label = default_device_label.clone();

    column![
        setting_row(
            context,
            locale.get(Key::SettingsAudioDevice),
            None,
            styled_pick_list(
                context,
                display_names,
                Some(current_display),
                move |display_value| {
                    // Convert display name back to internal name
                    let device = if display_value == default_label {
                        None
                    } else {
                        devices_for_closure
                            .iter()
                            .find(|(_, description)| *description == display_value)
                            .map(|(name, _)| name.clone())
                    };
                    Message::UpdateAudioOutputDevice(device)
                },
            )
        ),
        divider(context),
        setting_row(
            context,
            locale.get(Key::SettingsDiscordRichPresence),
            Some(locale.get(Key::SettingsDiscordRichPresenceDesc)),
            toggler(settings.system.discord_enabled)
                .on_toggle(Message::UpdateDiscordEnabled)
                .size(tokens.text(TextRole::Title))
                .into(),
        ),
    ]
    .spacing(0)
    .into()
}

fn network_section(
    settings: &Settings,
    locale: Locale,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    use crate::features::ProxyType;

    let proxy_types = vec![
        locale.get(Key::SettingsProxyNone).to_string(),
        "HTTP".to_string(),
        "HTTPS".to_string(),
        "SOCKS5".to_string(),
        locale.get(Key::SettingsProxySystem).to_string(),
    ];

    let current_proxy_type = match settings.network.proxy_type {
        ProxyType::None => locale.get(Key::SettingsProxyNone).to_string(),
        ProxyType::Http => "HTTP".to_string(),
        ProxyType::Https => "HTTPS".to_string(),
        ProxyType::Socks5 => "SOCKS5".to_string(),
        ProxyType::System => locale.get(Key::SettingsProxySystem).to_string(),
    };

    let proxy_none_label = locale.get(Key::SettingsProxyNone).to_string();

    let show_proxy_details = !matches!(
        settings.network.proxy_type,
        ProxyType::None | ProxyType::System
    );

    // Clone values for use in UI
    let proxy_host = settings.network.proxy_host.clone();
    let proxy_port = settings.network.proxy_port.to_string();
    let proxy_username = settings.network.proxy_username.clone().unwrap_or_default();
    let proxy_password = settings.network.proxy_password.clone().unwrap_or_default();

    let mut items: Vec<Element<'static, Message>> = vec![
        setting_row(
            context,
            locale.get(Key::SettingsOverseasCompatibility),
            Some(locale.get(Key::SettingsOverseasCompatibilityDesc)),
            toggler(settings.network.overseas_compatibility)
                .on_toggle(Message::UpdateOverseasCompatibility)
                .size(context.tokens.text(TextRole::Title))
                .into(),
        ),
        divider(context),
        setting_row(
            context,
            locale.get(Key::SettingsProxyType),
            None,
            styled_pick_list(
                context,
                proxy_types,
                Some(current_proxy_type),
                move |value| {
                    let proxy_type = if value == proxy_none_label {
                        ProxyType::None
                    } else if value == "HTTP" {
                        ProxyType::Http
                    } else if value == "HTTPS" {
                        ProxyType::Https
                    } else if value == "SOCKS5" {
                        ProxyType::Socks5
                    } else {
                        ProxyType::System
                    };
                    Message::UpdateProxyType(proxy_type)
                },
            ),
        ),
    ];

    if show_proxy_details {
        items.push(divider(context));
        items.push(setting_row_with_input(
            context,
            locale.get(Key::SettingsProxyHost),
            "127.0.0.1",
            &proxy_host,
            Message::UpdateProxyHost,
        ));
        items.push(divider(context));
        items.push(setting_row_with_input(
            context,
            locale.get(Key::SettingsProxyPort),
            "1080",
            &proxy_port,
            Message::UpdateProxyPort,
        ));
        items.push(divider(context));
        items.push(setting_row_with_input(
            context,
            locale.get(Key::SettingsProxyUsername),
            "",
            &proxy_username,
            Message::UpdateProxyUsername,
        ));
        items.push(divider(context));
        items.push(setting_row_with_input(
            context,
            locale.get(Key::SettingsProxyPassword),
            "",
            &proxy_password,
            Message::UpdateProxyPassword,
        ));
    }

    column(items).spacing(0).into()
}

/// Setting row with text input - handles lifetime issues by creating owned strings.
/// The shared setting-row component owns alignment; the input only declares
/// its intrinsic editable width.
fn setting_row_with_input<F>(
    context: ResponsiveContext,
    label: &str,
    placeholder: &str,
    value: &str,
    on_input: F,
) -> Element<'static, Message>
where
    F: Fn(String) -> Message + 'static + Clone,
{
    let tokens = context.tokens;
    let placeholder_text = placeholder.to_string();
    let value_text = value.to_string();

    let input = text_input(placeholder_text, value_text)
        .on_input(on_input)
        .size(tokens.text(TextRole::Body))
        .padding([tokens.space(8.0), tokens.space(12.0)])
        .width(Length::Fixed(tokens.size(200.0)))
        .style(move |theme, status| {
            let border_color = match status {
                text_input::Status::Focused { .. } => theme::accent(theme),
                text_input::Status::Hovered => theme::settings_input_border_hover(theme),
                _ => theme::settings_input_border(theme),
            };
            text_input::Style {
                background: iced::Background::Color(theme::settings_input_bg(theme)),
                border: Border {
                    color: border_color,
                    width: tokens.size(1.0),
                    radius: tokens.radius(RadiusRole::Small).into(),
                },
                placeholder: theme::settings_desc(theme),
                value: theme::settings_label(theme),
                selection: theme::accent(theme),
            }
        });

    setting_row(context, label, None, input.into())
}

fn storage_section(
    settings: &Settings,
    locale: Locale,
    cache_stats: Option<&crate::cache::CacheStats>,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    // Get cache directory path
    let cache_dir = crate::utils::cache_dir();
    let cache_path_str = cache_dir.to_string_lossy().to_string();

    // Use cached stats if available, otherwise calculate on-demand
    let cache_size_str = if let Some(stats) = cache_stats {
        format_size_bytes(stats.total_bytes)
    } else {
        format_size_bytes(crate::cache::calculate_cache_stats().total_bytes)
    };

    column![
        setting_row(
            context,
            locale.get(Key::SettingsCacheLocation),
            None,
            text(cache_path_str)
                .size(tokens.text(TextRole::Body))
                .style(|theme| text::Style {
                    color: Some(theme::settings_value(theme))
                })
                .into()
        ),
        divider(context),
        setting_row(
            context,
            locale.get(Key::SettingsCacheSize),
            None,
            text(cache_size_str)
                .size(tokens.text(TextRole::Body))
                .style(|theme| text::Style {
                    color: Some(theme::settings_value(theme))
                })
                .into()
        ),
        divider(context),
        setting_row(
            context,
            locale.get(Key::SettingsMaxCache),
            None,
            styled_pick_list(
                context,
                vec![
                    "512 MB".to_string(),
                    "1 GB".to_string(),
                    "2 GB".to_string(),
                    "5 GB".to_string()
                ],
                Some(format_cache_size(settings.storage.max_cache_mb)),
                |value| {
                    let size_mb = parse_cache_size(&value);
                    Message::UpdateMaxCacheMb(size_mb)
                },
            )
        ),
        divider(context),
        // Download location
        {
            let dl_dir = settings.storage.effective_download_dir();
            let dl_path = dl_dir.to_string_lossy().to_string();
            setting_row(
                context,
                locale.get(Key::SettingsDownloadLocation),
                Some(locale.get(Key::SettingsDownloadLocationDesc)),
                row![
                    text(dl_path)
                        .size(tokens.text(TextRole::Body))
                        .width(Fill)
                        .wrapping(iced::widget::text::Wrapping::None)
                        .ellipsis(iced::widget::text::Ellipsis::End)
                        .style(|theme| text::Style {
                            color: Some(theme::settings_value(theme))
                        }),
                    button(
                        text(locale.get(Key::SettingsDownloadChange).to_string())
                            .size(tokens.text(TextRole::Body))
                    )
                    .style(move |theme, status| {
                        theme::secondary_button(theme, status, tokens.theme_metrics())
                    })
                    .height(tokens.target(crate::ui::responsive::TargetRole::Control))
                    .padding([tokens.space(8.0), tokens.space(16.0)])
                    .on_press(Message::UpdateDownloadDirDialog),
                ]
                .spacing(tokens.space(8.0))
                .align_y(iced::Alignment::Center)
                .width(Fill)
                .into(),
            )
        },
        divider(context),
        // Download quality
        {
            let current_quality = settings.storage.download_quality.display_name().to_string();
            let qualities: Vec<String> = crate::features::MusicQuality::all()
                .iter()
                .map(|q| q.display_name().to_string())
                .collect();
            setting_row(
                context,
                locale.get(Key::SettingsDownloadQuality),
                None,
                styled_pick_list(context, qualities, Some(current_quality), |value| {
                    let q = crate::features::MusicQuality::from_display_name(&value)
                        .unwrap_or(crate::features::MusicQuality::High);
                    Message::UpdateDownloadQuality(q)
                }),
            )
        },
        divider(context),
        setting_row(
            context,
            locale.get(Key::SettingsClearCache),
            Some(locale.get(Key::SettingsClearCacheDesc)),
            button(
                text(locale.get(Key::SettingsClearButton).to_string())
                    .size(tokens.text(TextRole::Body))
            )
            .style(move |theme, status| {
                theme::button_danger(theme, status, tokens.theme_metrics())
            })
            .padding([tokens.space(8.0), tokens.space(16.0)])
            .on_press(Message::ClearCache)
            .into()
        ),
    ]
    .spacing(0)
    .into()
}

/// Format bytes to human readable string
fn format_size_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn about_section(_locale: Locale, context: ResponsiveContext) -> Element<'static, Message> {
    use std::sync::LazyLock;
    let tokens = context.tokens;

    static ICON_DATA: &[u8] = include_bytes!("../../../../../assets/icons/icon_256.png");
    static ICON_HANDLE: LazyLock<iced::widget::image::Handle> =
        LazyLock::new(|| iced::widget::image::Handle::from_bytes(ICON_DATA));

    let icon = container(
        widgets::crossfade_image(Some(ICON_HANDLE.clone()))
            .width(tokens.size(240.0))
            .height(tokens.size(240.0)),
    )
    .style(move |_theme| container::Style {
        border: Border {
            radius: tokens.radius(RadiusRole::Large).into(),
            ..Default::default()
        },
        ..Default::default()
    })
    .clip(true);

    // App name
    let app_name = text("Rustle")
        .size(tokens.text(TextRole::Title))
        .style(|theme| text::Style {
            color: Some(theme::text_primary(theme)),
        });

    // Version
    let version = text(format!("v{}", env!("CARGO_PKG_VERSION")))
        .size(tokens.text(TextRole::Body))
        .style(|theme| text::Style {
            color: Some(theme::settings_desc(theme)),
        });

    // Description
    let description = text("A modern music player built with Rust & Iced")
        .size(tokens.text(TextRole::Label))
        .style(|theme| text::Style {
            color: Some(theme::settings_desc(theme)),
        });

    // Copyright
    let copyright = text("2025-2026 FXS")
        .size(tokens.text(TextRole::Caption))
        .style(|theme| text::Style {
            color: Some(theme::settings_desc(theme)),
        });

    container(
        column![
            icon,
            Space::new().height(tokens.space(16.0)),
            app_name,
            Space::new().height(tokens.space(4.0)),
            version,
            Space::new().height(tokens.space(12.0)),
            description,
            Space::new().height(tokens.space(8.0)),
            copyright,
        ]
        .align_x(Alignment::Center),
    )
    .width(Fill)
    .center_x(Fill)
    .padding([tokens.space(40.0), 0.0])
    .into()
}

fn shortcuts_section(
    keybindings: &KeyBindings,
    locale: Locale,
    editing_keybinding: Option<(Action, ShortcutScope)>,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let layout = shortcut_tables_layout(context);
    let left_actions = [
        (Action::PlayPause, Key::ActionPlayPause),
        (Action::NextTrack, Key::ActionNextTrack),
        (Action::PrevTrack, Key::ActionPrevTrack),
        (Action::VolumeUp, Key::ActionVolumeUp),
        (Action::VolumeDown, Key::ActionVolumeDown),
        (Action::VolumeMute, Key::ActionVolumeMute),
    ];

    let right_actions = [
        (Action::SeekForward, Key::ActionSeekForward),
        (Action::SeekBackward, Key::ActionSeekBackward),
        (Action::GoHome, Key::ActionGoHome),
        (Action::FocusSearch, Key::ActionGoSearch),
        (Action::ToggleQueue, Key::ActionToggleQueue),
        (Action::ToggleFullscreen, Key::ActionToggleFullscreen),
    ];

    if layout == ShortcutTablesLayout::Stacked {
        column![
            shortcut_table(
                &left_actions,
                keybindings,
                locale,
                editing_keybinding,
                context
            ),
            Space::new().height(context.tokens.space(16.0)),
            shortcut_table(
                &right_actions,
                keybindings,
                locale,
                editing_keybinding,
                context
            ),
        ]
        .width(Fill)
        .into()
    } else {
        row![
            shortcut_table(
                &left_actions,
                keybindings,
                locale,
                editing_keybinding,
                context
            ),
            Space::new().width(context.tokens.space(24.0)),
            shortcut_table(
                &right_actions,
                keybindings,
                locale,
                editing_keybinding,
                context
            ),
        ]
        .width(Fill)
        .into()
    }
}

fn shortcut_table(
    actions: &[(Action, Key)],
    keybindings: &KeyBindings,
    locale: Locale,
    editing_keybinding: Option<(Action, ShortcutScope)>,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let rows: Vec<Element<'static, Message>> = actions
        .iter()
        .map(|(action, key)| {
            shortcut_row(
                *action,
                locale.get(*key),
                &crate::platform::keybindings::display_for_action(keybindings, action),
                &crate::platform::keybindings::display_global_for_action(keybindings, action),
                editing_keybinding,
                locale,
                context,
            )
        })
        .collect();

    let rows = column(rows).spacing(context.tokens.space(4.0)).width(Fill);

    let header = shortcut_columns(
        [
            shortcut_header(locale.get(Key::SettingsShortcutFunction), context),
            shortcut_header(locale.get(Key::SettingsShortcutLocal), context),
            shortcut_header(locale.get(Key::SettingsShortcutGlobal), context),
        ],
        context,
    );

    container(
        column![header, divider(context), rows]
            .spacing(context.tokens.space(8.0))
            .width(Fill),
    )
    .width(Fill)
    .into()
}

fn shortcut_row(
    action: Action,
    action_name: &str,
    local_shortcut: &str,
    global_shortcut: &str,
    editing_keybinding: Option<(Action, ShortcutScope)>,
    locale: Locale,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    let action_label = action_name.to_string();
    let action_cell: Element<'static, Message> = tooltip(
        text(action_label.clone())
            .size(tokens.text(TextRole::Body))
            .wrapping(iced::widget::text::Wrapping::None)
            .ellipsis(iced::widget::text::Ellipsis::End)
            .style(|theme| text::Style {
                color: Some(theme::settings_label(theme)),
            }),
        text(action_label)
            .size(tokens.text(TextRole::Caption))
            .wrapping(iced::widget::text::Wrapping::None),
        tooltip::Position::Top,
    )
    .padding(tokens.space(5.0))
    .into();
    let local_cell = shortcut_cell(
        action,
        ShortcutScope::Local,
        local_shortcut,
        editing_keybinding == Some((action, ShortcutScope::Local)),
        locale,
        context,
    );
    let global_cell = shortcut_cell(
        action,
        ShortcutScope::Global,
        global_shortcut,
        editing_keybinding == Some((action, ShortcutScope::Global)),
        locale,
        context,
    );
    let content = shortcut_columns([action_cell, local_cell, global_cell], context);

    container(content)
        .padding(Padding::new(tokens.space(8.0)))
        .width(Fill)
        .into()
}

fn shortcut_columns<'a>(
    cells: [Element<'a, Message>; 3],
    context: ResponsiveContext,
) -> Element<'a, Message> {
    row(cells.map(|cell| container(cell).width(Length::FillPortion(1)).into()))
        .spacing(context.tokens.space(8.0))
        .align_y(Alignment::Center)
        .width(Fill)
        .into()
}

fn shortcut_header(label: &str, context: ResponsiveContext) -> Element<'static, Message> {
    text(label.to_string())
        .size(context.tokens.text(TextRole::Label))
        .wrapping(iced::widget::text::Wrapping::None)
        .ellipsis(iced::widget::text::Ellipsis::End)
        .style(|theme| text::Style {
            color: Some(theme::settings_desc(theme)),
        })
        .into()
}

fn shortcut_cell(
    action: Action,
    scope: ShortcutScope,
    shortcut: &str,
    is_editing: bool,
    locale: Locale,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    let display_value = if is_editing {
        locale.get(Key::SettingsShortcutRecording).to_string()
    } else {
        shortcut.to_string()
    };
    let shortcut_display: Element<'static, Message> = if is_editing {
        container(
            text(display_value.clone())
                .size(tokens.text(TextRole::Label))
                .wrapping(iced::widget::text::Wrapping::None)
                .ellipsis(iced::widget::text::Ellipsis::End)
                .style(|theme| text::Style {
                    color: Some(theme::accent(theme)),
                }),
        )
        .width(Fill)
        .center_x(Fill)
        .padding([tokens.space(4.0), tokens.space(12.0)])
        .style(move |theme| container::Style {
            background: Some(Background::Color(theme::shortcut_key_bg(theme))),
            border: Border {
                radius: tokens.radius(RadiusRole::Small).into(),
                width: tokens.size(1.0),
                color: theme::accent(theme),
            },
            ..Default::default()
        })
        .into()
    } else {
        container(
            text(display_value.clone())
                .size(tokens.text(TextRole::Label))
                .wrapping(iced::widget::text::Wrapping::None)
                .ellipsis(iced::widget::text::Ellipsis::End)
                .style(|theme| text::Style {
                    color: Some(theme::settings_value(theme)),
                }),
        )
        .width(Fill)
        .center_x(Fill)
        .padding([tokens.space(4.0), tokens.space(12.0)])
        .style(move |theme| container::Style {
            background: Some(Background::Color(theme::shortcut_bg(theme))),
            border: Border {
                radius: tokens.radius(RadiusRole::Small).into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
    };

    let edit_button = button(shortcut_display)
        .width(Fill)
        .padding(0)
        .style(|theme, _| button::Style {
            background: None,
            text_color: theme::text_primary(theme),
            border: Border::default(),
            ..Default::default()
        })
        .on_press(if is_editing {
            Message::CancelEditingKeybinding
        } else {
            Message::StartEditingKeybinding(action, scope)
        });
    let edit_button = tooltip(
        edit_button,
        text(display_value)
            .size(tokens.text(TextRole::Caption))
            .wrapping(iced::widget::text::Wrapping::None),
        tooltip::Position::Top,
    )
    .padding(tokens.space(5.0));

    container(edit_button).width(Fill).into()
}

fn divider(context: ResponsiveContext) -> Element<'static, Message> {
    container(Space::new().width(Fill).height(context.tokens.size(1.0)))
        .style(|theme| container::Style {
            background: Some(Background::Color(theme::shortcut_bg(theme))),
            ..Default::default()
        })
        .width(Fill)
        .into()
}

/// Styled pick list (dropdown) with custom appearance
fn styled_pick_list<'a, T, F>(
    context: ResponsiveContext,
    options: Vec<T>,
    selected: Option<T>,
    on_selected: F,
) -> Element<'a, Message>
where
    T: ToString + PartialEq + Clone + 'a,
    F: Fn(T) -> Message + 'a,
{
    let tokens = context.tokens;
    pick_list(selected, options, |value| value.to_string())
        .on_select(on_selected)
        .text_size(tokens.text(TextRole::Body))
        .style(move |theme, status| {
            theme::settings_pick_list(theme, status, tokens.theme_metrics())
        })
        .menu_style(move |theme| theme::settings_pick_list_menu(theme, tokens.theme_metrics()))
        .padding([tokens.space(8.0), tokens.space(12.0)])
        .into()
}

fn format_cache_size(mb: u64) -> String {
    match mb {
        512 => "512 MB".to_string(),
        1024 => "1 GB".to_string(),
        2048 => "2 GB".to_string(),
        5120 => "5 GB".to_string(),
        10240 => "10 GB".to_string(),
        _ => format!("{} MB", mb),
    }
}

fn parse_cache_size(s: &str) -> u64 {
    match s {
        "512 MB" => 512,
        "1 GB" => 1024,
        "2 GB" => 2048,
        "5 GB" => 5120,
        "10 GB" => 10240,
        _ => 1024,
    }
}
