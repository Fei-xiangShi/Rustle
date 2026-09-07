use sqlx::{Pool, Sqlite};

use crate::database::{DownloadRow, NewDownload};

pub async fn insert_download(
    pool: &Pool<Sqlite>,
    download: NewDownload<'_>,
) -> Result<(), sqlx::Error> {
    let now = super::current_timestamp();
    sqlx::query(
        "INSERT INTO downloads (song_id, ncm_id, title, artist, file_path, file_size, quality, downloaded_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(download.song_id)
    .bind(download.ncm_id as i64)
    .bind(download.title)
    .bind(download.artist)
    .bind(download.file_path)
    .bind(download.file_size as i64)
    .bind(download.quality)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_all_downloads(pool: &Pool<Sqlite>) -> Result<Vec<DownloadRow>, sqlx::Error> {
    sqlx::query_as::<_, DownloadRow>(
        "SELECT song_id, ncm_id, title, artist, file_path, file_size, quality, downloaded_at FROM downloads ORDER BY downloaded_at DESC",
    )
    .fetch_all(pool)
    .await
}

pub async fn delete_download(pool: &Pool<Sqlite>, song_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM downloads WHERE song_id = ?")
        .bind(song_id)
        .execute(pool)
        .await?;
    Ok(())
}
