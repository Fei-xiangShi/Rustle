use std::path::PathBuf;
use std::sync::Arc;

use iced::Task;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::app::App;
use crate::download::DownloadStatus;
use crate::download::task;
use crate::metadata::SongMetadata;

impl App {
    pub fn handle_download(
        &mut self,
        message: &crate::app::Message,
    ) -> Option<Task<crate::app::Message>> {
        use crate::app::Message;
        match message {
            Message::DownloadSong(song_id) => self.download_single(*song_id),
            Message::DownloadUrlResolved(song_id, ncm_id, url, meta) => {
                self.download_enqueue(*song_id, *ncm_id, url.clone(), meta.clone())
            }
            Message::DownloadPlaylist(playlist_id) => self.download_playlist(*playlist_id),
            Message::DownloadBatchEnqueue(items) => {
                let quality = self.core.settings.storage.download_quality;
                let download_dir = self.core.settings.storage.effective_download_dir();
                let songs = items
                    .iter()
                    .map(|(song_id, ncm_id, url, meta)| {
                        (*song_id, *ncm_id, url.clone(), meta.clone())
                    })
                    .collect();
                let count =
                    self.core
                        .download_manager
                        .enqueue_playlist(songs, quality, download_dir);
                Some(Task::batch([
                    self.download_schedule_next().unwrap_or(Task::none()),
                    Self::toast_info(format!("已添加 {} 首到下载队列", count)),
                ]))
            }
            Message::DownloadCancel(song_id) => {
                self.core.download_manager.cancel(*song_id);
                Some(Task::none())
            }
            Message::DownloadProgress(song_id, downloaded, total) => {
                if let Some(task) = self
                    .core
                    .download_manager
                    .active
                    .iter_mut()
                    .find(|t| t.song_id == *song_id)
                {
                    let progress = if *total > 0 {
                        *downloaded as f32 / *total as f32
                    } else {
                        0.0
                    };
                    let mb = *downloaded as f64 / 1_048_576.0;
                    let speed = format!("{:.1} MB/s", mb);
                    task.status = DownloadStatus::Active { progress, speed };
                    if *total > 0 {
                        task.file_size = *total;
                    }
                }
                Some(Task::none())
            }
            Message::DownloadCompleted(song_id, path_str) => {
                let path_buf = PathBuf::from(path_str.as_str());
                let actual_size = path_buf.metadata().map(|m| m.len()).unwrap_or(0);
                if let Some(task) = self
                    .core
                    .download_manager
                    .active
                    .iter_mut()
                    .find(|t| t.song_id == *song_id)
                {
                    task.file_size = actual_size;
                }
                let ncm_id = if *song_id < 0 { (-song_id) as u64 } else { 0 };
                let track_title = self
                    .core
                    .download_manager
                    .active
                    .iter()
                    .find(|t| t.song_id == *song_id)
                    .map(|t| format!("{} - {}", t.metadata.artist, t.metadata.title))
                    .unwrap_or_default();

                let ncm_id_for_db = ncm_id;
                let db_task = if ncm_id > 0 {
                    if let Some(ref db) = self.core.db {
                        let db = Arc::clone(db);
                        let old_path = format!("ncm://{}", ncm_id);
                        let new_path = path_buf.to_string_lossy().to_string();
                        let new_path_for_cover = path_buf.clone();
                        let title = track_title.clone();
                        let artist = self
                            .core
                            .download_manager
                            .active
                            .iter()
                            .find(|t| t.song_id == *song_id)
                            .map(|t| t.metadata.artist.clone())
                            .unwrap_or_default();
                        let quality_str =
                            format!("{:?}", self.core.settings.storage.download_quality);
                        let file_size = path_buf.metadata().map(|m| m.len()).unwrap_or(0);
                        let song_id_val = *song_id;
                        Task::perform(
                            async move {
                                // Extract cover from downloaded file into cache
                                let cover_path =
                                    crate::features::import::extract_metadata(&new_path_for_cover)
                                        .ok()
                                        .and_then(|m| m.cover_data)
                                        .map(|data| {
                                            let dir = crate::utils::covers_cache_dir();
                                            let _ = std::fs::create_dir_all(&dir);
                                            let path =
                                                dir.join(format!("song_{}.jpg", song_id_val));
                                            let _ = std::fs::write(&path, &data);
                                            path.to_string_lossy().to_string()
                                        });
                                let _ = db.update_song_path(&old_path, &new_path).await;
                                if let Some(ref cp) = cover_path {
                                    let _ = db.update_song_cover(song_id_val, cp).await;
                                }
                                let _ = db
                                    .insert_download(crate::database::NewDownload {
                                        song_id: song_id_val,
                                        ncm_id: ncm_id_for_db,
                                        title: &title,
                                        artist: &artist,
                                        file_path: &new_path,
                                        file_size,
                                        quality: &quality_str,
                                    })
                                    .await;
                            },
                            |_| crate::app::Message::Noop,
                        )
                    } else {
                        Task::none()
                    }
                } else {
                    Task::none()
                };

                self.core
                    .download_manager
                    .complete(*song_id, path_buf.clone());
                let new_path = path_buf.to_string_lossy().to_string();
                if let Some(ref mut playlist) = self.ui.playlist_page.current {
                    for item in &mut playlist.songs {
                        item.source = crate::utils::compute_source(
                            if item.id == *song_id { &new_path } else { "" },
                            item.id,
                            Some(&item.artist),
                            Some(&item.title),
                        );
                    }
                }
                Some(Task::batch([
                    db_task,
                    self.download_schedule_next().unwrap_or(Task::none()),
                    Self::toast_success(track_title),
                ]))
            }
            Message::DownloadError(song_id, error) => {
                warn!("Download failed for song {}: {}", song_id, error);
                if *song_id != 0 {
                    self.core.download_manager.fail(*song_id, error.clone());
                }
                Some(Task::batch([
                    self.download_schedule_next().unwrap_or(Task::none()),
                    Self::toast_error(error.user_summary().to_owned()),
                ]))
            }
            Message::SwitchDownloadTab(tab) => {
                self.ui.download_tab = *tab;
                Some(Task::none())
            }
            Message::DeleteDownloadHistory(song_id) => {
                let song_id = *song_id;
                // Remove from in-memory completed list
                self.core
                    .download_manager
                    .completed
                    .retain(|t| t.song_id != song_id);
                // Delete from DB
                let task = if let Some(ref db) = self.core.db {
                    let db = Arc::clone(db);
                    Some(Task::perform(
                        async move {
                            let _ = db.delete_download(song_id).await;
                        },
                        |_| crate::app::Message::Noop,
                    ))
                } else {
                    None
                };
                Some(Task::batch([
                    task.unwrap_or(Task::none()),
                    Self::toast_info("已删除下载记录".to_string()),
                ]))
            }
            Message::DownloadsLoaded(rows) => {
                self.core.download_manager.restore_from_rows(rows.clone());
                Some(Task::none())
            }
            _ => None,
        }
    }

    fn download_single(&mut self, song_id: i64) -> Option<Task<crate::app::Message>> {
        let ncm_id = if song_id < 0 {
            (-song_id) as u64
        } else {
            return None;
        };
        let song_info = self
            .ui
            .home
            .current_ncm_playlist_songs
            .iter()
            .find(|s| s.id == ncm_id)
            .cloned()
            .or_else(|| {
                self.ui
                    .search
                    .tracks
                    .iter()
                    .find(|s| s.id == ncm_id)
                    .cloned()
            });

        let info = song_info?;
        let Some(client) = self.core.ncm_client.clone() else {
            return Some(Self::toast_error("未登录网易云账号"));
        };

        // Convert to unified metadata once — no manual field extraction
        let meta = SongMetadata::from(&info);
        let quality = crate::api::NcmQualityLevel::from_api_rate(
            self.core.settings.storage.download_quality.to_api_rate(),
        )
        .expect("download quality setting must map to a canonical NCM level");

        info!(
            "Fetching download URL for: {} - {} (ncm_id={})",
            meta.artist, meta.title, ncm_id
        );

        let url_task = Task::perform(
            async move {
                match client.resolve_track_url(ncm_id, quality).await {
                    Ok(url) if !url.url.is_empty() => url.url,
                    Err(e) => {
                        tracing::error!("Failed to get song URL for {}: {}", ncm_id, e);
                        String::new()
                    }
                    _ => String::new(),
                }
            },
            move |url| {
                if url.is_empty() {
                    crate::app::Message::DownloadError(
                        song_id,
                        crate::error::AppError::new(
                            crate::error::ErrorCode::AudioSourceUnavailable,
                            "No downloadable audio source is available",
                        ),
                    )
                } else {
                    crate::app::Message::DownloadUrlResolved(song_id, ncm_id, url, meta)
                }
            },
        );

        Some(url_task)
    }

    fn download_enqueue(
        &mut self,
        song_id: i64,
        ncm_id: u64,
        url: String,
        meta: SongMetadata,
    ) -> Option<Task<crate::app::Message>> {
        let quality = self.core.settings.storage.download_quality;
        let download_dir = self.core.settings.storage.effective_download_dir();
        let enqueued = self.core.download_manager.enqueue_song(
            song_id,
            ncm_id,
            url,
            quality,
            download_dir,
            meta.clone(),
        );
        if enqueued {
            info!(
                "Download enqueued: {} - {} (ncm_id={})",
                meta.artist, meta.title, ncm_id
            );
            Some(Task::batch([
                self.download_schedule_next().unwrap_or(Task::none()),
                Self::toast_info(format!("已加入下载: {} - {}", meta.artist, meta.title)),
            ]))
        } else {
            None
        }
    }

    fn download_playlist(&mut self, _playlist_id: i64) -> Option<Task<crate::app::Message>> {
        let tracks: Vec<&crate::api::Track> = if !self.ui.home.current_ncm_playlist_songs.is_empty()
        {
            self.ui.home.current_ncm_playlist_songs.iter().collect()
        } else {
            return Some(Self::toast_error("无可下载的歌曲"));
        };

        let client = self.core.ncm_client.clone()?;
        let quality = crate::api::NcmQualityLevel::from_api_rate(
            self.core.settings.storage.download_quality.to_api_rate(),
        )
        .expect("download quality setting must map to a canonical NCM level");
        let all_ids: Vec<u64> = tracks.iter().map(|s| s.id).collect();
        let song_data: Vec<(i64, u64, SongMetadata)> = tracks
            .iter()
            .map(|s| (-(s.id as i64), s.id, SongMetadata::from(*s)))
            .collect();

        Some(Task::perform(
            async move {
                match client.resolve_track_urls(&all_ids, quality).await {
                    Ok(urls) => {
                        let url_map: Vec<(u64, String)> = urls
                            .into_iter()
                            .filter(|u| !u.url.is_empty())
                            .map(|u| (u.id, u.url))
                            .collect();
                        (song_data, url_map)
                    }
                    Err(e) => {
                        tracing::error!("Failed to get playlist URLs: {}", e);
                        (song_data, Vec::new())
                    }
                }
            },
            move |(data, url_map)| {
                let items: Vec<(i64, u64, String, SongMetadata)> = data
                    .into_iter()
                    .filter_map(|(sid, nid, meta)| {
                        url_map
                            .iter()
                            .find(|(id, _)| *id == nid)
                            .map(|(_, url)| (sid, nid, url.clone(), meta))
                    })
                    .collect();
                if items.is_empty() {
                    crate::app::Message::DownloadError(
                        0,
                        crate::error::AppError::new(
                            crate::error::ErrorCode::AudioSourceUnavailable,
                            "No downloadable audio sources are available",
                        ),
                    )
                } else {
                    crate::app::Message::DownloadBatchEnqueue(items)
                }
            },
        ))
    }

    fn download_schedule_next(&mut self) -> Option<Task<crate::app::Message>> {
        if let Some(task) = self.core.download_manager.schedule() {
            let download_dir = task.download_dir.clone();
            let ncm_id = task.ncm_id;
            let song_id = task.song_id;
            let song_url = task.song_url.clone();
            let meta = task.metadata.clone();

            let (tx, mut rx) = mpsc::unbounded_channel();
            let (download_task, handle) = Task::perform(
                async move {
                    task::download_song(
                        ncm_id,
                        &song_url,
                        &download_dir,
                        &meta,
                        |downloaded, total| {
                            let _ = tx.send(crate::app::Message::DownloadProgress(
                                song_id, downloaded, total,
                            ));
                        },
                    )
                    .await
                },
                move |result| match result {
                    Ok(path) => crate::app::Message::DownloadCompleted(
                        song_id,
                        path.to_string_lossy().to_string(),
                    ),
                    Err(e) => crate::app::Message::DownloadError(song_id, e.into()),
                },
            )
            .abortable();
            self.core
                .download_manager
                .abort_handles
                .insert(song_id, handle);

            let progress_stream = Task::run(
                async_stream::stream! { while let Some(msg) = rx.recv().await { yield msg; } },
                |msg| msg,
            );
            Some(Task::batch([download_task, progress_stream]))
        } else {
            None
        }
    }
}
