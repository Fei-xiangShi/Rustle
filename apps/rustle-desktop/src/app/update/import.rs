// src/app/update/import.rs
//! Import message handlers

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use iced::Task;

use crate::app::helpers::{
    create_playlist_from_import, load_playlist_view, load_playlists, load_songs,
    load_watched_folders, sync_playlist_from_import,
};
use crate::app::message::Message;
use crate::app::state::App;
use crate::database::{Database, NewSong, NewWatchedFolder};
use crate::features::import::{
    CoverCache, FolderWatcher, ScanConfig, ScanHandle, ScanProgress, ScanResult, ScanState,
    WatchEvent, is_audio_file, progress_channel, scan_and_import, scan_audio_file,
    spawn_debounced_processor, watch_channel,
};
use crate::ui::components::ImportingPlaylist;

fn build_new_song(path: &Path, scan_result: ScanResult) -> NewSong {
    let cover_path = scan_result
        .cover_path
        .as_ref()
        .map(|cover| cover.to_string_lossy().to_string());

    NewSong {
        file_path: path.to_string_lossy().to_string(),
        title: scan_result.metadata.title,
        artist: scan_result.metadata.artist,
        album: scan_result.metadata.album,
        duration_secs: scan_result.metadata.duration_secs,
        track_number: scan_result.metadata.track_number,
        year: scan_result.metadata.year,
        genre: scan_result.metadata.genre,
        cover_path,
        file_hash: scan_result.file_hash,
        file_size: scan_result.file_size as i64,
        format: Some(scan_result.metadata.format),
        normalization_gain: scan_result.normalization_gain,
    }
}

async fn sync_watched_file(
    db: Arc<Database>,
    cache: Arc<CoverCache>,
    playlist_id: i64,
    path: PathBuf,
) -> Result<Option<i64>> {
    if !path.exists() || !is_audio_file(&path) {
        return Ok(None);
    }

    let scan_path = path.clone();
    let scan_result = tokio::task::spawn_blocking(move || {
        scan_audio_file(&scan_path, &ScanConfig::default(), Some(cache.as_ref()))
    })
    .await
    .context("Failed to join file scan task")??;

    let song_id = db
        .upsert_local_song(build_new_song(&path, scan_result))
        .await?;
    db.add_song_to_playlist(playlist_id, song_id).await?;
    let _ = db.touch_watched_folder_scan(playlist_id).await;
    Ok(Some(playlist_id))
}

async fn delete_watched_file(
    db: Arc<Database>,
    playlist_id: Option<i64>,
    path: PathBuf,
) -> Result<Option<i64>> {
    db.mark_song_missing_by_path(&path.to_string_lossy())
        .await?;
    if let Some(playlist_id) = playlist_id {
        let _ = db.touch_watched_folder_scan(playlist_id).await;
    }
    Ok(playlist_id)
}

async fn rename_watched_file(
    db: Arc<Database>,
    cache: Arc<CoverCache>,
    old_playlist_id: Option<i64>,
    new_playlist_id: Option<i64>,
    old_path: PathBuf,
    new_path: PathBuf,
) -> Result<Option<i64>> {
    let old_path_str = old_path.to_string_lossy().to_string();
    let new_path_str = new_path.to_string_lossy().to_string();

    match (old_playlist_id, new_playlist_id) {
        (Some(old_id), Some(new_id)) if old_id == new_id => {
            db.update_song_path(&old_path_str, &new_path_str).await?;
            let _ = db.touch_watched_folder_scan(old_id).await;
            Ok(Some(old_id))
        }
        (Some(old_id), Some(new_id)) => {
            if let Some(song) = db.get_song_by_path(&old_path_str).await? {
                db.update_song_path(&old_path_str, &new_path_str).await?;
                let _ = db.remove_song_from_playlist(old_id, song.id).await;
                db.add_song_to_playlist(new_id, song.id).await?;
                let _ = db.touch_watched_folder_scan(old_id).await;
                let _ = db.touch_watched_folder_scan(new_id).await;
                Ok(Some(new_id))
            } else {
                sync_watched_file(db.clone(), cache, new_id, new_path).await?;
                let _ = db.touch_watched_folder_scan(old_id).await;
                Ok(Some(new_id))
            }
        }
        (Some(old_id), None) => {
            db.mark_song_missing_by_path(&old_path_str).await?;
            let _ = db.touch_watched_folder_scan(old_id).await;
            Ok(Some(old_id))
        }
        (None, Some(new_id)) => sync_watched_file(db, cache, new_id, new_path).await,
        (None, None) => Ok(None),
    }
}

impl App {
    /// Handle import-related messages
    pub fn handle_import(&mut self, message: &Message) -> Option<Task<Message>> {
        match message {
            Message::FolderSelected(path) => {
                self.ui.dialogs.import_open = false;
                if let Some(path) = path {
                    return Some(Task::done(Message::StartScan(path.clone())));
                }
                Some(Task::none())
            }

            Message::StartScan(path) => Some(self.start_scan(path.clone())),

            Message::CancelScan => {
                if let Some(handle) = &self.library.scan_handle {
                    handle.cancel();
                    if let Some(playlist) = &mut self.ui.importing_playlist {
                        playlist.begin_cancelling();
                    }
                }
                Some(Task::none())
            }

            Message::ScanProgressUpdate(progress) => {
                Some(self.process_scan_progress(progress.clone()))
            }

            Message::ImportedPlaylistCreated(result) => match result {
                Ok(playlist_id) => {
                    if let Some(playlist) = &mut self.ui.importing_playlist {
                        playlist.playlist_id = Some(*playlist_id);
                    }

                    if let Some(db) = &self.core.db {
                        let db_for_playlists = db.clone();
                        let db_for_watched = db.clone();
                        return Some(Task::batch([
                            Task::perform(
                                load_playlists(db_for_playlists),
                                Message::PlaylistsLoaded,
                            ),
                            Task::perform(
                                load_watched_folders(db_for_watched),
                                Message::WatchedFoldersLoaded,
                            ),
                        ]));
                    }
                    Some(Task::none())
                }
                Err(err) => Some(Self::toast_error(format!("创建本地媒体库失败：{}", err))),
            },

            Message::WatchedFoldersLoaded(folders) => {
                self.library.watched_folders = folders.clone();
                Some(self.sync_folder_watcher())
            }

            Message::WatcherEvent(event) => Some(self.handle_watcher_event(event.clone())),

            Message::WatchedFolderSyncCompleted(playlist_id) => {
                if let Some(db) = &self.core.db {
                    let db_for_songs = db.clone();
                    let mut tasks = vec![Task::perform(
                        load_songs(db_for_songs),
                        Message::SongsLoaded,
                    )];

                    if let Some(playlist_id) = playlist_id
                        && self
                            .ui
                            .playlist_page
                            .current
                            .as_ref()
                            .map(|playlist| playlist.id)
                            == Some(*playlist_id)
                    {
                        let db_for_playlist = db.clone();
                        let playlist_id = *playlist_id;
                        tasks.push(Task::perform(
                            load_playlist_view(db_for_playlist, playlist_id),
                            |result| match result {
                                Some(payload) => Message::PlaylistViewLoaded(payload),
                                None => Message::DatabaseError(crate::error::AppError::new(
                                    crate::error::ErrorCode::StorageNotFound,
                                    "The playlist was not found",
                                )),
                            },
                        ));
                    }

                    return Some(Task::batch(tasks));
                }
                Some(Task::none())
            }

            Message::CoverCacheReady(cache) => {
                tracing::info!("Cover cache initialized");
                self.core.cover_cache = Some(cache.clone());
                Some(self.sync_folder_watcher())
            }

            Message::ClearImportingPlaylist => {
                self.ui.importing_playlist = None;
                Some(Task::none())
            }

            _ => None,
        }
    }

    /// Start a folder scan for importing music
    fn start_scan(&mut self, path: PathBuf) -> Task<Message> {
        if let (Some(db), Some(cache)) = (&self.core.db, &self.core.cover_cache) {
            let root_path = path.canonicalize().unwrap_or(path);
            let folder_name = root_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("导入的歌单")
                .to_string();
            self.ui.importing_playlist =
                Some(ImportingPlaylist::new(folder_name, root_path.clone()));

            let db = db.clone();
            let cache = cache.clone();
            let state = Arc::new(ScanState::new());
            let handle = ScanHandle::new(state.clone());

            self.library.scan_state = Some(state.clone());
            self.library.scan_handle = Some(handle);

            let (tx, mut rx) = progress_channel();
            let config = ScanConfig::default();
            let progress_tx = tx.clone();

            tokio::spawn(async move {
                if let Err(e) = scan_and_import(db, root_path, config, cache, state, tx).await {
                    tracing::error!("Scan error: {}", e);
                    let _ = progress_tx.send(ScanProgress::Error(e.to_string()));
                }
            });

            return Task::run(
                async_stream::stream! {
                    while let Some(progress) = rx.recv().await {
                        yield progress;
                    }
                },
                Message::ScanProgressUpdate,
            );
        }
        Task::none()
    }

    fn matching_watched_playlist(&self, path: &Path) -> Option<i64> {
        self.library
            .watched_folders
            .iter()
            .filter(|folder| folder.enabled)
            .filter_map(|folder| {
                let playlist_id = folder.playlist_id?;
                let root = Path::new(&folder.path);
                if path.starts_with(root) {
                    Some((folder.path.len(), playlist_id))
                } else {
                    None
                }
            })
            .max_by_key(|(path_len, _)| *path_len)
            .map(|(_, playlist_id)| playlist_id)
    }

    fn start_folder_watcher_service(&mut self) -> Result<Task<Message>> {
        let (raw_tx, raw_rx) = watch_channel();
        let (debounced_tx, mut debounced_rx) = watch_channel();

        tokio::spawn(async move {
            spawn_debounced_processor(raw_rx, 750, debounced_tx).await;
        });

        self.library.folder_watcher = Some(FolderWatcher::new(raw_tx)?);

        Ok(Task::run(
            async_stream::stream! {
                while let Some(event) = debounced_rx.recv().await {
                    yield event;
                }
            },
            Message::WatcherEvent,
        ))
    }

    fn sync_folder_watcher(&mut self) -> Task<Message> {
        if self.core.db.is_none() || self.core.cover_cache.is_none() {
            return Task::none();
        }

        let desired_paths: HashSet<PathBuf> = self
            .library
            .watched_folders
            .iter()
            .filter(|folder| folder.enabled)
            .filter_map(|folder| {
                let path = PathBuf::from(&folder.path);
                match path.canonicalize() {
                    Ok(path) => Some(path),
                    Err(err) => {
                        tracing::warn!(
                            "Skipping watched folder {} because it is unavailable: {}",
                            folder.path,
                            err
                        );
                        None
                    }
                }
            })
            .collect();

        let mut tasks = Vec::new();
        if self.library.folder_watcher.is_none() {
            if desired_paths.is_empty() {
                return Task::none();
            }

            match self.start_folder_watcher_service() {
                Ok(task) => tasks.push(task),
                Err(err) => {
                    tracing::error!("Failed to start folder watcher service: {}", err);
                    return Self::toast_warning("本地媒体库监听启动失败".to_string());
                }
            }
        }

        if let Some(watcher) = &mut self.library.folder_watcher {
            let current_paths: HashSet<PathBuf> = watcher.watched_paths().into_iter().collect();

            for path in desired_paths.difference(&current_paths) {
                if let Err(err) = watcher.watch(path) {
                    tracing::error!("Failed to watch folder {}: {}", path.display(), err);
                }
            }

            for path in current_paths.difference(&desired_paths) {
                if let Err(err) = watcher.unwatch(path) {
                    tracing::error!("Failed to unwatch folder {}: {}", path.display(), err);
                }
            }
        }

        if tasks.is_empty() {
            Task::none()
        } else {
            Task::batch(tasks)
        }
    }

    fn handle_watcher_event(&mut self, event: WatchEvent) -> Task<Message> {
        let Some(db) = self.core.db.clone() else {
            return Task::none();
        };

        match event {
            WatchEvent::FileCreated(path) | WatchEvent::FileModified(path) => {
                let Some(playlist_id) = self.matching_watched_playlist(&path) else {
                    return Task::none();
                };
                let Some(cache) = self.core.cover_cache.clone() else {
                    return Task::none();
                };

                Task::perform(
                    async move {
                        match sync_watched_file(db, cache, playlist_id, path).await {
                            Ok(playlist_id) => playlist_id,
                            Err(err) => {
                                tracing::error!("Failed to sync watched file: {}", err);
                                None
                            }
                        }
                    },
                    Message::WatchedFolderSyncCompleted,
                )
            }
            WatchEvent::FileDeleted(path) => {
                let playlist_id = self.matching_watched_playlist(&path);
                Task::perform(
                    async move {
                        match delete_watched_file(db, playlist_id, path).await {
                            Ok(playlist_id) => playlist_id,
                            Err(err) => {
                                tracing::error!("Failed to remove deleted watched file: {}", err);
                                None
                            }
                        }
                    },
                    Message::WatchedFolderSyncCompleted,
                )
            }
            WatchEvent::FileRenamed(old_path, new_path) => {
                let old_playlist_id = self.matching_watched_playlist(&old_path);
                let new_playlist_id = self.matching_watched_playlist(&new_path);
                if old_playlist_id.is_none() && new_playlist_id.is_none() {
                    return Task::none();
                }

                let Some(cache) = self.core.cover_cache.clone() else {
                    return Task::none();
                };

                self.update_renamed_paths_in_memory(&old_path, &new_path);

                Task::perform(
                    async move {
                        match rename_watched_file(
                            db,
                            cache,
                            old_playlist_id,
                            new_playlist_id,
                            old_path,
                            new_path,
                        )
                        .await
                        {
                            Ok(playlist_id) => playlist_id,
                            Err(err) => {
                                tracing::error!("Failed to handle watched rename: {}", err);
                                None
                            }
                        }
                    },
                    Message::WatchedFolderSyncCompleted,
                )
            }
            WatchEvent::Error(err) => Self::toast_warning(format!("本地媒体库监听出错：{}", err)),
        }
    }

    fn update_renamed_paths_in_memory(&mut self, old_path: &Path, new_path: &Path) {
        let old_path_str = old_path.to_string_lossy().to_string();
        let new_path_str = new_path.to_string_lossy().to_string();

        if let Some(song) = &mut self.playback.current_song
            && song.file_path == old_path_str
        {
            song.file_path = new_path_str.clone();
        }

        for song in &mut self.library.db_songs {
            if song.file_path == old_path_str {
                song.file_path = new_path_str.clone();
            }
        }

        for song in &mut self.playback.queue {
            if song.file_path == old_path_str {
                song.file_path = new_path_str.clone();
            }
        }
    }

    /// Process scan progress updates
    fn process_scan_progress(&mut self, progress: ScanProgress) -> Task<Message> {
        match &progress {
            ScanProgress::Started { total_files } => {
                tracing::info!("Scan started: {} files", total_files);
                if let Some(playlist) = &mut self.ui.importing_playlist {
                    playlist.total = *total_files;
                    playlist.set_status("扫描中...");
                }
            }
            ScanProgress::Processing {
                current,
                total,
                file_name,
            } => {
                if let Some(playlist) = &mut self.ui.importing_playlist {
                    playlist.current = *current;
                    playlist.total = *total;
                    playlist.set_status(format!("正在处理 {}", file_name));
                }
            }
            ScanProgress::Imported {
                current,
                total,
                title,
                artist,
                cover_path,
            } => {
                tracing::debug!("Imported ({}/{}): {} - {}", current, total, artist, title);
                if let Some(playlist) = &mut self.ui.importing_playlist {
                    playlist.update_progress(*current, *total);
                    playlist.set_status(format!("{}/{}", current, total));
                    if let Some(cover) = cover_path {
                        playlist.set_cover(cover.clone());
                    }
                }
            }
            ScanProgress::Skipped {
                current,
                total,
                file_name,
                reason,
            } => {
                if let Some(playlist) = &mut self.ui.importing_playlist {
                    playlist.update_progress(*current, *total);
                    playlist.record_skip(file_name, &reason.to_string());
                    if matches!(
                        reason,
                        crate::features::import::SkipReason::DatabaseError(_)
                    ) {
                        playlist.record_error();
                    }
                    playlist.set_status(format!("跳过 {}：{}", file_name, reason));
                }
                tracing::debug!("Skipped file {}: {}", file_name, reason);
            }
            ScanProgress::Error(err) => {
                tracing::error!("Scan error: {}", err);
                self.library.scan_state = None;
                self.library.scan_handle = None;
                if let Some(playlist) = &mut self.ui.importing_playlist {
                    playlist.set_status("导入失败");
                }
                return Self::toast_error(format!("导入失败：{}", err));
            }
            ScanProgress::Completed {
                imported,
                skipped,
                errors,
                duration_secs,
            } => {
                tracing::info!(
                    "Scan completed: {} imported, {} skipped, {} errors in {:.2}s",
                    imported,
                    skipped,
                    errors,
                    duration_secs
                );

                let scanned_paths = self
                    .library
                    .scan_state
                    .as_ref()
                    .and_then(|state| state.get_scanned_paths())
                    .unwrap_or_default();

                self.library.scan_state = None;
                self.library.scan_handle = None;

                let is_success = *imported > 0;
                let total_processed = *imported + *skipped + *errors;
                let (toast_task, clear_delay_secs) = if total_processed == 0 {
                    self.ui.importing_playlist = None;
                    (
                        Self::toast_error("导入失败：未找到任何音频文件".to_string()),
                        None,
                    )
                } else if *errors == 0 && *skipped == 0 {
                    (
                        Self::toast_success(format!("导入完成！成功导入 {} 首歌曲", imported)),
                        Some(4),
                    )
                } else if *errors == 0 {
                    (
                        Self::toast_warning(format!(
                            "导入完成：{} 首成功，{} 个跳过",
                            imported, skipped
                        )),
                        Some(6),
                    )
                } else {
                    (
                        Self::toast_warning(format!(
                            "导入完成：{} 首成功，{} 个跳过，{} 个错误",
                            imported, skipped, errors
                        )),
                        Some(6),
                    )
                };

                if let Some(playlist) = &mut self.ui.importing_playlist
                    && total_processed > 0
                {
                    playlist.complete(*imported, *skipped, *errors);
                }

                if is_success
                    && let (Some(db), Some(playlist)) = (&self.core.db, &self.ui.importing_playlist)
                {
                    let db_for_library = db.clone();
                    let db_for_songs = db.clone();
                    let name = playlist.name.clone();
                    let cover_path = playlist.cover_path.clone();
                    let root_path = playlist.root_path.clone();

                    let mut tasks = vec![
                        toast_task,
                        Task::perform(
                            async move {
                                let create_result = async {
                                    let watched_path =
                                        root_path.canonicalize().unwrap_or(root_path);
                                    let watched_path_str =
                                        watched_path.to_string_lossy().to_string();
                                    let playlist_id = if let Some(existing) = db_for_library
                                        .get_watched_folder_by_path(&watched_path_str)
                                        .await?
                                        .and_then(|folder| folder.playlist_id)
                                    {
                                        sync_playlist_from_import(
                                            db_for_library.clone(),
                                            existing,
                                            name,
                                            cover_path,
                                            scanned_paths,
                                        )
                                        .await?
                                    } else {
                                        create_playlist_from_import(
                                            db_for_library.clone(),
                                            name,
                                            cover_path,
                                            scanned_paths,
                                        )
                                        .await?
                                    };
                                    db_for_library
                                        .upsert_watched_folder(NewWatchedFolder {
                                            path: watched_path_str,
                                            playlist_id: Some(playlist_id),
                                            enabled: true,
                                        })
                                        .await?;
                                    crate::database::StorageResult::Ok(playlist_id)
                                }
                                .await;

                                create_result.map_err(crate::error::AppError::from)
                            },
                            Message::ImportedPlaylistCreated,
                        ),
                        Task::perform(load_songs(db_for_songs), Message::SongsLoaded),
                    ];

                    if let Some(secs) = clear_delay_secs {
                        tasks.push(Task::perform(
                            async move {
                                tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                            },
                            |_| Message::ClearImportingPlaylist,
                        ));
                    }

                    return Task::batch(tasks);
                }

                if let Some(secs) = clear_delay_secs {
                    return Task::batch(vec![
                        toast_task,
                        Task::perform(
                            async move {
                                tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                            },
                            |_| Message::ClearImportingPlaylist,
                        ),
                    ]);
                }

                return toast_task;
            }
            ScanProgress::Cancelled => {
                tracing::info!("Scan cancelled");
                self.library.scan_state = None;
                self.library.scan_handle = None;
                self.ui.importing_playlist = None;

                return Self::toast_warning("导入已取消".to_string());
            }
        }

        self.library.scan_progress = Some(progress);
        Task::none()
    }
}
