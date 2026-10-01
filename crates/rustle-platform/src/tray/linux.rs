//! Linux system tray implementation using ksni (freedesktop StatusNotifierItem)

use super::{TrayCommand, TrayError, TrayHandle, TrayPresentation, TrayWindowCommand};
use ksni::{Icon, MenuItem, Status, ToolTip, Tray as KsniTray, TrayMethods, menu::*};
use rustle_domain::playback::PlayMode;
use tokio::sync::mpsc;

/// Linux system tray implementation using ksni
pub struct LinuxTray {
    /// Channel to send commands to the application
    tx: mpsc::Sender<TrayCommand>,
    /// Current presentation projected by the application.
    presentation: TrayPresentation,
}

impl LinuxTray {
    /// Create a new tray instance
    pub fn new(tx: mpsc::Sender<TrayCommand>, presentation: TrayPresentation) -> Self {
        Self { tx, presentation }
    }

    /// Update the tray state
    pub fn update_state(&mut self, presentation: TrayPresentation) {
        self.presentation = presentation;
    }
}

impl KsniTray for LinuxTray {
    fn id(&self) -> String {
        "rustle-music".to_string()
    }

    fn title(&self) -> String {
        "Rustle".to_string()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::ApplicationStatus
    }

    fn status(&self) -> Status {
        Status::Active
    }

    fn icon_name(&self) -> String {
        String::new()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        create_icon()
    }

    fn tool_tip(&self) -> ToolTip {
        let title = "Rustle Music Player".to_string();
        let description = self.presentation.now_playing.clone();

        ToolTip {
            title,
            description,
            icon_name: String::new(),
            icon_pixmap: vec![],
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        create_menu(&self.presentation, &self.tx)
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self
            .tx
            .try_send(TrayCommand::Window(TrayWindowCommand::PrimaryActivation));
    }
}

pub async fn start_linux_tray(
    presentation: TrayPresentation,
    command_capacity: usize,
) -> Result<(TrayHandle, mpsc::Receiver<TrayCommand>), TrayError> {
    let (tx, rx) = mpsc::channel(command_capacity);
    let tray = LinuxTray::new(tx, presentation);

    let handle = tray
        .spawn()
        .await
        .map_err(|error| TrayError::backend("start Linux system tray", error))?;

    Ok((TrayHandle { handle }, rx))
}

fn create_icon() -> Vec<Icon> {
    static ICON_DATA: &[u8] = include_bytes!("../../../../assets/icons/icon_256.png");

    if let Ok(img) = image::load_from_memory(ICON_DATA) {
        let rgba = img
            .resize(32, 32, image::imageops::FilterType::Lanczos3)
            .to_rgba8();
        let (width, height) = rgba.dimensions();

        let mut data = Vec::with_capacity((width * height * 4) as usize);
        for pixel in rgba.pixels() {
            data.push(pixel[3]); // A
            data.push(pixel[0]); // R
            data.push(pixel[1]); // G
            data.push(pixel[2]); // B
        }

        return vec![Icon {
            width: width as i32,
            height: height as i32,
            data,
        }];
    }

    // Fallback icon
    let size = 32;
    let mut data = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - size as f32 / 2.0;
            let dy = y as f32 - size as f32 / 2.0;
            let dist = (dx * dx + dy * dy).sqrt();

            let idx = (y * size + x) * 4;
            if dist < size as f32 / 2.0 - 2.0 {
                data[idx] = 255; // A
                data[idx + 1] = 255; // R
                data[idx + 2] = 105; // G
                data[idx + 3] = 180; // B
            }
        }
    }

    vec![Icon {
        width: size as i32,
        height: size as i32,
        data,
    }]
}

fn create_menu(
    presentation: &TrayPresentation,
    _tx: &mpsc::Sender<TrayCommand>,
) -> Vec<MenuItem<LinuxTray>> {
    let play_icon = if presentation.is_playing {
        "media-playback-pause-symbolic"
    } else {
        "media-playback-start-symbolic"
    };

    let play_mode_index = match presentation.play_mode {
        PlayMode::Sequential => 0,
        PlayMode::LoopAll => 1,
        PlayMode::LoopOne => 2,
        PlayMode::Shuffle => 3,
    };

    vec![
        // Now playing info
        StandardItem {
            label: presentation.now_playing.clone(),
            enabled: false,
            ..Default::default()
        }
        .into(),
        MenuItem::Separator,
        // Playback controls
        StandardItem {
            label: presentation.play_pause.to_string(),
            icon_name: play_icon.to_string(),
            activate: Box::new(|tray: &mut LinuxTray| {
                let _ = tray.tx.try_send(TrayCommand::PlayPause);
            }),
            ..Default::default()
        }
        .into(),
        StandardItem {
            label: presentation.previous.to_string(),
            icon_name: "media-skip-backward-symbolic".to_string(),
            activate: Box::new(|tray: &mut LinuxTray| {
                let _ = tray.tx.try_send(TrayCommand::PrevTrack);
            }),
            ..Default::default()
        }
        .into(),
        StandardItem {
            label: presentation.next.to_string(),
            icon_name: "media-skip-forward-symbolic".to_string(),
            activate: Box::new(|tray: &mut LinuxTray| {
                let _ = tray.tx.try_send(TrayCommand::NextTrack);
            }),
            ..Default::default()
        }
        .into(),
        // Favorite button
        if presentation.favorite_enabled {
            let (fav_label, fav_icon) = if presentation.is_favorited {
                (presentation.favorite, "starred-symbolic")
            } else {
                (presentation.favorite, "non-starred-symbolic")
            };
            StandardItem {
                label: fav_label.to_string(),
                icon_name: fav_icon.to_string(),
                activate: Box::new(|tray: &mut LinuxTray| {
                    let _ = tray.tx.try_send(TrayCommand::ToggleFavorite);
                }),
                ..Default::default()
            }
            .into()
        } else {
            StandardItem {
                label: presentation.favorite.to_string(),
                icon_name: "non-starred-symbolic".to_string(),
                enabled: false,
                ..Default::default()
            }
            .into()
        },
        MenuItem::Separator,
        // Play mode submenu
        SubMenu {
            label: presentation.play_mode_label.to_string(),
            icon_name: "media-playlist-consecutive-symbolic".to_string(),
            submenu: vec![
                RadioGroup {
                    selected: play_mode_index,
                    select: Box::new(|tray: &mut LinuxTray, index| {
                        let mode = match index {
                            0 => PlayMode::Sequential,
                            1 => PlayMode::LoopAll,
                            2 => PlayMode::LoopOne,
                            3 => PlayMode::Shuffle,
                            _ => PlayMode::Sequential,
                        };
                        let _ = tray.tx.try_send(TrayCommand::SetPlayMode(mode));
                    }),
                    options: vec![
                        RadioItem {
                            label: presentation.sequential.to_string(),
                            icon_name: "media-playlist-consecutive-symbolic".to_string(),
                            ..Default::default()
                        },
                        RadioItem {
                            label: presentation.loop_all.to_string(),
                            icon_name: "media-playlist-repeat-symbolic".to_string(),
                            ..Default::default()
                        },
                        RadioItem {
                            label: presentation.loop_one.to_string(),
                            icon_name: "media-playlist-repeat-song-symbolic".to_string(),
                            ..Default::default()
                        },
                        RadioItem {
                            label: presentation.shuffle.to_string(),
                            icon_name: "media-playlist-shuffle-symbolic".to_string(),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }
                .into(),
            ],
            ..Default::default()
        }
        .into(),
        MenuItem::Separator,
        // Window control
        StandardItem {
            label: presentation.toggle_window.to_string(),
            icon_name: "view-restore-symbolic".to_string(),
            activate: Box::new(|tray: &mut LinuxTray| {
                let _ = tray
                    .tx
                    .try_send(TrayCommand::Window(TrayWindowCommand::Toggle));
            }),
            ..Default::default()
        }
        .into(),
        MenuItem::Separator,
        // Quit
        StandardItem {
            label: presentation.quit.to_string(),
            icon_name: "application-exit-symbolic".to_string(),
            activate: Box::new(|tray: &mut LinuxTray| {
                let _ = tray.tx.try_send(TrayCommand::Quit);
            }),
            ..Default::default()
        }
        .into(),
    ]
}
