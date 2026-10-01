//! Individual artist links shared by playback surfaces.

use iced::widget::{container, mouse_area, text};
use iced::{Element, mouse};

use crate::api::ArtistSummary;
use crate::app::Message;
use crate::ui::{theme, widgets};

pub fn items(
    artist_text: &str,
    structured_artists: &[ArtistSummary],
    text_size: f32,
    line_height: Option<f32>,
) -> Vec<Element<'static, Message>> {
    let metadata_text = |content: String| {
        let label = text(content).size(text_size);
        if let Some(height) = line_height {
            label
                .line_height(text::LineHeight::Relative(1.0))
                .height(height)
                .align_y(iced::alignment::Vertical::Center)
                .wrapping(text::Wrapping::None)
        } else {
            label
        }
    };
    let mut items = Vec::new();
    for (index, link) in artist_links(artist_text, structured_artists)
        .into_iter()
        .enumerate()
    {
        if index > 0 {
            items.push(
                metadata_text(" / ".to_string())
                    .style(|theme| text::Style {
                        color: Some(theme::text_secondary(theme)),
                    })
                    .into(),
            );
        }
        let artist = mouse_area(metadata_text(link.name))
            .on_press(artist_target_message(link.target))
            .interaction(mouse::Interaction::Pointer);
        items.push(
            widgets::hover_surface(artist)
                .style(|theme, progress| container::Style {
                    text_color: Some(theme::lerp_color(
                        theme::text_secondary(theme),
                        theme::text_primary(theme),
                        progress,
                    )),
                    ..Default::default()
                })
                .into(),
        );
    }
    items
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ArtistTarget {
    Id(u64),
    Name(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ArtistLink {
    name: String,
    target: ArtistTarget,
}

fn artist_links(artist_text: &str, structured_artists: &[ArtistSummary]) -> Vec<ArtistLink> {
    let mut links = Vec::new();

    if !structured_artists.is_empty() {
        for artist in structured_artists {
            let name = artist.name.trim();
            if name.is_empty() {
                continue;
            }

            links.push(ArtistLink {
                name: name.to_string(),
                target: if artist.id == 0 {
                    ArtistTarget::Name(name.to_string())
                } else {
                    ArtistTarget::Id(artist.id)
                },
            });
        }
    } else {
        for name in artist_text
            .split('/')
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            if links.iter().any(|link: &ArtistLink| link.name == name) {
                continue;
            }

            links.push(ArtistLink {
                name: name.to_string(),
                target: ArtistTarget::Name(name.to_string()),
            });
        }
    }

    if links.is_empty() {
        let name = artist_text.trim();
        if !name.is_empty() {
            links.push(ArtistLink {
                name: name.to_string(),
                target: ArtistTarget::Name(name.to_string()),
            });
        }
    }

    links
}

fn artist_target_message(target: ArtistTarget) -> Message {
    match target {
        ArtistTarget::Id(id) => Message::OpenArtist(id),
        ArtistTarget::Name(name) => Message::OpenArtistByName(name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_artists_keep_individual_ids() {
        let artists = vec![
            ArtistSummary {
                id: 12,
                name: "Artist A".to_string(),
                image_url: String::new(),
            },
            ArtistSummary {
                id: 34,
                name: "Artist B".to_string(),
                image_url: String::new(),
            },
        ];

        assert_eq!(
            artist_links("Artist A / Artist B", &artists),
            vec![
                ArtistLink {
                    name: "Artist A".to_string(),
                    target: ArtistTarget::Id(12),
                },
                ArtistLink {
                    name: "Artist B".to_string(),
                    target: ArtistTarget::Id(34),
                },
            ]
        );
    }

    #[test]
    fn structured_artists_with_the_same_name_keep_distinct_ids() {
        let artists = vec![
            ArtistSummary {
                id: 12,
                name: "Shared Name".to_string(),
                image_url: String::new(),
            },
            ArtistSummary {
                id: 34,
                name: "Shared Name".to_string(),
                image_url: String::new(),
            },
        ];

        assert_eq!(
            artist_links("Shared Name / Shared Name", &artists),
            vec![
                ArtistLink {
                    name: "Shared Name".to_string(),
                    target: ArtistTarget::Id(12),
                },
                ArtistLink {
                    name: "Shared Name".to_string(),
                    target: ArtistTarget::Id(34),
                },
            ]
        );
    }

    #[test]
    fn string_artists_are_split_trimmed_and_deduplicated() {
        assert_eq!(
            artist_links(" Artist A / Artist B / Artist A ", &[]),
            vec![
                ArtistLink {
                    name: "Artist A".to_string(),
                    target: ArtistTarget::Name("Artist A".to_string()),
                },
                ArtistLink {
                    name: "Artist B".to_string(),
                    target: ArtistTarget::Name("Artist B".to_string()),
                },
            ]
        );
    }

    #[test]
    fn artist_targets_preserve_id_and_name_navigation_messages() {
        assert!(matches!(
            artist_target_message(ArtistTarget::Id(12)),
            Message::OpenArtist(12)
        ));
        assert!(matches!(
            artist_target_message(ArtistTarget::Name("Artist A".to_string())),
            Message::OpenArtistByName(name) if name == "Artist A"
        ));
    }
}
