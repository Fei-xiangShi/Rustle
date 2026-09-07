//! Audio Engine page component
//!
//! Rustle Audio Engine page with:
//! - 10-band parametric equalizer with vertical sliders
//! - EQ curve visualization using Canvas
//! - Preset selection
//! - Preamp control
//! - Professional spectrum analyzer (FFT-based)

use iced::widget::canvas::{self, Canvas, Frame, Geometry, Path, Stroke, Text};
use iced::widget::{Space, column, container, pick_list, row, scrollable, slider, text, toggler};
use iced::{Alignment, Background, Color, Element, Fill, Length, Padding, Point, Theme};

use crate::app::Message;
use crate::audio::{
    AudioAnalysisData,
    analyzer::{FFT_SIZE, SPECTRUM_BARS},
};
use crate::features::{EqualizerPreset, Settings};
use crate::i18n::{Key, Locale};
use crate::ui::animation::SmoothScrollTarget;
use crate::ui::responsive::{LayoutProfile, ResponsiveContext, TextRole, UiTokens, top_bar_height};
use crate::ui::theme;
use crate::ui::widgets::vertical_slider;

/// Frequency labels for 10-band EQ
const FREQ_LABELS: [&str; 10] = [
    "32", "64", "125", "250", "500", "1K", "2K", "4K", "8K", "16K",
];

/// Frequency labels for spectrum analyzer (logarithmic scale)
const SPECTRUM_FREQ_LABELS: [(&str, f32); 10] = [
    ("32", 32.0),
    ("64", 64.0),
    ("125", 100.0),
    ("250", 200.0),
    ("500", 500.0),
    ("1k", 1000.0),
    ("2k", 2000.0),
    ("4k", 4000.0),
    ("8k", 8000.0),
    ("16k", 16000.0),
];

/// dB labels for spectrum analyzer
const SPECTRUM_DB_LABELS: [(i32, &str); 5] = [
    (12, "+12 dB"),
    (0, "0 dB"),
    (-12, "-12 dB"),
    (-24, "-24 dB"),
    (-48, "-48 dB"),
];

/// Audio Engine page view
pub fn view<'a>(
    settings: &'a Settings,
    locale: Locale,
    analysis_data: Option<AudioAnalysisData>,
    context: ResponsiveContext,
) -> Element<'a, Message> {
    let tokens = context.tokens;
    // Header with just title
    let header = text(locale.get(Key::AudioEngineTitle).to_string())
        .size(tokens.text(TextRole::TitleLarge))
        .style(|theme| text::Style {
            color: Some(theme::settings_title(theme)),
        });

    let header_container = container(header)
        .width(Fill)
        .padding(
            Padding::new(tokens.space(40.0))
                .top(top_bar_height(&context))
                .right(tokens.space(32.0))
                .bottom(tokens.space(20.0))
                .left(tokens.space(32.0)),
        )
        .style(|theme| container::Style {
            background: Some(Background::Color(theme::background(theme))),
            ..Default::default()
        });

    // Get real-time audio levels and spectrum
    let (left_level, right_level, spectrum_db, sample_rate) =
        if let Some(data) = analysis_data.as_ref() {
            (
                data.left_rms(),
                data.right_rms(),
                data.spectrum_db(),
                data.sample_rate(),
            )
        } else {
            (0.0, 0.0, [-60.0; SPECTRUM_BARS], 48000)
        };

    let decay = settings.playback.spectrum_decay;
    let bars_mode = settings.playback.spectrum_bars_mode;

    // Main content with equalizer and audio visualization
    let content = column![
        // Equalizer section
        equalizer_section(settings, locale, context),
        Space::new().height(tokens.space(40.0)),
        // Audio visualization section
        audio_visualization_section(AudioVisualization {
            channel_levels: [left_level, right_level],
            spectrum_db,
            sample_rate,
            decay,
            bars_mode,
            locale,
            context,
        }),
    ]
    .spacing(0)
    .width(Fill);

    let scrollable_content = crate::ui::widgets::smooth_scroll(
        scrollable(
            container(content).width(Fill).padding(
                Padding::new(tokens.space(20.0))
                    .right(tokens.space(32.0))
                    .bottom(tokens.space(60.0))
                    .left(tokens.space(32.0)),
            ),
        )
        .width(Fill)
        .height(Fill)
        .direction(crate::ui::widgets::vertical_scrollbar(tokens))
        .id(iced::widget::Id::new("audio_engine_scroll")),
        SmoothScrollTarget::Native("audio_engine_scroll"),
        tokens,
        Message::SmoothScroll,
    );

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

/// Equalizer section with title
fn equalizer_section(
    settings: &Settings,
    locale: Locale,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    let eq_enabled = settings.playback.equalizer_enabled;
    let eq_preset = settings.playback.equalizer_preset;
    let eq_values = settings.playback.equalizer_values;
    let preamp = settings.playback.equalizer_preamp;

    // Section title row with toggle and preset
    let title_row = row![
        text(locale.get(Key::AudioEngineEqualizer).to_string())
            .size(tokens.text(TextRole::Subtitle))
            .style(|theme| text::Style {
                color: Some(theme::settings_title(theme))
            }),
        Space::new().width(tokens.space(24.0)),
        toggler(eq_enabled)
            .on_toggle(Message::UpdateEqualizerEnabled)
            .size(tokens.text(TextRole::Subtitle)),
        Space::new().width(tokens.space(8.0)),
        text(if eq_enabled { "ON" } else { "OFF" })
            .size(tokens.text(TextRole::Label))
            .style(|theme| text::Style {
                color: Some(theme::settings_desc(theme))
            }),
        Space::new().width(Fill),
        preset_picker(eq_preset, context),
    ]
    .align_y(Alignment::Center)
    .width(Fill);

    // Equalizer content
    let eq_content: Element<'static, Message> = if eq_enabled {
        column![
            Space::new().height(tokens.space(24.0)),
            // EQ Curve visualization
            eq_curve_canvas(eq_values, context),
            Space::new().height(tokens.space(24.0)),
            // Sliders row with preamp
            sliders_with_preamp(eq_values, preamp, context),
        ]
        .spacing(0)
        .width(Fill)
        .into()
    } else {
        container(
            text(locale.get(Key::AudioEngineEqualizerDisabled).to_string())
                .size(tokens.text(TextRole::Body))
                .style(|theme| text::Style {
                    color: Some(theme::settings_desc(theme)),
                }),
        )
        .width(Fill)
        .height(tokens.size(100.0))
        .center_x(Fill)
        .center_y(tokens.size(100.0))
        .into()
    };

    column![title_row, eq_content,]
        .spacing(0)
        .width(Fill)
        .into()
}

/// Audio visualization section with spectrum analyzer
struct AudioVisualization {
    channel_levels: [f32; 2],
    spectrum_db: [f32; SPECTRUM_BARS],
    sample_rate: u32,
    decay: f32,
    bars_mode: bool,
    locale: Locale,
    context: ResponsiveContext,
}

fn audio_visualization_section(view: AudioVisualization) -> Element<'static, Message> {
    let AudioVisualization {
        channel_levels: [left_level, right_level],
        spectrum_db,
        sample_rate,
        decay,
        bars_mode,
        locale,
        context,
    } = view;
    let tokens = context.tokens;

    // Separator line
    let separator = container(Space::new().width(Fill).height(tokens.size(1.0))).style(|theme| {
        container::Style {
            background: Some(Background::Color(theme::divider(theme))),
            ..Default::default()
        }
    });

    // Section title
    let title = text(locale.get(Key::AudioEngineSpectrum).to_string())
        .size(tokens.text(TextRole::Subtitle))
        .style(|theme| text::Style {
            color: Some(theme::settings_title(theme)),
        });

    // Mode dropdown (bars/line) - like equalizer preset picker
    let mode_picker = spectrum_mode_picker(bars_mode, context);

    // Spectrum analyzer canvas (main visualization)
    let spectrum_height = tokens.size(280.0);
    let spectrum: Element<'static, Message> = Canvas::new(SpectrumAnalyzer {
        spectrum_db,
        sample_rate,
        bars_mode,
        tokens,
    })
    .width(Fill)
    .height(Length::Fixed(spectrum_height))
    .into();

    // Volume meters on the left (same height as spectrum graph area)
    let meter_height = (spectrum_height - tokens.space(50.0)).max(tokens.size(120.0));
    let meters: Element<'static, Message> = column![
        Space::new().height(tokens.space(10.0)), // Align with graph top margin
        row![
            volume_meter_view("L", left_level, meter_height, context),
            Space::new().width(tokens.space(8.0)),
            volume_meter_view("R", right_level, meter_height, context),
        ]
        .align_y(Alignment::Start),
    ]
    .align_x(Alignment::Center)
    .into();

    // Decay slider
    let decay_label = text("Decay")
        .size(tokens.text(TextRole::Caption))
        .style(|theme| text::Style {
            color: Some(theme::settings_desc(theme)),
        });
    let decay_slider = slider(0.0..=0.95, decay, Message::UpdateSpectrumDecay)
        .step(0.01_f32)
        .width(Length::Fixed(tokens.size(120.0)))
        .style(move |theme, status| rem_slider_style(theme, status, tokens));

    column![
        separator,
        Space::new().height(tokens.space(24.0)),
        row![title, Space::new().width(Fill), mode_picker,]
            .align_y(Alignment::Center)
            .width(Fill),
        Space::new().height(tokens.space(16.0)),
        visualization_lanes(meters, spectrum, context),
        Space::new().height(tokens.space(12.0)),
        row![
            decay_label,
            Space::new().width(tokens.space(8.0)),
            decay_slider,
        ]
        .align_y(Alignment::Center)
        .width(Fill),
    ]
    .spacing(0)
    .width(Fill)
    .into()
}

fn visualization_lanes(
    meters: Element<'static, Message>,
    spectrum: Element<'static, Message>,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    match context.profile {
        LayoutProfile::Expanded | LayoutProfile::Standard => {
            row![meters, Space::new().width(tokens.space(16.0)), spectrum,]
                .align_y(Alignment::Start)
                .width(Fill)
                .into()
        }
        LayoutProfile::Compact | LayoutProfile::Tablet | LayoutProfile::Narrow => {
            column![meters, Space::new().height(tokens.space(12.0)), spectrum,]
                .spacing(0)
                .width(Fill)
                .into()
        }
    }
}

/// Volume meter view for a single channel
fn volume_meter_view(
    label: &'static str,
    level: f32,
    height: f32,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    column![
        text(label)
            .size(tokens.text(TextRole::Caption))
            .style(|theme| text::Style {
                color: Some(theme::settings_desc(theme))
            }),
        Space::new().height(tokens.space(8.0)),
        Canvas::new(VolumeMeter { level, tokens })
            .width(Length::Fixed(tokens.size(24.0)))
            .height(Length::Fixed(height)),
    ]
    .spacing(0)
    .align_x(Alignment::Center)
    .into()
}

/// Preset picker dropdown
fn preset_picker(
    current: EqualizerPreset,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    let presets: Vec<EqualizerPreset> = EqualizerPreset::all().to_vec();

    pick_list(Some(current), presets, |preset| preset.to_string())
        .on_select(Message::UpdateEqualizerPreset)
        .text_size(tokens.text(TextRole::Body))
        .padding([tokens.space(8.0), tokens.space(16.0)])
        .style(move |theme, status| {
            theme::settings_pick_list(theme, status, tokens.theme_metrics())
        })
        .menu_style(move |theme| theme::settings_pick_list_menu(theme, tokens.theme_metrics()))
        .into()
}

/// Spectrum display mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpectrumMode {
    Bars,
    Line,
}

impl std::fmt::Display for SpectrumMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpectrumMode::Bars => write!(f, "柱状"),
            SpectrumMode::Line => write!(f, "曲线"),
        }
    }
}

/// Spectrum mode picker dropdown
fn spectrum_mode_picker(bars_mode: bool, context: ResponsiveContext) -> Element<'static, Message> {
    let tokens = context.tokens;
    let modes = vec![SpectrumMode::Bars, SpectrumMode::Line];
    let current = if bars_mode {
        SpectrumMode::Bars
    } else {
        SpectrumMode::Line
    };

    pick_list(Some(current), modes, |mode| mode.to_string())
        .on_select(|mode| Message::UpdateSpectrumBarsMode(mode == SpectrumMode::Bars))
        .text_size(tokens.text(TextRole::Body))
        .padding([tokens.space(8.0), tokens.space(16.0)])
        .style(move |theme, status| {
            theme::settings_pick_list(theme, status, tokens.theme_metrics())
        })
        .menu_style(move |theme| theme::settings_pick_list_menu(theme, tokens.theme_metrics()))
        .into()
}

/// EQ curve visualization using Canvas
fn eq_curve_canvas(eq_values: [f32; 10], context: ResponsiveContext) -> Element<'static, Message> {
    Canvas::new(EqCurve {
        values: eq_values,
        tokens: context.tokens,
    })
    .width(Fill)
    .height(Length::Fixed(context.tokens.size(120.0)))
    .into()
}

// ============================================================================
// Canvas Programs
// ============================================================================

/// Canvas program for drawing EQ curve
struct EqCurve {
    values: [f32; 10],
    tokens: UiTokens,
}

impl canvas::Program<Message> for EqCurve {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let width = bounds.width;
        let height = bounds.height;

        // Draw horizontal grid lines (subtle)
        let grid_color = Color::from_rgba(1.0, 1.0, 1.0, 0.08);
        for i in 0..=4 {
            let y = height * (i as f32 / 4.0);
            let line = Path::line(Point::new(0.0, y), Point::new(width, y));
            frame.stroke(
                &line,
                Stroke::default()
                    .with_color(grid_color)
                    .with_width(self.tokens.size(1.0)),
            );
        }

        // Draw center line (0 dB) slightly brighter
        let center_y = height / 2.0;
        let center_line = Path::line(Point::new(0.0, center_y), Point::new(width, center_y));
        frame.stroke(
            &center_line,
            Stroke::default()
                .with_color(Color::from_rgba(1.0, 1.0, 1.0, 0.15))
                .with_width(self.tokens.size(1.0)),
        );

        // Calculate curve points with smooth interpolation
        let points = self.calculate_curve_points(width, height);

        // Draw gradient fill under the curve
        self.draw_gradient_fill(&mut frame, &points, width, height);

        // Draw the main curve line
        if points.len() >= 2 {
            let curve = Path::new(|builder| {
                builder.move_to(points[0]);
                for point in points.iter().skip(1) {
                    builder.line_to(*point);
                }
            });

            frame.stroke(
                &curve,
                Stroke::default()
                    .with_color(theme::ACCENT_PINK)
                    .with_width(self.tokens.size(2.0)),
            );
        }

        vec![frame.into_geometry()]
    }
}

impl EqCurve {
    /// Calculate smooth curve points using catmull-rom spline interpolation
    fn calculate_curve_points(&self, width: f32, height: f32) -> Vec<Point> {
        let mut points = Vec::new();
        let num_samples = 200;
        let center_y = height / 2.0;
        let max_db = 12.0;

        // Preamp offset at the start (left side)
        let preamp_width = self.tokens.size(60.0);
        let eq_start_x = preamp_width + self.tokens.size(20.0);
        let eq_width = width - eq_start_x;
        let vertical_inset = self.tokens.size(5.0);

        for i in 0..=num_samples {
            let t = i as f32 / num_samples as f32;
            let x = eq_start_x + t * eq_width;

            let eq_pos = t * 9.0;
            let band_index = (eq_pos.floor() as usize).min(8);
            let band_frac = eq_pos - band_index as f32;

            let v0 = if band_index > 0 {
                self.values[band_index - 1]
            } else {
                self.values[0]
            };
            let v1 = self.values[band_index];
            let v2 = self.values[(band_index + 1).min(9)];
            let v3 = self.values[(band_index + 2).min(9)];

            let value = catmull_rom(v0, v1, v2, v3, band_frac);
            let normalized = value / max_db;
            let y = center_y - normalized * (height / 2.0 - self.tokens.size(10.0));

            points.push(Point::new(
                x,
                y.clamp(vertical_inset, height - vertical_inset),
            ));
        }

        points
    }

    /// Draw gradient fill from curve to bottom with fade effect
    fn draw_gradient_fill(&self, frame: &mut Frame, points: &[Point], _width: f32, height: f32) {
        if points.len() < 2 {
            return;
        }

        let num_strips = 12;
        for strip in 0..num_strips {
            let strip_ratio = strip as f32 / num_strips as f32;
            let next_ratio = (strip + 1) as f32 / num_strips as f32;
            let alpha = 0.20 * (1.0 - strip_ratio).powf(1.5);

            if alpha < 0.005 {
                continue;
            }

            let fill_path = Path::new(|builder| {
                let mut started = false;
                for point in points.iter() {
                    let y_at_curve = point.y;
                    let y_top = y_at_curve + (height - y_at_curve) * strip_ratio;
                    if !started {
                        builder.move_to(Point::new(point.x, y_top));
                        started = true;
                    } else {
                        builder.line_to(Point::new(point.x, y_top));
                    }
                }
                for point in points.iter().rev() {
                    let y_at_curve = point.y;
                    let y_bottom = y_at_curve + (height - y_at_curve) * next_ratio;
                    builder.line_to(Point::new(point.x, y_bottom));
                }
                builder.close();
            });

            let fill_color = Color::from_rgba(0.95, 0.3, 0.5, alpha);
            frame.fill(&fill_path, fill_color);
        }
    }
}

/// Catmull-Rom spline interpolation
fn catmull_rom(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * ((2.0 * p1)
        + (-p0 + p2) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
        + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
}

/// Volume meter canvas program
struct VolumeMeter {
    level: f32, // 0.0 to 1.0
    tokens: UiTokens,
}

impl canvas::Program<Message> for VolumeMeter {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let width = bounds.width;
        let height = bounds.height;

        // Background track
        let bg_rect = Path::new(|builder| {
            builder.move_to(Point::new(0.0, 0.0));
            builder.line_to(Point::new(width, 0.0));
            builder.line_to(Point::new(width, height));
            builder.line_to(Point::new(0.0, height));
            builder.close();
        });
        frame.fill(&bg_rect, Color::from_rgba(1.0, 1.0, 1.0, 0.1));

        // Level indicator (from bottom up)
        // Draw level with gradient colors (green -> yellow -> red)
        let num_segments = 20;
        let segment_height = height / num_segments as f32;
        let horizontal_inset = self.tokens.size(2.0);
        let vertical_inset = self.tokens.size(1.0);

        for i in 0..num_segments {
            let seg_bottom = height - (i as f32 * segment_height);
            let seg_top = seg_bottom - segment_height;
            let seg_level = i as f32 / num_segments as f32;

            // Only draw if this segment is within the level
            if seg_level > self.level {
                continue;
            }

            // Color based on level: green (low) -> yellow (mid) -> red (high)
            let color = if seg_level < 0.6 {
                // Green zone
                theme::spectrum_green()
            } else if seg_level < 0.85 {
                // Yellow zone
                theme::spectrum_yellow()
            } else {
                // Red zone
                theme::spectrum_red()
            };

            let seg_rect = Path::new(|builder| {
                builder.move_to(Point::new(horizontal_inset, seg_top + vertical_inset));
                builder.line_to(Point::new(
                    width - horizontal_inset,
                    seg_top + vertical_inset,
                ));
                builder.line_to(Point::new(
                    width - horizontal_inset,
                    seg_bottom - vertical_inset,
                ));
                builder.line_to(Point::new(horizontal_inset, seg_bottom - vertical_inset));
                builder.close();
            });
            frame.fill(&seg_rect, color);
        }

        vec![frame.into_geometry()]
    }
}

/// Professional spectrum analyzer canvas (FFT-based)
struct SpectrumAnalyzer {
    spectrum_db: [f32; SPECTRUM_BARS],
    sample_rate: u32,
    bars_mode: bool,
    tokens: UiTokens,
}

impl canvas::Program<Message> for SpectrumAnalyzer {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let width = bounds.width;
        let height = bounds.height;

        // Layout constants - dB labels on left side
        let left_margin = self.tokens.size(50.0); // Space for dB labels on left
        let right_margin = self.tokens.size(10.0);
        let top_margin = self.tokens.size(10.0);
        let bottom_margin = self.tokens.size(40.0); // Space for frequency labels and info
        let graph_width = width - left_margin - right_margin;
        let graph_height = height - top_margin - bottom_margin;

        // dB range: +12 to -60
        let db_max = 12.0_f32;
        let db_min = -60.0_f32;
        let db_range = db_max - db_min;

        // Background
        let bg_rect = Path::new(|builder| {
            builder.move_to(Point::new(left_margin, top_margin));
            builder.line_to(Point::new(left_margin + graph_width, top_margin));
            builder.line_to(Point::new(
                left_margin + graph_width,
                top_margin + graph_height,
            ));
            builder.line_to(Point::new(left_margin, top_margin + graph_height));
            builder.close();
        });
        frame.fill(&bg_rect, Color::from_rgba(0.0, 0.0, 0.0, 0.3));

        // Draw horizontal grid lines (dB levels)
        let grid_color = Color::from_rgba(1.0, 1.0, 1.0, 0.08);
        let zero_db_color = Color::from_rgba(1.0, 1.0, 1.0, 0.2);

        for &(db, label) in &SPECTRUM_DB_LABELS {
            let y = top_margin + ((db_max - db as f32) / db_range) * graph_height;
            let line = Path::line(
                Point::new(left_margin, y),
                Point::new(left_margin + graph_width, y),
            );
            let color = if db == 0 { zero_db_color } else { grid_color };
            frame.stroke(
                &line,
                Stroke::default()
                    .with_color(color)
                    .with_width(self.tokens.size(1.0)),
            );

            // dB label on left side
            let text = Text {
                content: label.to_string(),
                position: Point::new(left_margin - self.tokens.size(8.0), y),
                color: Color::from_rgba(1.0, 1.0, 1.0, 0.5),
                size: iced::Pixels(self.tokens.size(10.0)),
                align_x: iced::alignment::Horizontal::Right.into(),
                align_y: iced::alignment::Vertical::Center,
                ..Text::default()
            };
            frame.fill_text(text);
        }

        // Draw vertical grid lines (frequency markers)
        for &(label, freq) in &SPECTRUM_FREQ_LABELS {
            let x = left_margin + freq_to_x(freq, graph_width);
            let line = Path::line(
                Point::new(x, top_margin),
                Point::new(x, top_margin + graph_height),
            );
            frame.stroke(
                &line,
                Stroke::default()
                    .with_color(grid_color)
                    .with_width(self.tokens.size(1.0)),
            );

            // Frequency label
            let text = Text {
                content: label.to_string(),
                position: Point::new(x, top_margin + graph_height + self.tokens.size(12.0)),
                color: Color::from_rgba(1.0, 1.0, 1.0, 0.5),
                size: iced::Pixels(self.tokens.size(10.0)),
                align_x: iced::alignment::Horizontal::Center.into(),
                align_y: iced::alignment::Vertical::Top,
                ..Text::default()
            };
            frame.fill_text(text);
        }

        // Draw spectrum visualization
        let num_bars = self.spectrum_db.len();
        if num_bars > 0 {
            if self.bars_mode {
                // Bar mode
                let bar_gap = self.tokens.size(1.0);
                let bar_width = (graph_width / num_bars as f32).max(bar_gap) - bar_gap;

                for (i, &db) in self.spectrum_db.iter().enumerate() {
                    let t = i as f32 / num_bars as f32;
                    let x = left_margin + t * graph_width;
                    let normalized = ((db - db_min) / db_range).clamp(0.0, 1.0);
                    let bar_height = normalized * graph_height;
                    let y = top_margin + graph_height - bar_height;

                    if bar_height < self.tokens.size(1.0) {
                        continue;
                    }

                    let color = db_to_color(db);
                    let bar = Path::new(|builder| {
                        builder.move_to(Point::new(x, top_margin + graph_height));
                        builder.line_to(Point::new(x, y));
                        builder.line_to(Point::new(x + bar_width, y));
                        builder.line_to(Point::new(x + bar_width, top_margin + graph_height));
                        builder.close();
                    });
                    frame.fill(&bar, color);
                }
            } else {
                // Line/curve mode with gradient fill
                if num_bars >= 2 {
                    // Draw filled area under curve
                    let fill_path = Path::new(|builder| {
                        builder.move_to(Point::new(left_margin, top_margin + graph_height));
                        for (i, &db) in self.spectrum_db.iter().enumerate() {
                            let t = i as f32 / num_bars as f32;
                            let x = left_margin + t * graph_width;
                            let normalized = ((db - db_min) / db_range).clamp(0.0, 1.0);
                            let y = top_margin + graph_height - normalized * graph_height;
                            builder.line_to(Point::new(x, y));
                        }
                        builder.line_to(Point::new(
                            left_margin + graph_width,
                            top_margin + graph_height,
                        ));
                        builder.close();
                    });
                    frame.fill(&fill_path, Color::from_rgba(0.9, 0.3, 0.5, 0.3));

                    // Draw curve line
                    let curve = Path::new(|builder| {
                        let mut started = false;
                        for (i, &db) in self.spectrum_db.iter().enumerate() {
                            let t = i as f32 / num_bars as f32;
                            let x = left_margin + t * graph_width;
                            let normalized = ((db - db_min) / db_range).clamp(0.0, 1.0);
                            let y = top_margin + graph_height - normalized * graph_height;

                            if !started {
                                builder.move_to(Point::new(x, y));
                                started = true;
                            } else {
                                builder.line_to(Point::new(x, y));
                            }
                        }
                    });
                    frame.stroke(
                        &curve,
                        Stroke::default()
                            .with_color(theme::ACCENT_PINK)
                            .with_width(self.tokens.size(2.0)),
                    );
                }
            }
        }

        // Draw info text at bottom right (inside graph area)
        let info_text = format!("FFT: {}  SR: {}Hz", FFT_SIZE, self.sample_rate);
        let info = Text {
            content: info_text,
            position: Point::new(
                left_margin + graph_width - self.tokens.size(4.0),
                top_margin + graph_height - self.tokens.size(4.0),
            ),
            color: Color::from_rgba(1.0, 1.0, 1.0, 0.35),
            size: iced::Pixels(self.tokens.size(10.0)),
            align_x: iced::alignment::Horizontal::Right.into(),
            align_y: iced::alignment::Vertical::Bottom,
            ..Text::default()
        };
        frame.fill_text(info);

        vec![frame.into_geometry()]
    }
}

/// Convert frequency to x position (hybrid log scale with compressed low end)
fn freq_to_x(freq: f32, width: f32) -> f32 {
    let min_freq = 32.0_f32;
    let max_freq = 16000.0_f32;

    // First apply logarithmic mapping
    let log_min = min_freq.ln();
    let log_max = max_freq.ln();
    let log_freq = freq.clamp(min_freq, max_freq).ln();
    let t = (log_freq - log_min) / (log_max - log_min);

    // Then apply power function to compress low frequencies further
    // power < 1 compresses low end (where t is small)
    let power = 1.0_f32;
    let x = t.powf(power);

    x * width
}

/// Convert dB value to color (gradient from dark blue to pink/red)
fn db_to_color(db: f32) -> Color {
    // Normalize to 0-1 range (-60dB to +12dB)
    let t = ((db + 60.0) / 72.0).clamp(0.0, 1.0);

    if t < 0.5 {
        // Low levels: dark blue to cyan
        let s = t * 2.0;
        Color::from_rgba(0.1 + s * 0.2, 0.2 + s * 0.4, 0.4 + s * 0.3, 0.7)
    } else if t < 0.8 {
        // Mid levels: cyan to pink
        let s = (t - 0.5) / 0.3;
        Color::from_rgba(0.3 + s * 0.6, 0.6 - s * 0.2, 0.7 - s * 0.2, 0.8)
    } else {
        // High levels: pink to red (hot)
        let s = (t - 0.8) / 0.2;
        Color::from_rgba(0.9 + s * 0.05, 0.4 - s * 0.2, 0.5 - s * 0.3, 0.9)
    }
}

// ============================================================================
// Slider Components
// ============================================================================

/// Format dB value for display
fn format_db(value: f32) -> String {
    if value >= 0.0 {
        format!("+{:.1}", value)
    } else {
        format!("{:.1}", value)
    }
}

/// Render the ten EQ bands and preamp using the profile's composition policy.
fn sliders_with_preamp(
    eq_values: [f32; 10],
    preamp: f32,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    let slider_height = tokens.size(180.0);
    let slider_width = tokens.size(40.0);

    let preamp_slider = column![
        text("PREAMP")
            .size(tokens.text(TextRole::Caption))
            .style(|theme| text::Style {
                color: Some(theme::settings_desc(theme))
            }),
        Space::new().height(tokens.space(4.0)),
        text(format_db(preamp))
            .size(tokens.text(TextRole::Micro))
            .style(move |theme| text::Style {
                color: Some(if preamp != 0.0 {
                    theme::ACCENT_PINK
                } else {
                    theme::settings_value(theme)
                })
            }),
        Space::new().height(tokens.space(4.0)),
        vertical_slider(-12.0..=12.0, preamp, tokens, Message::UpdateEqualizerPreamp,)
            .step(0.5)
            .width(slider_width)
            .height(slider_height),
    ]
    .spacing(0)
    .align_x(Alignment::Center)
    .width(Length::Fixed(tokens.size(60.0)));

    let separator = container(
        Space::new()
            .width(tokens.size(1.0))
            .height(slider_height + tokens.space(20.0)),
    )
    .style(|theme| container::Style {
        background: Some(Background::Color(theme::divider(theme))),
        ..Default::default()
    });

    let band_sliders: Vec<Element<'static, Message>> = (0..10)
        .map(|i| vertical_slider_band(i, eq_values[i], FREQ_LABELS[i], eq_values, context))
        .collect();

    match context.profile {
        LayoutProfile::Expanded | LayoutProfile::Standard | LayoutProfile::Compact => row![
            preamp_slider,
            Space::new().width(tokens.space(16.0)),
            separator,
            Space::new().width(tokens.space(16.0)),
            row(band_sliders).spacing(tokens.space(4.0)).width(Fill),
        ]
        .align_y(Alignment::End)
        .width(Fill)
        .into(),
        LayoutProfile::Tablet => {
            let mut bands = band_sliders.into_iter();
            let first_row = row(bands.by_ref().take(5).collect::<Vec<_>>())
                .spacing(tokens.space(4.0))
                .width(Fill);
            let second_row = row(bands.collect::<Vec<_>>())
                .spacing(tokens.space(4.0))
                .width(Fill);
            column![
                row![preamp_slider, first_row]
                    .spacing(tokens.space(12.0))
                    .align_y(Alignment::End)
                    .width(Fill),
                second_row,
            ]
            .spacing(tokens.space(16.0))
            .width(Fill)
            .into()
        }
        LayoutProfile::Narrow => horizontal_eq_surface(eq_values, preamp, context),
    }
}

/// Single vertical slider band with label and value display
fn vertical_slider_band(
    index: usize,
    value: f32,
    freq_label: &'static str,
    eq_values: [f32; 10],
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    column![
        // Value display above slider
        text(format_db(value))
            .size(tokens.text(TextRole::Micro))
            .style(move |theme| text::Style {
                color: Some(if value != 0.0 {
                    theme::ACCENT_PINK
                } else {
                    theme::settings_value(theme)
                })
            })
            .align_x(Alignment::Center)
            .width(Fill),
        Space::new().height(tokens.space(4.0)),
        // Vertical slider
        vertical_slider(-12.0..=12.0, value, tokens, move |v| {
            let mut new_values = eq_values;
            new_values[index] = v;
            Message::UpdateEqualizerValues(new_values)
        })
        .step(0.5)
        .width(tokens.size(40.0))
        .height(tokens.size(180.0)),
        Space::new().height(tokens.space(8.0)),
        // Frequency label
        text(freq_label)
            .size(tokens.text(TextRole::Caption))
            .style(|theme| text::Style {
                color: Some(theme::settings_desc(theme))
            })
            .align_x(Alignment::Center)
            .width(Fill),
    ]
    .spacing(0)
    .align_x(Alignment::Center)
    .width(Fill)
    .into()
}

fn horizontal_eq_surface(
    eq_values: [f32; 10],
    preamp: f32,
    context: ResponsiveContext,
) -> Element<'static, Message> {
    let tokens = context.tokens;
    let mut controls: Vec<Element<'static, Message>> = Vec::with_capacity(11);
    controls.push(horizontal_slider_band(
        "PREAMP",
        preamp,
        Message::UpdateEqualizerPreamp,
        context,
    ));
    for (index, value) in eq_values.into_iter().enumerate() {
        let label = FREQ_LABELS[index];
        let values = eq_values;
        controls.push(horizontal_slider_band(
            label,
            value,
            move |new_value| {
                let mut next = values;
                next[index] = new_value;
                Message::UpdateEqualizerValues(next)
            },
            context,
        ));
    }

    crate::ui::widgets::scaled_scroll(
        scrollable(
            row(controls)
                .spacing(tokens.space(12.0))
                .padding(tokens.space(4.0)),
        )
        .direction(crate::ui::widgets::hidden_horizontal_scrollbar())
        .id(iced::widget::Id::new("audio_eq_horizontal_scroll"))
        .width(Fill),
        tokens,
    )
    .into()
}

fn horizontal_slider_band<F>(
    label: &'static str,
    value: f32,
    on_change: F,
    context: ResponsiveContext,
) -> Element<'static, Message>
where
    F: Fn(f32) -> Message + 'static,
{
    let tokens = context.tokens;
    column![
        text(label)
            .size(tokens.text(TextRole::Caption))
            .style(|theme| text::Style {
                color: Some(theme::settings_desc(theme)),
            }),
        text(format_db(value))
            .size(tokens.text(TextRole::Micro))
            .style(move |theme| text::Style {
                color: Some(if value != 0.0 {
                    theme::ACCENT_PINK
                } else {
                    theme::settings_value(theme)
                }),
            }),
        slider(-12.0..=12.0, value, on_change)
            .step(0.5)
            .width(tokens.size(120.0))
            .style(move |theme, status| rem_slider_style(theme, status, tokens)),
    ]
    .spacing(tokens.space(4.0))
    .width(Length::Fixed(tokens.size(140.0)))
    .into()
}

/// Preserve iced's slider palette while resolving its default visual geometry
/// through the application root rem.
fn rem_slider_style(theme: &Theme, status: slider::Status, tokens: UiTokens) -> slider::Style {
    let mut style = slider::default(theme, status);
    style.rail.width = tokens.size(4.0);
    style.rail.border.radius = tokens.size(2.0).into();
    style.handle.shape = slider::HandleShape::Circle {
        radius: tokens.size(7.0),
    };
    style
}
