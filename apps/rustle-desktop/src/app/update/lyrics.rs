// src/app/update/lyrics.rs
//! Lyrics page message handlers
//!
//! Architecture: Async-first loading to prevent UI blocking
//! - Background colors: extracted asynchronously
//! - Local/cached lyrics: loaded asynchronously
//! - Online lyrics: fetched asynchronously (already was)

use iced::Task;

use crate::app::message::Message;
use crate::app::state::{App, LyricsDisplayMode};
use crate::app::update::lyrics_preload_manager::DisplayFetchAction;

/// Renderer safety floor used to avoid zero-sized shaping/GPU buffers during
/// sensor churn. This is not an application visual dimension.
const MIN_RENDERER_VIEWPORT_EXTENT: f32 = 100.0;

impl App {
    pub(super) fn refresh_lyrics_display_font(&mut self) {
        use crate::features::lyrics::engine::{FontConfig, TextShaper};
        let mut config = FontConfig {
            font_family: self.core.settings.lyrics.lyrics_font_family.clone(),
            ..FontConfig::default()
        };
        if let Some(font_system) = &self.ui.lyrics.shared_font_system {
            config = TextShaper::resolve_font_config(font_system, config);
        }
        self.ui.lyrics.display_font = config
            .font_family
            .as_deref()
            .map(iced::Font::with_family)
            .unwrap_or_default()
            .weight(match config.font_weight.0 {
                0..=149 => iced::font::Weight::Thin,
                150..=249 => iced::font::Weight::ExtraLight,
                250..=349 => iced::font::Weight::Light,
                350..=449 => iced::font::Weight::Normal,
                450..=549 => iced::font::Weight::Medium,
                550..=649 => iced::font::Weight::Semibold,
                650..=749 => iced::font::Weight::Bold,
                750..=849 => iced::font::Weight::ExtraBold,
                _ => iced::font::Weight::Black,
            });
    }

    /// Handle lyrics page related messages
    pub fn handle_lyrics(&mut self, message: &Message) -> Option<Task<Message>> {
        match message {
            Message::OpenLyricsPage => {
                // Only open if there's a song playing
                if let Some(song) = self.playback.current_song.clone() {
                    if !self.ui.lyrics.is_open {
                        self.ui.lyrics.display_mode = LyricsDisplayMode::Artwork;
                    }
                    self.ui.lyrics.is_open = true;
                    if self.core.settings.display.power_saving_mode {
                        self.ui.lyrics.animation.settle_at(1.0);
                    } else {
                        self.ui.lyrics.animation.start();
                    }

                    // Render-ready source of truth: LyricsRenderManager
                    let viewport = self.current_lyrics_shape_metrics();
                    let font_family = self.core.settings.lyrics.lyrics_font_family.as_deref();
                    let render_ready = viewport.is_some_and(|(cw, fs)| {
                        self.playback.lyrics_render_manager.is_render_ready(
                            song.id,
                            cw,
                            fs,
                            font_family,
                        )
                    });

                    if render_ready {
                        // Pre-rendered result exists — install into UI/engine
                        if self.install_current_lyrics_render_if_ready(song.id) {
                            tracing::debug!(
                                "Installed/restored pre-rendered lyrics for song: {} (id={})",
                                song.title,
                                song.id
                            );
                        }

                        return Some(self.update_background_async(&song));
                    }

                    let lyrics_need_load = self.should_load_lyrics_for_song(song.id);

                    if lyrics_need_load {
                        tracing::debug!("Loading lyrics for song: {} (id={})", song.title, song.id);
                        return Some(self.load_lyrics_async(&song));
                    } else {
                        tracing::debug!(
                            "Lyrics already loaded for song: {} (id={})",
                            song.title,
                            song.id
                        );
                        self.restore_cached_shaped_lines_to_engine();
                        // Still need to update background if cover changed
                        return Some(Task::batch([
                            self.request_lyrics_shaping_for_current_viewport(),
                            self.update_background_async(&song),
                        ]));
                    }
                }
                Some(Task::none())
            }

            Message::ShowLyricsContent => {
                if self.ui.lyrics.is_open {
                    self.ui.lyrics.display_mode = LyricsDisplayMode::Lyrics;
                }
                Some(Task::none())
            }

            Message::ShowLyricsArtwork => {
                if self.ui.lyrics.is_open {
                    self.ui.lyrics.display_mode = LyricsDisplayMode::Artwork;
                }
                Some(Task::none())
            }

            Message::CloseLyricsPage => {
                if self.core.settings.display.power_saving_mode {
                    self.ui.lyrics.animation.settle_at(0.0);
                    self.ui.lyrics.is_open = false;
                    self.ui.lyrics.pending_viewport_size = None;
                    self.ui.lyrics.last_update = None;
                } else {
                    // Start close animation, actual close happens when animation completes
                    self.ui.lyrics.animation.stop();
                }
                Some(Task::none())
            }

            &Message::LyricsScroll(delta) => {
                self.handle_lyrics_scroll(delta);
                Some(Task::none())
            }

            &Message::LyricsSeek(time_ms) => {
                if self.core.settings.display.power_saving_mode
                    || !self
                        .ui
                        .lyrics
                        .engine
                        .as_ref()
                        .is_some_and(|engine| engine.borrow().is_manually_scrolling())
                {
                    return Some(Task::none());
                }
                self.refresh_playback_runtime();
                let runtime = self.playback_runtime();
                if !self.ui.lyrics.is_open
                    || !runtime.has_loaded_audio
                    || runtime.info.duration.is_zero()
                {
                    return Some(Task::none());
                }

                let position = std::time::Duration::from_millis(time_ms).min(runtime.info.duration);
                self.ui.seek_preview_position = None;
                self.seek_to_position(position);
                if let Some(engine) = &self.ui.lyrics.engine {
                    engine.borrow_mut().resume_auto_follow();
                }
                Some(Task::none())
            }

            Message::LyricsViewportResized(size) => {
                if self.ui.lyrics.animation.is_animating() {
                    self.ui.lyrics.pending_viewport_size = Some(*size);
                    return Some(Task::none());
                }

                Some(self.apply_lyrics_viewport_size(*size))
            }

            Message::WindowResized(size) => {
                self.core.window_width = size.width;
                self.core.window_height = size.height;
                // Layout-only responsive decisions are handled by the root
                // `Responsive` view. Lyrics renderer dimensions come from its
                // own `Sensor` so a guessed split ratio cannot overwrite the
                // actual measured renderer viewport during a resize.
                let responsive_context =
                    crate::ui::responsive::ResponsiveContext::from_viewport(*size);

                if let Some(engine_cell) = &self.ui.lyrics.engine {
                    engine_cell
                        .borrow_mut()
                        .set_visual_scale(responsive_context.root_rem.scale());
                }

                Some(Self::sync_window_maximized_after_resize_task())
            }

            // Handle async FontSystem initialization
            Message::LyricsFontSystemReady(font_system) => {
                tracing::info!("FontSystem ready for lyrics");
                self.ui.lyrics.shared_font_system = Some(font_system.clone());
                self.ui.lyrics.font_families =
                    crate::platform::theme::list_installed_font_families(font_system.lock().db());
                self.refresh_lyrics_display_font();

                // Create LyricsEngine with the shared font system
                if self.ui.lyrics.engine.is_none() {
                    let context = crate::ui::responsive::ResponsiveContext::from_viewport(
                        iced::Size::new(self.core.window_width, self.core.window_height),
                    );
                    let config = crate::features::lyrics::engine::LyricsEngineConfig::default()
                        .with_visual_scale(context.root_rem.scale());
                    self.ui.lyrics.engine = Some(std::cell::RefCell::new(
                        crate::features::lyrics::engine::LyricsEngine::new(config),
                    ));
                    tracing::info!("LyricsEngine created with shared FontSystem");
                }

                if let (Some(engine_cell), Some(shaped_lines)) = (
                    &self.ui.lyrics.engine,
                    self.ui.lyrics.cached_shaped_lines.as_ref(),
                ) {
                    let mut engine = engine_cell.borrow_mut();
                    engine.set_cached_shaped_lines_arc_with_metrics(
                        shaped_lines.clone(),
                        self.ui.lyrics.shaped_content_width,
                        self.ui.lyrics.shaped_font_size,
                    );
                }

                Some(self.request_lyrics_shaping_for_current_viewport())
            }

            Message::FetchLyricsOnline(song_id, ncm_id) => {
                let song_id = *song_id;
                let ncm_id = *ncm_id;

                if self.ui.lyrics.pending_song_id != Some(song_id) {
                    return Some(Task::none());
                }

                match self
                    .playback
                    .lyrics_preload_manager
                    .register_display_fetch(song_id, ncm_id)
                {
                    DisplayFetchAction::UseCache => {
                        Some(self.resume_display_lyrics_load_from_cache(song_id))
                    }
                    DisplayFetchAction::AwaitExisting => Some(Task::none()),
                    DisplayFetchAction::StartFetch => {
                        if let Some(client) = self.core.ncm_client.clone() {
                            let metadata = self.online_lyrics_metadata(song_id);
                            let proxy_url = self.core.settings.network.proxy_url();
                            Some(Task::perform(
                                async move {
                                    match crate::features::lyrics::fetch_lyrics(
                                        &client, ncm_id, metadata, proxy_url,
                                    )
                                    .await
                                    {
                                        Ok(lines) => {
                                            let ui_lines =
                                                crate::application::lyrics::project_lyrics(lines);
                                            Message::LyricsLoaded(song_id, ui_lines)
                                        }
                                        Err(e) => Message::LyricsLoadFailed(
                                            song_id,
                                            crate::error::AppError::with_message_source(
                                                crate::error::ErrorCode::MediaReadFailed,
                                                "Rustle could not load lyrics",
                                                e.to_string(),
                                            ),
                                        ),
                                    }
                                },
                                |msg| msg,
                            ))
                        } else {
                            self.playback
                                .lyrics_preload_manager
                                .finish_warmup(song_id, Err("No NCM client".to_string()));
                            Some(Task::done(Message::LyricsLoadFailed(
                                song_id,
                                crate::error::AppError::new(
                                    crate::error::ErrorCode::AuthenticationRequired,
                                    "Please sign in to load online lyrics",
                                ),
                            )))
                        }
                    }
                }
            }

            Message::WarmLyricsCache(song_id, ncm_id) => {
                let song_id = *song_id;
                let ncm_id = *ncm_id;

                if !self
                    .playback
                    .lyrics_preload_manager
                    .begin_warmup(song_id, ncm_id)
                {
                    return Some(Task::none());
                }

                if let Some(client) = self.core.ncm_client.clone() {
                    let metadata = self.online_lyrics_metadata(song_id);
                    let proxy_url = self.core.settings.network.proxy_url();
                    Some(Task::perform(
                        async move {
                            crate::features::lyrics::fetch_lyrics(
                                &client, ncm_id, metadata, proxy_url,
                            )
                            .await
                            .map(|_| ())
                            .map_err(|err| {
                                crate::error::AppError::with_message_source(
                                    crate::error::ErrorCode::MediaReadFailed,
                                    "Rustle could not warm the lyrics cache",
                                    err.to_string(),
                                )
                            })
                        },
                        move |result| Message::LyricsWarmupFinished(song_id, result),
                    ))
                } else {
                    Some(Task::done(Message::LyricsWarmupFinished(
                        song_id,
                        Err(crate::error::AppError::new(
                            crate::error::ErrorCode::AuthenticationRequired,
                            "Please sign in to load online lyrics",
                        )),
                    )))
                }
            }

            Message::LyricsWarmupFinished(song_id, result) => {
                match result {
                    Ok(()) => tracing::debug!("Lyrics warmup completed for song {}", song_id),
                    Err(error) => {
                        tracing::debug!("Lyrics warmup failed for song {}: {}", song_id, error)
                    }
                }
                self.playback.lyrics_preload_manager.finish_warmup(
                    *song_id,
                    result
                        .as_ref()
                        .map(|_| ())
                        .map_err(|error| error.user_summary().to_owned()),
                );
                if result.is_ok() {
                    self.playback
                        .preload_coordinator
                        .ensure_lyrics_slot(*song_id);
                    self.playback
                        .preload_coordinator
                        .mark_lyrics_text_ready(*song_id);
                }

                if self.ui.lyrics.pending_song_id == Some(*song_id) {
                    return Some(match result {
                        Ok(()) => self.resume_display_lyrics_load_from_cache(*song_id),
                        Err(error) => {
                            Task::done(Message::LyricsLoadFailed(*song_id, error.clone()))
                        }
                    });
                }
                if result.is_ok() && self.ui.lyrics.is_open {
                    return Some(self.schedule_adjacent_lyrics_render_prep());
                }
                Some(Task::none())
            }

            Message::LyricsLoaded(song_id, lines) => {
                if self.ui.lyrics.pending_song_id == Some(*song_id) {
                    self.note_lyrics_cache_ready_if_available(*song_id);
                    self.playback
                        .preload_coordinator
                        .mark_lyrics_text_ready(*song_id);
                    self.apply_lyrics_lines(*song_id, lines.clone());
                    tracing::info!(
                        "Loaded {} online lyrics lines for song {}",
                        lines.len(),
                        song_id
                    );

                    return Some(Self::prepare_engine_lines_task(*song_id, lines.clone()));
                }
                Some(Task::none())
            }

            Message::LyricsLoadFailed(song_id, error) => {
                if self.ui.lyrics.pending_song_id == Some(*song_id) {
                    if *song_id < 0 {
                        self.playback
                            .lyrics_preload_manager
                            .finish_warmup(*song_id, Err(error.user_summary().to_owned()));
                    }
                    self.apply_lyrics_error(*song_id, error);
                    tracing::warn!("Failed to load lyrics for song {}: {}", song_id, error);
                }
                Some(Task::none())
            }

            Message::LocalLyricsReady(song_id, lines) => {
                if self.ui.lyrics.pending_song_id == Some(*song_id) {
                    self.note_lyrics_cache_ready_if_available(*song_id);
                    self.playback
                        .preload_coordinator
                        .mark_lyrics_text_ready(*song_id);
                    self.apply_lyrics_lines(*song_id, lines.clone());
                    tracing::info!(
                        "Loaded {} local/cached lyrics lines for song {}",
                        lines.len(),
                        song_id
                    );

                    return Some(Self::prepare_engine_lines_task(*song_id, lines.clone()));
                }
                Some(Task::none())
            }

            // Handle pre-computed engine lines
            Message::AdjacentLyricsPrepared(song_id, generation, lines) => {
                if !self
                    .playback
                    .lyrics_render_manager
                    .finish_engine_preparation(*song_id, *generation, lines.is_some())
                {
                    return Some(Task::none());
                }
                match lines {
                    Some(lines) => self
                        .handle_lyrics(&Message::LyricsEngineLinesReady(*song_id, lines.clone())),
                    None => Some(Task::none()),
                }
            }
            Message::LyricsCacheMiss(song_id) => {
                if self.ui.lyrics.pending_song_id != Some(*song_id) || *song_id >= 0 {
                    return Some(Task::none());
                }
                self.playback
                    .lyrics_preload_manager
                    .finish_warmup(*song_id, Err("Cached lyrics are unavailable".into()));
                Some(Task::done(Message::FetchLyricsOnline(
                    *song_id,
                    (-*song_id) as u64,
                )))
            }
            Message::LyricsEngineLinesReady(song_id, engine_lines) => {
                // Evicted adjacent work must not recreate cache entries/jobs.
                if self.ui.lyrics.displayed_song_id != Some(*song_id)
                    && self.playback.lyrics_render_manager.get(*song_id).is_none()
                {
                    return Some(Task::none());
                }
                self.playback
                    .lyrics_render_manager
                    .store_engine_lines(*song_id, engine_lines.clone());
                self.playback
                    .preload_coordinator
                    .ensure_lyrics_slot(*song_id);
                self.playback
                    .preload_coordinator
                    .mark_lyrics_engine_lines_ready(*song_id);

                if self.ui.lyrics.displayed_song_id == Some(*song_id) {
                    self.ui.lyrics.cached_engine_lines = Some(engine_lines.clone());
                    tracing::info!(
                        "Engine lines ready for song {}: {} lines",
                        song_id,
                        engine_lines.len()
                    );

                    return Some(self.request_lyrics_shaping_for_current_viewport());
                }

                if self.ui.lyrics.is_open {
                    return Some(self.request_lyrics_shaping_for_song(*song_id));
                }
                Some(Task::none())
            }

            Message::LyricsShapedLinesReady(song_id, generation, shaped_lines) => {
                if !self.playback.lyrics_render_manager.finish_shape(
                    *song_id,
                    *generation,
                    shaped_lines.clone(),
                ) {
                    return Some(Task::none());
                }
                let entry = self
                    .playback
                    .lyrics_render_manager
                    .get(*song_id)
                    .expect("accepted render entry");
                self.playback
                    .preload_coordinator
                    .ensure_lyrics_slot(*song_id);
                self.playback
                    .preload_coordinator
                    .mark_lyrics_shaped_lines_ready(
                        *song_id,
                        *generation,
                        entry.content_width,
                        entry.font_size,
                    );
                if self.ui.lyrics.displayed_song_id == Some(*song_id) {
                    self.install_current_lyrics_render_if_ready(*song_id);
                }
                Some(Task::none())
            }

            Message::LyricsShapingFailed(song_id, generation) => {
                self.playback
                    .lyrics_render_manager
                    .fail_shape(*song_id, *generation);
                Some(Task::none())
            }

            // Background color extraction result
            Message::LyricsBackgroundReady(song_id, cover_path, primary, secondary, tertiary) => {
                // Always store in coordinator for any song in the window
                self.playback.preload_coordinator.store_background_colors(
                    *song_id,
                    cover_path.clone(),
                    *primary,
                    *secondary,
                    *tertiary,
                );

                // Install into shader only if this is the current song
                if self.background_result_matches_current_song(*song_id, cover_path) {
                    self.ui
                        .lyrics
                        .bg_shader
                        .set_colors(*primary, *secondary, *tertiary);
                    self.ui.lyrics.bg_colors = crate::utils::DominantColors {
                        primary: iced::Color::from_rgba(
                            primary[0], primary[1], primary[2], primary[3],
                        ),
                        secondary: iced::Color::from_rgba(
                            secondary[0],
                            secondary[1],
                            secondary[2],
                            secondary[3],
                        ),
                        tertiary: iced::Color::from_rgba(
                            tertiary[0],
                            tertiary[1],
                            tertiary[2],
                            tertiary[3],
                        ),
                    };
                    tracing::debug!("Applied background colors for song {}", song_id);
                }
                Some(Task::none())
            }

            // Cover image loading result
            Message::LyricsCoverImageReady(song_id, cover_path, image_data, width, height) => {
                // Always store in coordinator for any song in the window
                self.playback.preload_coordinator.store_background_texture(
                    *song_id,
                    cover_path.clone(),
                    image_data.clone(),
                    *width,
                    *height,
                );

                // Install into shader only if this is the current song
                if self.background_result_matches_current_song(*song_id, cover_path)
                    && let Some(img) =
                        image::RgbImage::from_raw(*width, *height, image_data.clone())
                {
                    let dynamic_img = image::DynamicImage::ImageRgb8(img);
                    self.ui.lyrics.textured_bg_shader.set_album_image(
                        dynamic_img,
                        Some(std::path::PathBuf::from(cover_path.as_str())),
                    );
                    tracing::debug!(
                        "Applied cover image for song {} ({}x{})",
                        song_id,
                        width,
                        height
                    );
                }
                Some(Task::none())
            }

            _ => None,
        }
    }

    pub(super) fn current_lyrics_shape_metrics(&self) -> Option<(f32, f32)> {
        if !self.ui.lyrics.viewport_initialized {
            return None;
        }

        let viewport_width = self
            .ui
            .lyrics
            .viewport_width
            .max(MIN_RENDERER_VIEWPORT_EXTENT);
        let content_width = viewport_width * 0.9;
        let context = crate::ui::responsive::ResponsiveContext::from_viewport(iced::Size::new(
            self.core.window_width,
            self.core.window_height,
        ));
        let font_size = crate::features::lyrics::engine::FontSizeConfig::default()
            .calculate_font_size(context.root_rem.scale());

        Some((content_width, font_size))
    }

    fn apply_lyrics_viewport_size(&mut self, size: iced::Size) -> Task<Message> {
        if self.ui.lyrics.viewport_initialized
            && (self.ui.lyrics.viewport_width - size.width).abs() < 0.5
            && (self.ui.lyrics.viewport_height - size.height).abs() < 0.5
        {
            return Task::none();
        }

        self.ui.lyrics.viewport_width = size.width.max(MIN_RENDERER_VIEWPORT_EXTENT);
        self.ui.lyrics.viewport_height = size.height.max(MIN_RENDERER_VIEWPORT_EXTENT);
        self.ui.lyrics.viewport_initialized = true;

        if let Some(engine_cell) = &self.ui.lyrics.engine {
            let mut engine = engine_cell.borrow_mut();
            engine
                .line_animations_mut()
                .set_viewport_height(self.ui.lyrics.viewport_height);
            engine.invalidate_layout();
        }

        self.request_lyrics_shaping_for_current_viewport()
    }

    pub(super) fn request_lyrics_shaping_for_current_viewport(&mut self) -> Task<Message> {
        let Some(song_id) = self.ui.lyrics.displayed_song_id else {
            return Task::none();
        };
        let Some(lines) = self.ui.lyrics.cached_engine_lines.clone() else {
            return Task::none();
        };
        self.playback
            .lyrics_render_manager
            .store_engine_lines(song_id, lines);
        let task = self.request_lyrics_shaping_for_song(song_id);
        self.install_current_lyrics_render_if_ready(song_id);
        task
    }

    fn request_lyrics_shaping_for_song(&mut self, song_id: i64) -> Task<Message> {
        if self.core.settings.display.power_saving_mode {
            return Task::none();
        }
        let Some(font_system) = self.ui.lyrics.shared_font_system.clone() else {
            return Task::none();
        };
        let Some((content_width, font_size)) = self.current_lyrics_shape_metrics() else {
            return Task::none();
        };
        let Some(request) = self.playback.lyrics_render_manager.request_shape(
            song_id,
            content_width,
            font_size,
            self.core.settings.lyrics.lyrics_font_family.clone(),
        ) else {
            return Task::none();
        };
        let generation = request.generation;
        Task::perform(
            async move {
                match tokio::task::spawn_blocking(move || {
                    use crate::features::lyrics::engine::{
                        CachedShapedLine, FontConfig, TextShaper, pre_generate_sdf_batch,
                    };
                    if request.is_cancelled() {
                        return None;
                    }
                    let started = std::time::Instant::now();
                    let config = crate::features::lyrics::engine::LyricsEngineConfig::default();
                    let trans_height_ratio = config.trans_height_ratio;
                    let roman_height_ratio = config.roman_height_ratio;
                    let bg_font_size_ratio = config.bg_font_size_ratio;
                    let text_shaper = match &request.font_family {
                        Some(family) => {
                            TextShaper::with_config(font_system, FontConfig::with_family(family))
                        }
                        None => TextShaper::new(font_system),
                    };
                    let shaped_lines: Option<Vec<CachedShapedLine>> = request
                        .engine_lines
                        .iter()
                        .map(|line| {
                            if request.is_cancelled() {
                                return None;
                            }
                            let main_font_size = if line.is_bg {
                                font_size * bg_font_size_ratio
                            } else {
                                font_size
                            };
                            let trans_font_size = (main_font_size * trans_height_ratio).max(10.0);
                            let roman_font_size = (main_font_size * roman_height_ratio).max(10.0);

                            let main_shaped = text_shaper.shape_line(
                                &line.text,
                                &line.words,
                                main_font_size,
                                content_width,
                            );
                            let mut total_height = main_shaped.height;

                            let translation_shaped = if let Some(ref translated) = line.translated {
                                if !translated.is_empty() {
                                    let shaped = text_shaper.shape_simple(
                                        translated,
                                        trans_font_size,
                                        content_width,
                                    );
                                    total_height += shaped.height;
                                    Some(shaped)
                                } else {
                                    None
                                }
                            } else {
                                None
                            };

                            let romanized_shaped = if let Some(ref romanized) = line.romanized {
                                if !romanized.is_empty() {
                                    let shaped = text_shaper.shape_simple(
                                        romanized,
                                        roman_font_size,
                                        content_width,
                                    );
                                    total_height += shaped.height;
                                    Some(shaped)
                                } else {
                                    None
                                }
                            } else {
                                None
                            };

                            Some(CachedShapedLine {
                                main: main_shaped,
                                main_font_size,
                                translation: translation_shaped,
                                translation_font_size: trans_font_size,
                                romanized: romanized_shaped,
                                romanized_font_size: roman_font_size,
                                total_height,
                            })
                        })
                        .collect();

                    let shaped_lines = shaped_lines?;
                    let cache_keys: Vec<cosmic_text::CacheKey> = shaped_lines
                        .iter()
                        .flat_map(|line| {
                            let main_keys = line.main.glyphs.iter().map(|g| g.cache_key);
                            let trans_keys = line
                                .translation
                                .iter()
                                .flat_map(|t| t.glyphs.iter().map(|g| g.cache_key));
                            let roman_keys = line
                                .romanized
                                .iter()
                                .flat_map(|r| r.glyphs.iter().map(|g| g.cache_key));
                            main_keys.chain(trans_keys).chain(roman_keys)
                        })
                        .collect();

                    let generated = pre_generate_sdf_batch(&cache_keys, &request.cancelled);
                    if request.is_cancelled() {
                        return None;
                    }
                    tracing::debug!(
                        song_id = request.song_id,
                        generation,
                        lines = shaped_lines.len(),
                        generated,
                        elapsed_ms = started.elapsed().as_millis(),
                        "lyrics render prepared"
                    );
                    Some(std::sync::Arc::new(shaped_lines))
                })
                .await
                {
                    Ok(Some(lines)) => Message::LyricsShapedLinesReady(song_id, generation, lines),
                    Ok(None) => Message::Noop,
                    Err(error) => {
                        tracing::warn!(song_id, generation, %error, "lyrics shaping worker failed");
                        Message::LyricsShapingFailed(song_id, generation)
                    }
                }
            },
            |message| message,
        )
    }

    /// Schedule render preparation for adjacent songs when lyrics page is open.
    /// This triggers engine lines → shaped lines → SDF in the background.
    pub(super) fn schedule_adjacent_lyrics_render_prep(&mut self) -> Task<Message> {
        let window = self.playback.preload_coordinator.window();
        let shape_metrics = self.current_lyrics_shape_metrics();
        let font_family = self.core.settings.lyrics.lyrics_font_family.clone();
        let mut tasks = Vec::new();

        for song_id in [window.next_song_id, window.prev_song_id]
            .into_iter()
            .flatten()
        {
            if let Some((cw, fs)) = shape_metrics
                && self.playback.lyrics_render_manager.is_render_ready(
                    song_id,
                    cw,
                    fs,
                    font_family.as_deref(),
                )
            {
                continue;
            }

            if self.playback.lyrics_render_manager.get(song_id).is_some() {
                tasks.push(self.request_lyrics_shaping_for_song(song_id));
                continue;
            }

            // Check if lyrics text is cached on disk
            let ncm_id = if song_id < 0 {
                (-song_id) as u64
            } else {
                continue;
            };
            if !crate::features::lyrics::has_cached_lyrics(ncm_id) {
                continue;
            }
            let Some(generation) = self
                .playback
                .lyrics_render_manager
                .begin_engine_preparation(song_id)
            else {
                continue;
            };

            tracing::info!(
                "Background render prep: preparing engine lines for adjacent song {}",
                song_id
            );

            // Read and parse the cache off the UI thread, then prepare engine
            // lines (the handler will store in manager and trigger shaping).
            tasks.push(Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        crate::features::lyrics::load_cached_lyrics(ncm_id)
                            .map(crate::application::lyrics::project_lyrics)
                            .map(|lines| Self::build_engine_lines(&lines))
                    })
                    .await
                    .unwrap_or_else(|error| {
                        tracing::warn!(song_id, %error, "Adjacent lyrics worker failed");
                        None
                    })
                },
                move |lines| Message::AdjacentLyricsPrepared(song_id, generation, lines),
            ));
        }

        if tasks.is_empty() {
            Task::none()
        } else {
            Task::batch(tasks)
        }
    }

    /// Async background preparation for a single song (colors + cover image).
    /// Returns two independent tasks that store results in coordinator via messages.
    fn prepare_background_for_song_task(
        song_id: i64,
        cover_path: String,
        artwork: Option<std::sync::Arc<image::DynamicImage>>,
    ) -> (Task<Message>, Task<Message>) {
        use crate::ui::effects::background::color_to_array;

        let path_for_image = cover_path.clone();
        let path_for_colors = cover_path;
        let path_msg_img = path_for_image.clone();
        let path_msg_colors = path_for_colors.clone();

        let image_artwork = artwork.clone();
        let image_task = Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    match image_artwork.or_else(|| {
                        crate::image::artwork::load(std::path::Path::new(&path_for_image))
                            .map(std::sync::Arc::new)
                    }) {
                        Some(img) => {
                            let rgb = img.to_rgb8();
                            let (width, height) = rgb.dimensions();
                            Some((song_id, path_msg_img, rgb.into_raw(), width, height))
                        }
                        None => {
                            tracing::warn!("Background prep: failed to load cover");
                            None
                        }
                    }
                })
                .await
                .ok()
                .flatten()
            },
            |result| match result {
                Some((song_id, path, data, w, h)) => {
                    Message::LyricsCoverImageReady(song_id, path, data, w, h)
                }
                None => Message::Noop,
            },
        );

        let colors_task = Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    artwork
                        .as_deref()
                        .map(crate::utils::DominantColors::from_image)
                        .or_else(|| {
                            crate::image::artwork::load(std::path::Path::new(&path_for_colors))
                                .map(|img| crate::utils::DominantColors::from_image(&img))
                        })
                        .map(|colors| {
                            (
                                song_id,
                                path_msg_colors,
                                color_to_array(colors.primary),
                                color_to_array(colors.secondary),
                                color_to_array(colors.tertiary),
                            )
                        })
                })
                .await
                .ok()
                .flatten()
            },
            |result| match result {
                Some((song_id, path, primary, secondary, tertiary)) => {
                    Message::LyricsBackgroundReady(song_id, path, primary, secondary, tertiary)
                }
                None => Message::Noop,
            },
        );

        (image_task, colors_task)
    }

    pub(super) fn prepare_lyrics_background_for_cover_path(
        &mut self,
        song_id: i64,
        cover_path: std::path::PathBuf,
    ) -> Task<Message> {
        let artwork = self
            .ui
            .image_state
            .entries
            .values()
            .find(|entry| entry.path == cover_path)
            .and_then(|entry| entry.artwork.clone());
        let cover_path = cover_path.to_string_lossy().to_string();
        self.playback
            .preload_coordinator
            .ensure_background_slot(song_id, Some(cover_path.clone()));

        if self
            .playback
            .preload_coordinator
            .is_background_ready(song_id, Some(&cover_path))
        {
            return Task::none();
        }

        let (image_task, colors_task) =
            Self::prepare_background_for_song_task(song_id, cover_path, artwork);
        Task::batch([image_task, colors_task])
    }

    /// Schedule background prep for adjacent songs in the preload window.
    pub(super) fn schedule_background_prep(&mut self) -> Task<Message> {
        let window = self.playback.preload_coordinator.window();
        let mut tasks = Vec::new();

        for song_id in [window.next_song_id, window.prev_song_id]
            .into_iter()
            .flatten()
        {
            // Find cover path from queue / artwork cache
            let Some(song) = self
                .playback
                .queue
                .iter()
                .find(|s| s.id == song_id)
                .cloned()
            else {
                continue;
            };

            let Some(cover_path) = self.resolved_song_cover_local_path(&song) else {
                continue;
            };

            tasks.push(self.prepare_lyrics_background_for_cover_path(song_id, cover_path));
        }

        if tasks.is_empty() {
            Task::none()
        } else {
            Task::batch(tasks)
        }
    }

    fn should_load_lyrics_for_song(&self, song_id: i64) -> bool {
        (self.ui.lyrics.displayed_song_id != Some(song_id)
            && self.ui.lyrics.pending_song_id != Some(song_id))
            || self.ui.lyrics.load_error.is_some()
    }

    fn restore_cached_shaped_lines_to_engine(&mut self) {
        let (Some(engine_cell), Some(shaped_lines)) = (
            &self.ui.lyrics.engine,
            self.ui.lyrics.cached_shaped_lines.as_ref(),
        ) else {
            return;
        };

        let mut engine = engine_cell.borrow_mut();
        engine.set_cached_shaped_lines_arc_with_metrics(
            shaped_lines.clone(),
            self.ui.lyrics.shaped_content_width,
            self.ui.lyrics.shaped_font_size,
        );
    }

    /// Install pre-rendered lyrics from the render manager into UI state and engine.
    pub(super) fn install_current_lyrics_render_if_ready(&mut self, song_id: i64) -> bool {
        let Some((content_width, font_size)) = self.current_lyrics_shape_metrics() else {
            return false;
        };
        let font_family = self.core.settings.lyrics.lyrics_font_family.as_deref();
        if !self.playback.lyrics_render_manager.is_render_ready(
            song_id,
            content_width,
            font_size,
            font_family,
        ) {
            return false;
        }

        self.install_lyrics_from_render_manager(song_id)
    }

    /// Install pre-rendered lyrics from the render manager into UI state and engine.
    /// Used when the render manager has shaped lines for the current song but
    /// UI cache hasn't been populated yet (e.g., background render prep completed first).
    fn install_lyrics_from_render_manager(&mut self, song_id: i64) -> bool {
        let Some((shaped_lines, engine_lines, content_width, font_size)) = self
            .playback
            .lyrics_render_manager
            .get(song_id)
            .and_then(|entry| {
                let (Some(shaped_lines), Some(engine_lines)) =
                    (&entry.shaped_lines, &entry.engine_lines)
                else {
                    return None;
                };
                Some((
                    shaped_lines.clone(),
                    engine_lines.clone(),
                    entry.content_width,
                    entry.font_size,
                ))
            })
        else {
            return false;
        };

        let source_changed = !self
            .ui
            .lyrics
            .cached_engine_lines
            .as_ref()
            .is_some_and(|old| std::sync::Arc::ptr_eq(old, &engine_lines));
        self.ui.lyrics.displayed_song_id = Some(song_id);
        self.ui.lyrics.pending_song_id = None;
        if source_changed {
            self.ui.lyrics.lines = Self::engine_lines_to_ui_lines(&engine_lines);
        }
        self.ui.lyrics.cached_engine_lines = Some(engine_lines);
        self.ui.lyrics.cached_shaped_lines = Some(shaped_lines.clone());
        self.ui.lyrics.shaped_content_width = content_width;
        self.ui.lyrics.shaped_font_size = font_size;
        self.ui.lyrics.is_loading = false;
        self.ui.lyrics.load_error = None;

        if let Some(engine_cell) = &self.ui.lyrics.engine {
            let mut engine = engine_cell.borrow_mut();
            if source_changed {
                engine.reset_for_new_lyrics();
            }
            engine.set_cached_shaped_lines_arc_with_metrics(shaped_lines, content_width, font_size);
        }

        tracing::info!(
            "Installed pre-rendered lyrics for song {} from render manager",
            song_id
        );
        true
    }

    fn background_result_matches_current_song(&self, song_id: i64, cover_path: &str) -> bool {
        let Some(current) = self.playback.current_song.as_ref() else {
            return false;
        };
        if current.id != song_id {
            return false;
        }

        self.resolved_song_cover_local_path(current)
            .is_some_and(|path| path.as_path() == std::path::Path::new(cover_path))
    }

    /// Install cached background colors + texture from coordinator into shader.
    fn install_background_from_coordinator(&mut self, song_id: i64) {
        let Some(prepared) = self.playback.preload_coordinator.background_data(song_id) else {
            return;
        };

        let super::preload_coordinator::PreparedBackground {
            cover_path,
            primary,
            secondary,
            tertiary,
            image_data,
            image_width: width,
            image_height: height,
        } = prepared;

        self.ui
            .lyrics
            .bg_shader
            .set_colors(primary, secondary, tertiary);
        self.ui.lyrics.bg_colors = crate::utils::DominantColors {
            primary: iced::Color::from_rgba(primary[0], primary[1], primary[2], primary[3]),
            secondary: iced::Color::from_rgba(
                secondary[0],
                secondary[1],
                secondary[2],
                secondary[3],
            ),
            tertiary: iced::Color::from_rgba(tertiary[0], tertiary[1], tertiary[2], tertiary[3]),
        };

        if let Some(img) = image::RgbImage::from_raw(width, height, image_data) {
            let dynamic_img = image::DynamicImage::ImageRgb8(img);
            self.ui
                .lyrics
                .textured_bg_shader
                .set_album_image(dynamic_img, cover_path.map(std::path::PathBuf::from));
        }

        tracing::info!(
            "Installed cached background for song {} from coordinator",
            song_id
        );
    }

    fn engine_lines_to_ui_lines(
        engine_lines: &[crate::features::lyrics::engine::LyricLineData],
    ) -> Vec<crate::application::lyrics::LyricLine> {
        engine_lines
            .iter()
            .map(|line| crate::application::lyrics::LyricLine {
                start_ms: line.start_ms,
                end_ms: line.end_ms,
                text: line.text.clone(),
                words: line
                    .words
                    .iter()
                    .map(|word| crate::application::lyrics::LyricWord {
                        start_ms: word.start_ms,
                        end_ms: word.end_ms,
                        word: word.text.clone(),
                    })
                    .collect(),
                translated: line.translated.clone(),
                romanized: line.romanized.clone(),
                is_background: line.is_bg,
                is_duet: line.is_duet,
            })
            .collect()
    }

    fn prepare_display_lyrics_load(&mut self, song_id: i64) {
        self.ui.lyrics.displayed_song_id = None;
        self.ui.lyrics.pending_song_id = Some(song_id);
        self.ui.lyrics.lines.clear();
        self.ui.lyrics.cached_engine_lines = None;
        self.ui.lyrics.cached_shaped_lines = None;
        self.ui.lyrics.shaped_content_width = 0.0;
        self.ui.lyrics.shaped_font_size = 0.0;
        self.ui.lyrics.pending_viewport_size = None;
        self.ui.lyrics.current_line_idx = None;
        self.ui.lyrics.is_loading = true;
        self.ui.lyrics.load_error = None;

        if let Some(engine_cell) = &self.ui.lyrics.engine {
            let mut engine = engine_cell.borrow_mut();
            engine.reset_for_new_lyrics();
            engine.set_cached_shaped_lines(Vec::new());
        }
    }

    fn apply_lyrics_lines(
        &mut self,
        song_id: i64,
        lines: Vec<crate::application::lyrics::LyricLine>,
    ) {
        self.ui.lyrics.displayed_song_id = Some(song_id);
        self.ui.lyrics.pending_song_id = None;
        self.ui.lyrics.lines = lines;
        self.ui.lyrics.cached_engine_lines = None;
        self.ui.lyrics.cached_shaped_lines = None;
        self.ui.lyrics.shaped_content_width = 0.0;
        self.ui.lyrics.shaped_font_size = 0.0;
        self.ui.lyrics.is_loading = false;
        self.ui.lyrics.load_error = None;
        self.ui.lyrics.current_line_idx = None;

        // Clear engine's cached data for re-layout, but keep the engine instance
        // (engine is pre-created at app startup to avoid FontSystem::new() delay)
        if let Some(engine_cell) = &self.ui.lyrics.engine {
            let mut engine = engine_cell.borrow_mut();
            engine.reset_for_new_lyrics();
            engine.set_cached_shaped_lines(Vec::new());
        }
    }

    fn apply_lyrics_error(&mut self, song_id: i64, error: &crate::error::AppError) {
        if self.ui.lyrics.pending_song_id != Some(song_id) {
            return;
        }

        self.ui.lyrics.displayed_song_id = None;
        self.ui.lyrics.pending_song_id = None;
        self.ui.lyrics.lines.clear();
        self.ui.lyrics.cached_engine_lines = None;
        self.ui.lyrics.cached_shaped_lines = None;
        self.ui.lyrics.shaped_content_width = 0.0;
        self.ui.lyrics.shaped_font_size = 0.0;
        self.ui.lyrics.is_loading = false;
        self.ui.lyrics.load_error = Some(error.user_summary().to_owned());
        self.ui.lyrics.current_line_idx = None;

        if let Some(engine_cell) = &self.ui.lyrics.engine {
            let mut engine = engine_cell.borrow_mut();
            engine.set_cached_shaped_lines(Vec::new());
        }
    }

    fn note_lyrics_cache_ready_if_available(&mut self, song_id: i64) {
        if song_id < 0 {
            let ncm_id = (-song_id) as u64;
            if crate::features::lyrics::has_cached_lyrics(ncm_id) {
                self.playback
                    .lyrics_preload_manager
                    .mark_ready(song_id, ncm_id);
            }
        }
    }

    fn resume_display_lyrics_load_from_cache(&mut self, song_id: i64) -> Task<Message> {
        let ncm_id = if song_id < 0 { (-song_id) as u64 } else { 0 };

        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    if song_id < 0 {
                        crate::features::lyrics::load_cached_lyrics(ncm_id)
                            .map(crate::application::lyrics::project_lyrics)
                    } else {
                        None
                    }
                })
                .await
                .ok()
                .flatten()
            },
            move |lines| match lines {
                Some(lines) => Message::LocalLyricsReady(song_id, lines),
                None => Message::LyricsCacheMiss(song_id),
            },
        )
    }

    fn build_engine_lines(
        lines_for_task: &[crate::application::lyrics::LyricLine],
    ) -> std::sync::Arc<Vec<crate::features::lyrics::engine::LyricLineData>> {
        let engine_lines: Vec<crate::features::lyrics::engine::LyricLineData> = lines_for_task
            .iter()
            .map(|line| {
                let word_count = line.words.len();
                crate::features::lyrics::engine::LyricLineData {
                    text: line.text.clone(),
                    words: line
                        .words
                        .iter()
                        .enumerate()
                        .map(|(i, w)| crate::features::lyrics::engine::WordData {
                            text: w.word.clone(),
                            start_ms: w.start_ms,
                            end_ms: w.end_ms,
                            emphasize: false,
                            is_last_word: i == word_count.saturating_sub(1),
                        })
                        .collect(),
                    translated: line.translated.clone(),
                    romanized: line.romanized.clone(),
                    start_ms: line.start_ms,
                    end_ms: line.end_ms,
                    is_duet: line.is_duet,
                    is_bg: line.is_background,
                }
            })
            .collect();
        std::sync::Arc::new(engine_lines)
    }

    fn prepare_engine_lines_task(
        song_id: i64,
        lines_for_task: Vec<crate::application::lyrics::LyricLine>,
    ) -> Task<Message> {
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    (song_id, Self::build_engine_lines(&lines_for_task))
                })
                .await
                .ok()
            },
            |result| {
                if let Some((song_id, engine_lines)) = result {
                    Message::LyricsEngineLinesReady(song_id, engine_lines)
                } else {
                    Message::Noop
                }
            },
        )
    }

    /// Check if lyrics page should be fully closed (animation complete)
    pub fn check_lyrics_page_close(&mut self) {
        let progress = self.ui.lyrics.animation.progress();
        if progress < 0.01 && !self.ui.lyrics.animation.is_animating() && self.ui.lyrics.is_open {
            self.ui.lyrics.is_open = false;
            self.ui.lyrics.pending_viewport_size = None;
            self.ui.lyrics.last_update = None;
        }
    }

    pub fn flush_pending_lyrics_viewport_after_animation(&mut self) -> Task<Message> {
        if self.ui.lyrics.animation.progress() < 0.99
            || self.ui.lyrics.animation.is_animating()
            || !self.ui.lyrics.is_open
        {
            return Task::none();
        }

        let Some(size) = self.ui.lyrics.pending_viewport_size.take() else {
            return Task::none();
        };

        self.apply_lyrics_viewport_size(size)
    }

    /// Update lyrics line animations based on current playback position
    pub fn update_lyrics_animations(&mut self) -> Task<Message> {
        let now = std::time::Instant::now();
        let delta_secs = if let Some(last) = self.ui.lyrics.last_update {
            let delta = now.duration_since(last).as_secs_f32();
            delta.clamp(0.001, 0.1)
        } else {
            0.016
        };
        self.ui.lyrics.last_update = Some(now);

        let power_saving = self.core.settings.display.power_saving_mode;

        if !power_saving && let Some(start_time) = self.ui.lyrics.shader_start_time {
            let elapsed_ms = now.duration_since(start_time).as_secs_f32() * 1000.0;
            let shader_time = elapsed_ms / 10000.0;
            self.ui.lyrics.bg_shader.set_time(elapsed_ms);
            self.ui.lyrics.textured_bg_shader.set_time(shader_time);
            self.ui
                .lyrics
                .textured_bg_shader
                .update(delta_secs * 1000.0);
        }

        if self.ui.lyrics.lines.is_empty() {
            return Task::none();
        }

        let runtime = self.playback_runtime();
        let position_ms = if runtime.has_loaded_audio && runtime.info.duration.as_secs_f32() > 0.0 {
            (runtime.info.position.as_secs_f32() * 1000.0) as u64
        } else {
            0
        };

        let new_current_line =
            crate::ui::pages::find_current_line(&self.ui.lyrics.lines, position_ms);

        if new_current_line != self.ui.lyrics.current_line_idx {
            self.ui.lyrics.current_line_idx = new_current_line;
        }

        if !power_saving {
            self.update_lyrics_engine(delta_secs);
        }

        Task::none()
    }

    /// Update lyrics engine with current state
    fn update_lyrics_engine(&mut self, delta_secs: f32) {
        // Engine is now pre-created at app startup, so just check if lines changed
        let just_initialized = false;

        let Some(engine_lines) = self.ui.lyrics.cached_engine_lines.clone() else {
            return;
        };
        let defer_layout_until_transition_finishes = self.ui.lyrics.animation.is_animating()
            && self.ui.lyrics.pending_viewport_size.is_some();

        let viewport_height = self.ui.lyrics.viewport_height;

        let runtime = self.playback_runtime();
        let time_ms = if runtime.has_loaded_audio && runtime.info.duration.as_secs_f32() > 0.0 {
            runtime.info.position.as_secs_f64() * 1000.0
        } else {
            self.playback
                .saved_state
                .as_ref()
                .map(|s| s.position_secs * 1000.0)
                .unwrap_or(0.0)
        };

        let is_playing = matches!(
            self.playback_runtime().info.status,
            crate::audio::PlaybackStatus::Playing
        );

        if let Some(engine_cell) = &self.ui.lyrics.engine {
            let mut engine = engine_cell.borrow_mut();

            engine.update(delta_secs);

            if !defer_layout_until_transition_finishes {
                engine.set_viewport_info(viewport_height, self.ui.lyrics.viewport_width);
            }

            if is_playing {
                engine.resume();
            } else {
                engine.pause();
            }

            engine.set_current_time(time_ms, &engine_lines, just_initialized);
        }
    }

    /// Handle user scroll event on lyrics
    pub fn handle_lyrics_scroll(&mut self, delta: f32) {
        tracing::debug!("Lyrics scroll: delta={}", delta);

        if let Some(engine_cell) = &self.ui.lyrics.engine {
            engine_cell.borrow_mut().handle_wheel(delta);
        }
    }

    // ============ ASYNC LOADING METHODS ============

    fn online_lyrics_metadata(
        &self,
        song_id: i64,
    ) -> crate::features::lyrics::OnlineLyricsMetadata {
        self.playback
            .current_song
            .as_ref()
            .filter(|song| song.id == song_id)
            .or_else(|| self.playback.queue.iter().find(|song| song.id == song_id))
            .map(|song| crate::features::lyrics::OnlineLyricsMetadata {
                title: song.title.clone(),
                artist: song.artist.clone(),
                album: song.album.clone(),
                duration_ms: u64::try_from(song.duration_secs)
                    .unwrap_or_default()
                    .saturating_mul(1_000),
            })
            .unwrap_or_default()
    }

    /// 异步加载歌词（本地、缓存或在线）
    /// 歌词加载的主入口
    pub fn load_lyrics_async(&mut self, song: &crate::database::DbSong) -> Task<Message> {
        tracing::info!(
            "load_lyrics_async called for song: {} (id={})",
            song.title,
            song.id
        );
        self.prepare_display_lyrics_load(song.id);
        self.playback
            .preload_coordinator
            .ensure_lyrics_slot(song.id);

        let song_id = song.id;
        let file_path = song.file_path.clone();
        let is_ncm = song.id < 0;
        let ncm_id = if is_ncm { (-song.id) as u64 } else { 0 };

        // Also start background color extraction
        let bg_task = self.update_background_async(song);

        // Create async task for lyrics loading
        // CRITICAL: Use spawn_blocking for synchronous I/O operations
        let lyrics_task = Task::perform(
            async move {
                // Use spawn_blocking to move sync I/O to blocking thread pool
                tokio::task::spawn_blocking(move || {
                    // Priority 1: Local lyrics file or embedded
                    if !file_path.is_empty() {
                        let audio_path = std::path::Path::new(&file_path);
                        if let Some(lrc_lines) =
                            crate::features::media::lyrics::find_lyrics(audio_path)
                        {
                            let ui_lines = crate::application::lyrics::project_lyrics(lrc_lines);
                            return Some((song_id, ui_lines, false)); // false = no online fetch needed
                        }
                    }

                    // Priority 2: Cached online lyrics (for NCM songs)
                    if is_ncm {
                        if let Some(cached_lines) =
                            crate::features::lyrics::load_cached_best_lyrics(ncm_id)
                        {
                            let ui_lines = crate::application::lyrics::project_lyrics(cached_lines);
                            return Some((song_id, ui_lines, false));
                        }
                        // Need online fetch
                        return Some((song_id, Vec::new(), true)); // true = need online fetch
                    }

                    // No lyrics found for local song
                    Some((song_id, Vec::new(), false))
                })
                .await
                .ok()
                .flatten()
            },
            |result| match result {
                Some((song_id, lines, needs_online)) => {
                    if needs_online {
                        let ncm_id = (-song_id) as u64;
                        Message::FetchLyricsOnline(song_id, ncm_id)
                    } else if !lines.is_empty() {
                        Message::LocalLyricsReady(song_id, lines)
                    } else {
                        Message::LyricsLoadFailed(
                            song_id,
                            crate::error::AppError::new(
                                crate::error::ErrorCode::MediaReadFailed,
                                "No lyrics were found",
                            ),
                        )
                    }
                }
                None => Message::Noop,
            },
        );

        Task::batch([bg_task, lyrics_task])
    }

    /// Update background asynchronously (color extraction + texture)
    fn update_background_async(&mut self, song: &crate::database::DbSong) -> Task<Message> {
        let song_id = song.id;

        // Reset shader time if needed
        if self.ui.lyrics.shader_start_time.is_none() {
            self.ui.lyrics.shader_start_time = Some(std::time::Instant::now());
        }

        let Some(path) = self.resolved_song_cover_local_path(song) else {
            self.ui.lyrics.textured_bg_shader.clear_cover();
            let colors = crate::utils::DominantColors::dark_default();
            self.ui.lyrics.bg_shader.set_colors(
                crate::ui::effects::background::color_to_array(colors.primary),
                crate::ui::effects::background::color_to_array(colors.secondary),
                crate::ui::effects::background::color_to_array(colors.tertiary),
            );
            self.ui.lyrics.bg_colors = colors;
            return Task::none();
        };
        let path = path.to_string_lossy().to_string();

        self.playback
            .preload_coordinator
            .ensure_background_slot(song_id, Some(path.clone()));

        // Fast path: coordinator has cached data for this song + cover — install directly
        if self
            .playback
            .preload_coordinator
            .is_background_ready(song_id, Some(path.as_str()))
        {
            self.install_background_from_coordinator(song_id);
            return Task::none();
        }

        // A GPU texture cache hit does not imply that the coordinator has
        // colors and texture data for this cover (e.g. Thumbnail -> Hero ->
        // Thumbnail). Only the readiness check above can skip preparation.
        self.prepare_lyrics_background_for_cover_path(song_id, std::path::PathBuf::from(path))
    }

    /// 只更新歌词页面背景（封面下载完成后调用）
    /// 不重新加载歌词
    pub fn update_lyrics_background(&mut self, song: &crate::database::DbSong) -> Task<Message> {
        self.update_background_async(song)
    }
}
