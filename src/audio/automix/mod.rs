//! Native, opt-in Automix planning primitives.
//!
//! This module intentionally contains deterministic data contracts and pure
//! planning/math. The audio thread remains the owner of playback execution;
//! analysis and scheduling identities are immutable snapshots that can be
//! discarded when their generation/group is stale.

use std::collections::VecDeque;
use std::fs;
use std::hash::Hasher;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::application::ports::cache::CachePublisher;
use rodio::Source;
use serde::{Deserialize, Serialize};

pub const ANALYSIS_SCHEMA_VERSION: u32 = 3;
pub const ANALYZER_VERSION: &str = "rustle-native-v3";
pub const DEFAULT_ANALYSIS_MAX_SECONDS: u32 = 60;
pub const BASELINE_CROSSFADE_MS: u32 = 5_000;
pub const MANUAL_CROSSFADE_MS: u32 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisConfig {
    pub max_seconds: u32,
    pub schema_version: u32,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            max_seconds: DEFAULT_ANALYSIS_MAX_SECONDS,
            schema_version: ANALYSIS_SCHEMA_VERSION,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnergyPoint {
    pub at: Duration,
    pub rms_db: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VocalRegion {
    pub start: Duration,
    pub end: Duration,
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackAnalysis {
    pub schema_version: u32,
    pub analyzer_version: String,
    pub content_id: String,
    pub duration: Duration,
    pub bpm: Option<f32>,
    pub bpm_confidence: f32,
    pub first_beat: Option<Duration>,
    pub beat_period: Option<Duration>,
    pub bar_beats: u8,
    pub lufs: Option<f32>,
    pub energy: f32,
    pub energy_profile: Vec<EnergyPoint>,
    pub vocals_confidence: f32,
    pub vocal_regions: Vec<VocalRegion>,
    pub vocal_out: Option<Duration>,
    pub outro_energy_db: Option<f32>,
    pub key: Option<String>,
    pub cut_start: Duration,
    pub cut_out: Duration,
    pub fade_out_start: Duration,
    pub recommended_exit: Duration,
    pub recommended_entry: Duration,
    pub transition_confidence: f32,
}

impl TrackAnalysis {
    pub fn is_compatible(&self, config: AnalysisConfig) -> bool {
        self.schema_version == config.schema_version
            && self.analyzer_version == ANALYZER_VERSION
            && !self.content_id.is_empty()
            && self.duration > Duration::ZERO
            && self.bpm_confidence.is_finite()
            && (0.0..=1.0).contains(&self.bpm_confidence)
            && self
                .bpm
                .is_none_or(|value| value.is_finite() && value > 0.0)
            && self.lufs.is_none_or(f32::is_finite)
            && self.energy.is_finite()
            && (0.0..=1.0).contains(&self.energy)
            && self.vocals_confidence.is_finite()
            && (0.0..=1.0).contains(&self.vocals_confidence)
            && self.transition_confidence.is_finite()
            && (0.0..=1.0).contains(&self.transition_confidence)
            && self.cut_start <= self.cut_out
            && self.cut_out <= self.duration
            && self.fade_out_start <= self.cut_out
            && self.recommended_exit <= self.cut_out
            && self.recommended_entry <= self.duration
            && self.vocal_out.is_none_or(|value| value <= self.cut_out)
            && self.outro_energy_db.is_none_or(f32::is_finite)
            && self
                .energy_profile
                .iter()
                .all(|point| point.at <= self.duration && point.rms_db.is_finite())
            && self
                .energy_profile
                .windows(2)
                .all(|pair| pair[0].at <= pair[1].at)
            && self.vocal_regions.iter().all(|region| {
                region.start < region.end
                    && region.end <= self.duration
                    && region.confidence.is_finite()
                    && (0.0..=1.0).contains(&region.confidence)
            })
            && self
                .vocal_regions
                .windows(2)
                .all(|pair| pair[0].start <= pair[1].start)
    }
}

/// Deterministic FNV-1a cache key; avoids relying on randomized hash seeds.
pub fn cache_key(content_id: &str, config: AnalysisConfig) -> String {
    let mut hasher = Fnv64::default();
    hasher.write(content_id.as_bytes());
    hasher.write_u32(config.schema_version);
    hasher.write(ANALYZER_VERSION.as_bytes());
    hasher.write_u32(config.max_seconds);
    format!("{:016x}", hasher.finish())
}

pub fn content_identity(path: &Path, fallback: &str) -> String {
    match path.metadata() {
        Ok(metadata) => {
            let modified = metadata
                .modified()
                .ok()
                .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|value| value.as_nanos())
                .unwrap_or(0);
            format!(
                "file:{}:{}:{}",
                path.to_string_lossy(),
                metadata.len(),
                modified
            )
        }
        Err(_) => format!("track:{fallback}"),
    }
}

#[derive(Clone)]
pub struct AnalysisCache {
    root: PathBuf,
    max_entries: usize,
    publisher: Arc<dyn CachePublisher>,
}

impl std::fmt::Debug for AnalysisCache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AnalysisCache")
            .field("root", &self.root)
            .field("max_entries", &self.max_entries)
            .finish_non_exhaustive()
    }
}

struct CacheClaim(PathBuf);

impl Drop for CacheClaim {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

impl AnalysisCache {
    pub fn new(
        root: impl Into<PathBuf>,
        max_entries: usize,
        publisher: Arc<dyn CachePublisher>,
    ) -> Self {
        Self {
            root: root.into(),
            max_entries: max_entries.max(1),
            publisher,
        }
    }

    fn path_for(&self, content_id: &str, config: AnalysisConfig) -> PathBuf {
        self.root
            .join(format!("{}.json", cache_key(content_id, config)))
    }

    pub fn load(
        &self,
        content_id: &str,
        config: AnalysisConfig,
    ) -> Result<Option<TrackAnalysis>, String> {
        let path = self.path_for(content_id, config);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("read Automix cache {:?}: {error}", path)),
        };
        let analysis: TrackAnalysis = match serde_json::from_slice(&bytes) {
            Ok(analysis) => analysis,
            Err(_) => {
                let _ = fs::remove_file(&path);
                return Ok(None);
            }
        };
        if analysis.content_id != content_id || !analysis.is_compatible(config) {
            let _ = fs::remove_file(&path);
            return Ok(None);
        }
        Ok(Some(analysis))
    }

    pub fn store(&self, analysis: &TrackAnalysis, config: AnalysisConfig) -> Result<(), String> {
        if !analysis.is_compatible(config) {
            return Err("incompatible Automix analysis".to_string());
        }
        fs::create_dir_all(&self.root)
            .map_err(|error| format!("create Automix cache {:?}: {error}", self.root))?;
        let path = self.path_for(&analysis.content_id, config);
        let claim = path.with_extension("write.lock");
        let claim_file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&claim)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return match self.load(&analysis.content_id, config) {
                    Ok(Some(_)) => Ok(()),
                    Ok(None) => Err(format!(
                        "Automix cache write already in progress: {claim:?}"
                    )),
                    Err(load_error) => Err(load_error),
                };
            }
            Err(error) => return Err(format!("claim Automix cache write {:?}: {error}", claim)),
        };
        let _claim = CacheClaim(claim);
        drop(claim_file);

        if path.exists() {
            match self.load(&analysis.content_id, config)? {
                Some(_) => return Ok(()),
                None => {
                    // `load` removes corrupt or incompatible entries. The
                    // following rename therefore publishes into an absent
                    // destination instead of deleting a valid cache first.
                }
            }
        }

        let temp = self.publisher.unique_temp_path(&path);
        let bytes = serde_json::to_vec(analysis)
            .map_err(|error| format!("serialize Automix analysis: {error}"))?;
        let write_result = (|| {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
                .map_err(|error| format!("create Automix cache temp {:?}: {error}", temp))?;
            file.write_all(&bytes)
                .map_err(|error| format!("write Automix cache {:?}: {error}", temp))?;
            file.sync_all()
                .map_err(|error| format!("sync Automix cache {:?}: {error}", temp))?;
            drop(file);
            self.publisher
                .publish_or_reuse(&temp, &path, None)
                .map_err(|error| format!("publish Automix cache {:?}: {error}", path))?;
            Ok::<(), String>(())
        })();
        if write_result.is_err() {
            self.publisher.cleanup_temp_file(&temp);
        }
        write_result?;
        self.prune()?;
        Ok(())
    }

    pub fn analyze_file_if_missing<F>(
        &self,
        path: &Path,
        content_id: &str,
        config: AnalysisConfig,
        is_cancelled: F,
    ) -> Result<bool, String>
    where
        F: Fn() -> bool,
    {
        if is_cancelled() {
            return Ok(false);
        }
        if self.load(content_id, config)?.is_some() {
            return Ok(false);
        }
        fs::create_dir_all(&self.root)
            .map_err(|error| format!("create Automix cache {:?}: {error}", self.root))?;
        let claim = self
            .path_for(content_id, config)
            .with_extension("analysis.lock");
        let claim_file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&claim)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
            Err(error) => return Err(format!("claim Automix analysis {:?}: {error}", claim)),
        };
        let result = analyze_file_cancellable(path, content_id.to_string(), config, &is_cancelled)
            .and_then(|analysis| {
                if is_cancelled() {
                    Ok(false)
                } else {
                    self.store(&analysis, config).map(|_| true)
                }
            });
        drop(claim_file);
        let _ = fs::remove_file(claim);
        if is_cancelled() { Ok(false) } else { result }
    }

    fn prune(&self) -> Result<(), String> {
        let mut entries: Vec<_> = fs::read_dir(&self.root)
            .map_err(|error| format!("scan Automix cache {:?}: {error}", self.root))?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .collect();
        entries.sort_by(|left, right| {
            modified(left.path().as_path()).cmp(&modified(right.path().as_path()))
        });
        let remove_count = entries.len().saturating_sub(self.max_entries);
        for entry in entries.into_iter().take(remove_count) {
            let _ = fs::remove_file(entry.path());
        }
        Ok(())
    }
}

fn modified(path: &Path) -> std::time::SystemTime {
    path.metadata()
        .and_then(|metadata| metadata.modified())
        .unwrap_or(std::time::UNIX_EPOCH)
}

#[derive(Default)]
struct Fnv64(u64);

impl Hasher for Fnv64 {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        if self.0 == 0 {
            self.0 = 0xcbf29ce484222325;
        }
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
}

impl Fnv64 {
    fn write_u32(&mut self, value: u32) {
        self.write(&value.to_le_bytes());
    }
}

const PROFILE_FRAME_MS: u64 = 100;

#[derive(Default)]
struct TimelineFeatures {
    energy_profile: Vec<EnergyPoint>,
    vocal_regions: Vec<VocalRegion>,
    vocals_confidence: f32,
    cut_start: Duration,
    cut_out: Duration,
}

fn mix_to_mono(samples: &[f32], channels: u16) -> Vec<f32> {
    let channel_count = channels.max(1) as usize;
    samples
        .chunks(channel_count)
        .map(|frame| frame.iter().copied().sum::<f32>() / frame.len().max(1) as f32)
        .collect()
}

fn timeline_features(
    mono: &[f32],
    sample_rate: u32,
    track_energy: f32,
    offset: Duration,
    duration: Duration,
) -> TimelineFeatures {
    if mono.is_empty() || sample_rate == 0 {
        return TimelineFeatures {
            cut_out: duration,
            ..TimelineFeatures::default()
        };
    }

    let frame_samples = ((u64::from(sample_rate) * PROFILE_FRAME_MS) / 1_000).max(1) as usize;
    let audible_threshold = (track_energy * 0.12).max(0.002);
    let low_alpha = 1.0 - (-std::f32::consts::TAU * 300.0 / sample_rate as f32).exp();
    let high_alpha = 1.0 - (-std::f32::consts::TAU * 3_400.0 / sample_rate as f32).exp();
    let mut low_300 = 0.0f32;
    let mut low_3400 = 0.0f32;
    let mut audible_frames = Vec::new();
    let mut vocal_candidates = Vec::new();
    let mut energy_profile = Vec::new();

    for (index, frame) in mono.chunks(frame_samples).enumerate() {
        let mut total_power = 0.0f32;
        let mut voice_power = 0.0f32;
        let mut crossings = 0usize;
        let mut previous = frame.first().copied().unwrap_or(0.0);
        for &sample in frame {
            low_300 += low_alpha * (sample - low_300);
            low_3400 += high_alpha * (sample - low_3400);
            let voice_band = low_3400 - low_300;
            total_power += sample * sample;
            voice_power += voice_band * voice_band;
            crossings += usize::from((previous >= 0.0) != (sample >= 0.0));
            previous = sample;
        }
        let mean_square = total_power / frame.len().max(1) as f32;
        let rms = mean_square.sqrt();
        let start = offset.saturating_add(Duration::from_millis(index as u64 * PROFILE_FRAME_MS));
        let end = start
            .saturating_add(Duration::from_millis(PROFILE_FRAME_MS))
            .min(duration);
        energy_profile.push(EnergyPoint {
            at: start.min(duration),
            rms_db: 20.0 * rms.max(1.0e-6).log10(),
        });
        if rms >= audible_threshold {
            audible_frames.push((start, end));
        }

        let voice_ratio = voice_power / total_power.max(1.0e-12);
        let crossing_rate = crossings as f32 / frame.len().max(1) as f32;
        let band_score = ((voice_ratio - 0.08) / 0.55).clamp(0.0, 1.0);
        let crossing_score = (1.0 - (crossing_rate - 0.08).abs() / 0.12).clamp(0.0, 1.0);
        let loudness_score = (rms / track_energy.max(0.01)).clamp(0.0, 1.0);
        let confidence =
            (band_score * 0.55 + crossing_score * 0.25 + loudness_score * 0.20).clamp(0.0, 1.0);
        if rms >= audible_threshold && confidence >= 0.52 {
            vocal_candidates.push((start, end, confidence));
        }
    }

    let mut vocal_regions = Vec::new();
    let mut open: Option<(Duration, Duration, f32, u32)> = None;
    for (start, end, confidence) in vocal_candidates {
        match open.as_mut() {
            Some((_, region_end, confidence_sum, count))
                if start <= region_end.saturating_add(Duration::from_millis(300)) =>
            {
                *region_end = end;
                *confidence_sum += confidence;
                *count += 1;
            }
            _ => {
                if let Some((region_start, region_end, confidence_sum, count)) = open.take()
                    && region_end.saturating_sub(region_start) >= Duration::from_millis(400)
                {
                    vocal_regions.push(VocalRegion {
                        start: region_start,
                        end: region_end,
                        confidence: (confidence_sum / count.max(1) as f32).clamp(0.0, 1.0),
                    });
                }
                open = Some((start, end, confidence, 1));
            }
        }
    }
    if let Some((region_start, region_end, confidence_sum, count)) = open
        && region_end.saturating_sub(region_start) >= Duration::from_millis(400)
    {
        vocal_regions.push(VocalRegion {
            start: region_start,
            end: region_end,
            confidence: (confidence_sum / count.max(1) as f32).clamp(0.0, 1.0),
        });
    }

    let vocals_confidence = vocal_regions
        .iter()
        .map(|region| region.confidence)
        .fold(0.0f32, f32::max);
    let cut_start = audible_frames
        .first()
        .map(|(start, _)| *start)
        .unwrap_or(offset.min(duration));
    let cut_out = audible_frames
        .last()
        .map(|(_, end)| *end)
        .unwrap_or(duration)
        .min(duration);
    TimelineFeatures {
        energy_profile,
        vocal_regions,
        vocals_confidence,
        cut_start,
        cut_out,
    }
}

fn profile_energy_after(profile: &[EnergyPoint], start: Duration) -> Option<f32> {
    let mut count = 0usize;
    let power = profile
        .iter()
        .filter(|point| point.at >= start)
        .map(|point| {
            count += 1;
            10.0_f32.powf(point.rms_db / 10.0)
        })
        .sum::<f32>();
    (count > 0).then(|| 10.0 * (power / count as f32).max(1.0e-12).log10())
}

fn transition_confidence(
    bpm_confidence: f32,
    timeline: &TimelineFeatures,
    duration: Duration,
) -> f32 {
    let profile_score = if timeline.energy_profile.is_empty() {
        0.0
    } else {
        1.0
    };
    let bounds_score = if timeline.cut_start < timeline.cut_out && timeline.cut_out <= duration {
        1.0
    } else {
        0.0
    };
    (bpm_confidence.clamp(0.0, 1.0) * 0.45
        + timeline.vocals_confidence.clamp(0.0, 1.0) * 0.20
        + profile_score * 0.20
        + bounds_score * 0.15)
        .clamp(0.0, 1.0)
}

/// Analyze a bounded in-memory sample window without touching the playback Sink.
pub fn analyze_samples(
    content_id: impl Into<String>,
    samples: &[f32],
    sample_rate: u32,
    channels: u16,
    duration: Duration,
    config: AnalysisConfig,
) -> TrackAnalysis {
    let max_samples = usize::try_from(config.max_seconds)
        .unwrap_or(usize::MAX)
        .saturating_mul(sample_rate as usize)
        .saturating_mul(channels.max(1) as usize);
    let window = &samples[..samples.len().min(max_samples)];
    let mono = mix_to_mono(window, channels);
    let mean_square = if mono.is_empty() {
        0.0
    } else {
        mono.iter().map(|sample| sample * sample).sum::<f32>() / mono.len() as f32
    };
    let energy = mean_square.sqrt().clamp(0.0, 1.0);
    let frame_samples = (sample_rate as usize / 50).max(1);
    let frame_energy: Vec<f32> = mono
        .chunks(frame_samples)
        .map(|frame| {
            (frame.iter().map(|sample| sample * sample).sum::<f32>() / frame.len().max(1) as f32)
                .sqrt()
        })
        .collect();
    let onset: Vec<f32> = frame_energy
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).max(0.0))
        .collect();
    let (bpm, bpm_confidence, first_beat) = estimate_tempo(&onset);
    let beat_period = bpm.map(|value| Duration::from_secs_f32(60.0 / value));
    let timeline = timeline_features(&mono, sample_rate, energy, Duration::ZERO, duration);
    let vocal_out = timeline.vocal_regions.last().map(|region| region.end);
    let fade_out_start = timeline.cut_out.saturating_sub(Duration::from_secs(4));
    let exit = fade_out_start
        .max(vocal_out.unwrap_or(Duration::ZERO))
        .min(duration.saturating_sub(Duration::from_millis(100)));
    let outro_energy_db =
        vocal_out.and_then(|out| profile_energy_after(&timeline.energy_profile, out));
    let confidence = transition_confidence(bpm_confidence, &timeline, duration);
    let TimelineFeatures {
        energy_profile,
        vocal_regions,
        vocals_confidence,
        cut_start,
        cut_out,
    } = timeline;
    TrackAnalysis {
        schema_version: config.schema_version,
        analyzer_version: ANALYZER_VERSION.to_string(),
        content_id: content_id.into(),
        duration,
        bpm,
        bpm_confidence,
        first_beat,
        beat_period,
        bar_beats: 4,
        lufs: if mean_square > 0.0 {
            Some(-0.691 + 10.0 * mean_square.log10())
        } else {
            None
        },
        energy,
        energy_profile,
        vocals_confidence,
        vocal_regions,
        vocal_out,
        outro_energy_db,
        key: estimate_key(&mono, sample_rate, energy),
        cut_start,
        cut_out,
        fade_out_start,
        recommended_exit: exit,
        recommended_entry: cut_start,
        transition_confidence: confidence,
    }
}

fn estimate_tempo(onset: &[f32]) -> (Option<f32>, f32, Option<Duration>) {
    if onset.len() < 40 || onset.iter().copied().sum::<f32>() <= f32::EPSILON {
        return (None, 0.0, None);
    }
    let mut best_lag = 0usize;
    let mut best_score = 0.0f32;
    let normalization = onset.iter().map(|value| value * value).sum::<f32>();
    for lag in 17..=75 {
        if lag >= onset.len() {
            break;
        }
        let score = onset
            .iter()
            .skip(lag)
            .zip(onset.iter())
            .map(|(a, b)| a * b)
            .sum::<f32>();
        if score > best_score {
            best_score = score;
            best_lag = lag;
        }
    }
    if best_lag == 0 || best_score <= f32::EPSILON {
        return (None, 0.0, None);
    }
    let bpm = 60_000.0 / (best_lag as f32 * 20.0);
    let confidence = (best_score / normalization.max(best_score)).clamp(0.0, 1.0);
    let first = onset
        .iter()
        .take(best_lag)
        .enumerate()
        .max_by(|left, right| left.1.total_cmp(right.1))
        .map(|(index, _)| Duration::from_millis((index as u64 + 1) * 20));
    (Some(bpm.clamp(40.0, 240.0)), confidence, first)
}

fn estimate_key(samples: &[f32], sample_rate: u32, energy: f32) -> Option<String> {
    if sample_rate == 0 || samples.is_empty() || energy < 0.005 {
        return None;
    }
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let limit = samples.len().min(sample_rate as usize * 10);
    let stride = 4usize;
    let mut best = (0usize, 0.0f64);
    for pitch in 0..12 {
        let frequency = 130.812_782_65_f64 * 2.0_f64.powf(pitch as f64 / 12.0);
        let omega = std::f64::consts::TAU * frequency / sample_rate as f64;
        let mut re = 0.0f64;
        let mut im = 0.0f64;
        for (index, sample) in samples[..limit].iter().step_by(stride).enumerate() {
            let phase = omega * (index * stride) as f64;
            re += f64::from(*sample) * phase.cos();
            im -= f64::from(*sample) * phase.sin();
        }
        let magnitude = re * re + im * im;
        if magnitude > best.1 {
            best = (pitch, magnitude);
        }
    }
    (best.1 > 0.0).then(|| NAMES[best.0].to_string())
}

fn collect_samples<I, F>(source: &mut I, limit: usize, is_cancelled: &F) -> Result<Vec<f32>, String>
where
    I: Iterator<Item = f32>,
    F: Fn() -> bool,
{
    let mut samples = Vec::with_capacity(limit.min(1_048_576));
    for (index, sample) in source.take(limit).enumerate() {
        if index % 4_096 == 0 && is_cancelled() {
            return Err("Automix analysis cancelled".to_string());
        }
        samples.push(sample);
    }
    if is_cancelled() {
        return Err("Automix analysis cancelled".to_string());
    }
    Ok(samples)
}

fn analyze_file_cancellable<F>(
    path: &Path,
    content_id: impl Into<String>,
    config: AnalysisConfig,
    is_cancelled: &F,
) -> Result<TrackAnalysis, String>
where
    F: Fn() -> bool,
{
    if is_cancelled() {
        return Err("Automix analysis cancelled".to_string());
    }
    let file = fs::File::open(path).map_err(|error| format!("open {:?}: {error}", path))?;
    let byte_len = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
    let mut decoder = rodio::Decoder::builder()
        .with_data(BufReader::new(file))
        .with_byte_len(byte_len)
        .with_seekable(true)
        .build()
        .map_err(|error| format!("decode {:?}: {error}", path))?;
    let duration = decoder.total_duration().unwrap_or(Duration::ZERO);
    let sample_rate = decoder.sample_rate().get();
    let channels = decoder.channels().get();
    let max_samples = usize::try_from(config.max_seconds)
        .unwrap_or(usize::MAX)
        .saturating_mul(sample_rate as usize)
        .saturating_mul(channels.max(1) as usize);
    let split_samples = max_samples / 2;
    let mut head = collect_samples(&mut decoder, split_samples.max(1), is_cancelled)?;
    let mut tail = Vec::new();
    let analyzed_head_duration = Duration::from_secs_f64(
        head.len() as f64 / sample_rate.max(1) as f64 / channels.max(1) as f64,
    );
    if duration > analyzed_head_duration.saturating_mul(2) {
        let tail_duration = Duration::from_secs_f64(
            split_samples as f64 / sample_rate.max(1) as f64 / channels.max(1) as f64,
        );
        let tail_start = duration.saturating_sub(tail_duration);
        decoder
            .try_seek(tail_start)
            .map_err(|error| format!("seek Automix tail {:?}: {error}", path))?;
        tail = collect_samples(&mut decoder, split_samples.max(1), is_cancelled)?;
    } else {
        let remaining = max_samples.saturating_sub(head.len());
        head.extend(collect_samples(&mut decoder, remaining, is_cancelled)?);
    }
    let mut samples = Vec::with_capacity(head.len().saturating_add(tail.len()));
    samples.extend_from_slice(&head);
    samples.extend_from_slice(&tail);
    let content_id = content_id.into();
    let mut analysis = analyze_samples(
        content_id,
        &samples,
        sample_rate,
        channels,
        duration,
        config,
    );
    if !tail.is_empty() {
        let head_mono = mix_to_mono(&head, channels);
        let tail_mono = mix_to_mono(&tail, channels);
        let tail_frames = tail.len() / channels.max(1) as usize;
        let tail_duration = Duration::from_secs_f64(tail_frames as f64 / sample_rate.max(1) as f64);
        let tail_start = duration.saturating_sub(tail_duration);
        let head_timeline = timeline_features(
            &head_mono,
            sample_rate,
            analysis.energy,
            Duration::ZERO,
            duration,
        );
        let tail_timeline = timeline_features(
            &tail_mono,
            sample_rate,
            analysis.energy,
            tail_start,
            duration,
        );
        let mut combined_timeline = TimelineFeatures {
            cut_start: head_timeline.cut_start,
            cut_out: tail_timeline.cut_out,
            vocals_confidence: head_timeline
                .vocals_confidence
                .max(tail_timeline.vocals_confidence),
            energy_profile: head_timeline.energy_profile,
            vocal_regions: head_timeline.vocal_regions,
        };
        combined_timeline
            .energy_profile
            .extend(tail_timeline.energy_profile);
        combined_timeline
            .vocal_regions
            .extend(tail_timeline.vocal_regions);
        combined_timeline
            .energy_profile
            .sort_by_key(|point| point.at);
        combined_timeline
            .vocal_regions
            .sort_by_key(|region| region.start);
        analysis.vocal_out = combined_timeline
            .vocal_regions
            .last()
            .map(|region| region.end);
        analysis.outro_energy_db = analysis
            .vocal_out
            .and_then(|out| profile_energy_after(&combined_timeline.energy_profile, out));
        analysis.cut_start = combined_timeline.cut_start;
        analysis.cut_out = combined_timeline.cut_out;
        analysis.fade_out_start = analysis.cut_out.saturating_sub(Duration::from_secs(4));
        analysis.recommended_exit = analysis
            .fade_out_start
            .max(analysis.vocal_out.unwrap_or(Duration::ZERO))
            .min(duration.saturating_sub(Duration::from_millis(100)));
        analysis.recommended_entry = analysis.cut_start;
        analysis.transition_confidence =
            transition_confidence(analysis.bpm_confidence, &combined_timeline, duration);
        analysis.energy_profile = combined_timeline.energy_profile;
        analysis.vocals_confidence = combined_timeline.vocals_confidence;
        analysis.vocal_regions = combined_timeline.vocal_regions;
    }
    Ok(analysis)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionFallback {
    LowConfidence,
    InvalidBounds,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionKind {
    Manual,
    BaselineNatural,
    Automix,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransitionDirective {
    pub kind: TransitionKind,
    pub duration: Duration,
    pub entry: Duration,
    pub expected_exit: Option<Duration>,
    pub group: ScheduleGroup,
    pub automation: AdvancedAutomation,
}

impl TransitionDirective {
    pub fn manual(group: ScheduleGroup) -> Self {
        Self {
            kind: TransitionKind::Manual,
            duration: Duration::from_millis(MANUAL_CROSSFADE_MS.into()),
            entry: Duration::ZERO,
            expected_exit: None,
            group,
            automation: AdvancedAutomation::default(),
        }
    }

    pub fn baseline_natural(group: ScheduleGroup) -> Self {
        Self {
            kind: TransitionKind::BaselineNatural,
            duration: Duration::from_millis(BASELINE_CROSSFADE_MS.into()),
            entry: Duration::ZERO,
            expected_exit: None,
            group,
            automation: AdvancedAutomation::default(),
        }
    }

    pub fn automix(
        group: ScheduleGroup,
        plan: &TransitionPlan,
        automation: AdvancedAutomation,
    ) -> Self {
        Self {
            kind: TransitionKind::Automix,
            duration: plan.duration,
            entry: plan.entry,
            expected_exit: Some(plan.exit),
            group,
            automation,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransitionPlan {
    pub exit: Duration,
    pub entry: Duration,
    pub duration: Duration,
    pub beat_aligned: bool,
    pub bar_aligned: bool,
    pub aggressive_outro: bool,
    pub fallback: Option<TransitionFallback>,
}

pub fn plan_transition(
    current: &TrackAnalysis,
    next: &TrackAnalysis,
    default_duration: Duration,
) -> Result<TransitionPlan, TransitionFallback> {
    if current.duration.is_zero() || next.duration.is_zero() {
        return Err(TransitionFallback::InvalidBounds);
    }
    if current.bpm_confidence <= 0.4
        || next.bpm_confidence <= 0.4
        || current.transition_confidence < 0.5
        || next.transition_confidence < 0.5
    {
        return Err(TransitionFallback::LowConfidence);
    }
    let minimum_duration = Duration::from_millis(100);
    let mut duration = default_duration
        .min(current.duration)
        .min(next.duration)
        .max(minimum_duration);
    let mut exit = current
        .recommended_exit
        .min(current.duration.saturating_sub(minimum_duration));
    let mut entry = next
        .recommended_entry
        .min(next.duration.saturating_sub(minimum_duration));
    let mut beat_aligned = false;
    let mut bar_aligned = false;
    if let (Some(current_period), Some(current_first), Some(next_period), Some(next_first)) = (
        current.beat_period,
        current.first_beat,
        next.beat_period,
        next.first_beat,
    ) && current_period > Duration::ZERO
        && next_period > Duration::ZERO
    {
        let current_bar_period = current_period.saturating_mul(current.bar_beats.max(1) as u32);
        let full_duration_grid = || {
            Some((
                snap_to_grid_with_limit(
                    exit,
                    current_first,
                    current_bar_period,
                    current.duration.saturating_sub(duration),
                )?,
                snap_to_grid_with_limit(
                    entry,
                    next_first,
                    next_period,
                    next.duration.saturating_sub(duration),
                )?,
            ))
        };
        let contracted_grid = || {
            Some((
                snap_to_grid_with_limit(
                    exit,
                    current_first,
                    current_bar_period,
                    current.duration.saturating_sub(minimum_duration),
                )?,
                snap_to_grid_with_limit(
                    entry,
                    next_first,
                    next_period,
                    next.duration.saturating_sub(minimum_duration),
                )?,
            ))
        };
        if let Some((snapped_exit, snapped_entry)) = full_duration_grid().or_else(contracted_grid) {
            exit = snapped_exit;
            entry = snapped_entry;
            beat_aligned = true;
            bar_aligned = current.bar_beats > 1;
        }
    }
    duration = duration
        .min(current.duration.saturating_sub(exit))
        .min(next.duration.saturating_sub(entry));
    if duration < minimum_duration {
        return Err(TransitionFallback::InvalidBounds);
    }
    let aggressive_outro = apply_aggressive_outro(current, exit, duration);
    if let Some((new_exit, new_duration)) = aggressive_outro {
        exit = new_exit;
        duration = new_duration
            .min(current.duration.saturating_sub(exit))
            .min(next.duration.saturating_sub(entry));
    }
    if duration < minimum_duration {
        return Err(TransitionFallback::InvalidBounds);
    }
    Ok(TransitionPlan {
        exit,
        entry,
        duration,
        beat_aligned,
        bar_aligned,
        aggressive_outro: aggressive_outro.is_some(),
        fallback: None,
    })
}

fn snap_to_grid(value: Duration, first: Duration, period: Duration) -> Duration {
    if value <= first {
        return first;
    }
    let steps = value.saturating_sub(first).as_secs_f64() / period.as_secs_f64();
    first + period.mul_f64(steps.round().max(0.0))
}

fn snap_to_grid_with_limit(
    value: Duration,
    first: Duration,
    period: Duration,
    latest: Duration,
) -> Option<Duration> {
    if period.is_zero() || first > latest {
        return None;
    }
    let snapped = snap_to_grid(value, first, period);
    if snapped <= latest {
        return Some(snapped);
    }
    let steps = latest.saturating_sub(first).as_secs_f64() / period.as_secs_f64();
    Some(first + period.mul_f64(steps.floor().max(0.0)))
}

fn apply_aggressive_outro(
    analysis: &TrackAnalysis,
    current_exit: Duration,
    current_duration: Duration,
) -> Option<(Duration, Duration)> {
    if analysis.vocals_confidence < 0.55 {
        return None;
    }
    let vocal_out = analysis.vocal_out?;
    let tail_end = analysis.cut_out.min(analysis.duration);
    let tail_length = tail_end.saturating_sub(vocal_out);
    if tail_length <= Duration::from_secs(8) {
        return None;
    }

    let high_energy = analysis.outro_energy_db? > -12.0;
    let mut new_exit = if let (Some(first), Some(period)) =
        (analysis.first_beat, analysis.beat_period)
        && period > Duration::ZERO
    {
        let relative = vocal_out.saturating_sub(first).as_secs_f64();
        let period_seconds = period.as_secs_f64();
        let mut beat_index = (relative / period_seconds).floor().max(0.0) as u64;
        if relative % period_seconds > period_seconds * 0.9 {
            beat_index = beat_index.saturating_add(1);
        }
        beat_index = beat_index.saturating_add(if high_energy { 8 } else { 1 });
        if high_energy {
            let bar = u64::from(analysis.bar_beats.max(1));
            beat_index = beat_index.saturating_add(bar - 1) / bar * bar;
        }
        first.saturating_add(period.saturating_mul(beat_index.min(u32::MAX as u64) as u32))
    } else {
        vocal_out.saturating_add(if high_energy {
            Duration::from_secs(4)
        } else {
            Duration::from_millis(500)
        })
    };
    new_exit = new_exit.min(tail_end.saturating_sub(Duration::from_secs(1)));
    if new_exit >= current_exit {
        return None;
    }
    let max_fade = if high_energy {
        Duration::from_secs(5)
    } else {
        Duration::from_secs(3)
    };
    let duration = current_duration
        .min(max_fade)
        .min(tail_end.saturating_sub(new_exit));
    (duration >= Duration::from_millis(300)).then_some((new_exit, duration))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScheduleGroup(pub u64);

#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledAction {
    pub group: ScheduleGroup,
    pub at: Duration,
    pub underrun: bool,
}

#[derive(Debug, Default)]
pub struct AudioClockScheduler {
    horizon: Duration,
    actions: VecDeque<ScheduledAction>,
}

impl AudioClockScheduler {
    pub fn new(horizon: Duration) -> Self {
        Self {
            horizon,
            ..Self::default()
        }
    }

    pub fn schedule(&mut self, group: ScheduleGroup, at: Duration) {
        self.actions.retain(|action| action.group != group);
        self.actions.push_back(ScheduledAction {
            group,
            at,
            underrun: false,
        });
    }

    pub fn cancel(&mut self, group: ScheduleGroup) {
        self.actions.retain(|action| action.group != group);
    }

    pub fn poll(&mut self, now: Duration) -> Option<ScheduledAction> {
        let action = self.actions.front()?;
        if action.at > now {
            return None;
        }
        let mut action = self.actions.pop_front()?;
        action.underrun = now > action.at.saturating_add(self.horizon);
        Some(action)
    }
}

pub const SCHEDULER_HORIZON_MS: u64 = 1_500;
pub const SCHEDULER_POLL_MS: u64 = 75;
pub const AUTOMIX_PLANNING_WINDOW_SECS: u64 = 45;

pub fn loudness_gain_db(delta_db: f32) -> f32 {
    if delta_db.is_finite() {
        delta_db.clamp(-9.0, 9.0)
    } else {
        0.0
    }
}

fn linear_gain_db(gain: f32) -> f32 {
    if gain.is_finite() && gain > 0.0 {
        20.0 * gain.log10()
    } else {
        0.0
    }
}

/// Resolve the incoming per-track Automix loudness anchor in the effective
/// output domain, after both tracks' normalization gains are known.
pub fn effective_automix_gain_db(
    raw_lufs_delta_db: f32,
    outgoing_track_gain: f32,
    outgoing_automix_gain_db: f32,
    incoming_track_gain: f32,
) -> f32 {
    let outgoing_automix_gain_db = if outgoing_automix_gain_db.is_finite() {
        outgoing_automix_gain_db
    } else {
        0.0
    };

    loudness_gain_db(
        raw_lufs_delta_db + linear_gain_db(outgoing_track_gain) + outgoing_automix_gain_db
            - linear_gain_db(incoming_track_gain),
    )
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AdvancedAutomation {
    pub rate: f32,
    /// Raw analysis-domain LUFS delta. The player resolves this against both
    /// tracks' actual normalization gains and clamps the final DSP target.
    pub gain_db: f32,
    pub bass_swap: bool,
}

impl Default for AdvancedAutomation {
    fn default() -> Self {
        Self {
            rate: 1.0,
            gain_db: 0.0,
            bass_swap: false,
        }
    }
}

pub fn automation_for_transition(
    current: &TrackAnalysis,
    next: &TrackAnalysis,
) -> AdvancedAutomation {
    let rate = match (current.bpm, next.bpm) {
        (Some(a), Some(b))
            if a > 0.0
                && b > 0.0
                && current.bpm_confidence >= 0.4
                && next.bpm_confidence >= 0.4
                && (0.97..=1.03).contains(&(a / b)) =>
        {
            a / b
        }
        _ => 1.0,
    };
    let gain_db = match (current.lufs, next.lufs) {
        (Some(a), Some(b)) if a.is_finite() && b.is_finite() => a - b,
        _ => 0.0,
    };
    AdvancedAutomation {
        rate,
        gain_db,
        bass_swap: current.energy > 0.2
            && next.energy > 0.2
            && matches!((current.bpm, next.bpm), (Some(a), Some(b)) if a > 0.0 && ((a - b).abs() / a) < 0.06),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default)]
    struct TestPublisher;

    impl CachePublisher for TestPublisher {
        fn unique_temp_path(&self, final_path: &Path) -> PathBuf {
            static NEXT_TEMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let sequence = NEXT_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let name = final_path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("automix");
            final_path.with_file_name(format!(
                ".{name}.test-{}-{sequence}.part",
                std::process::id()
            ))
        }

        fn cleanup_temp_file(&self, path: &Path) {
            let _ = fs::remove_file(path);
        }

        fn publish_or_reuse(
            &self,
            temp_path: &Path,
            final_path: &Path,
            expected_size: Option<u64>,
        ) -> std::io::Result<crate::application::ports::cache::PublishOutcome> {
            let temp_size = fs::metadata(temp_path)?.len();
            if let Some(expected_size) = expected_size
                && expected_size != temp_size
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "test publication size mismatch",
                ));
            }
            if fs::metadata(final_path)
                .is_ok_and(|metadata| metadata.is_file() && metadata.len() == temp_size)
            {
                self.cleanup_temp_file(temp_path);
                return Ok(crate::application::ports::cache::PublishOutcome::Reused);
            }
            fs::rename(temp_path, final_path)?;
            Ok(crate::application::ports::cache::PublishOutcome::Published)
        }
    }

    fn test_publisher() -> Arc<dyn CachePublisher> {
        Arc::new(TestPublisher)
    }

    fn fallback_analysis(content_id: impl Into<String>, duration: Duration) -> TrackAnalysis {
        let exit = duration.saturating_sub(Duration::from_secs(5));
        TrackAnalysis {
            schema_version: ANALYSIS_SCHEMA_VERSION,
            analyzer_version: ANALYZER_VERSION.to_string(),
            content_id: content_id.into(),
            duration,
            bpm: None,
            bpm_confidence: 0.0,
            first_beat: None,
            beat_period: None,
            bar_beats: 4,
            lufs: None,
            energy: 0.0,
            energy_profile: Vec::new(),
            vocals_confidence: 0.0,
            vocal_regions: Vec::new(),
            vocal_out: None,
            outro_energy_db: None,
            key: None,
            cut_start: Duration::ZERO,
            cut_out: duration,
            fade_out_start: exit,
            recommended_exit: exit,
            recommended_entry: Duration::ZERO,
            transition_confidence: 0.0,
        }
    }

    #[test]
    fn cache_key_is_stable_and_versioned() {
        let config = AnalysisConfig::default();
        assert_eq!(cache_key("song-1", config), cache_key("song-1", config));
        assert_ne!(cache_key("song-1", config), cache_key("song-2", config));
    }

    #[test]
    fn cache_round_trip_and_corruption_are_non_fatal() {
        let root = std::env::temp_dir().join(format!(
            "rustle-automix-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let cache = AnalysisCache::new(&root, 2, test_publisher());
        let config = AnalysisConfig::default();
        let analysis = fallback_analysis("song", Duration::from_secs(30));
        cache.store(&analysis, config).unwrap();
        assert_eq!(cache.load("song", config).unwrap(), Some(analysis.clone()));

        let path = cache.path_for("song", config);
        fs::write(&path, b"not-json").unwrap();
        assert_eq!(cache.load("song", config).unwrap(), None);
        assert!(!path.exists());
        cache.store(&analysis, config).unwrap();
        assert_eq!(cache.load("song", config).unwrap(), Some(analysis));
        assert!(fs::read_dir(&root).unwrap().all(|entry| {
            let extension = entry
                .unwrap()
                .path()
                .extension()
                .map(|value| value.to_owned());
            extension.as_deref() == Some(std::ffi::OsStr::new("json"))
        }));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cache_invalidates_old_schema_and_preserves_a_competing_valid_publish() {
        let root = std::env::temp_dir().join(format!(
            "rustle-automix-version-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let cache = AnalysisCache::new(&root, 2, test_publisher());
        let config = AnalysisConfig::default();
        let analysis = fallback_analysis("song", Duration::from_secs(30));
        fs::create_dir_all(&root).unwrap();
        let path = cache.path_for("song", config);
        let mut old = analysis.clone();
        old.schema_version = config.schema_version.saturating_sub(1);
        fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        assert_eq!(cache.load("song", config).unwrap(), None);
        assert!(!path.exists());

        cache.store(&analysis, config).unwrap();
        let claim = path.with_extension("write.lock");
        fs::write(&claim, b"writer").unwrap();
        let mut competing = analysis.clone();
        competing.energy = 0.9;
        cache.store(&competing, config).unwrap();
        assert_eq!(cache.load("song", config).unwrap(), Some(analysis));
        let _ = fs::remove_file(claim);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn analysis_is_bounded_and_has_fallback_for_silence() {
        let config = AnalysisConfig {
            max_seconds: 1,
            ..AnalysisConfig::default()
        };
        let analysis = analyze_samples(
            "silence",
            &[0.0; 48_000 * 2],
            48_000,
            1,
            Duration::from_secs(10),
            config,
        );
        assert_eq!(analysis.energy, 0.0);
        assert_eq!(analysis.bpm_confidence, 0.0);
        assert!(!analysis.energy_profile.is_empty());
        assert!(analysis.vocal_regions.is_empty());
        assert!(analysis.is_compatible(config));
    }

    #[test]
    fn periodic_onsets_produce_usable_bpm_confidence() {
        let sample_rate = 1_000;
        let mut samples = vec![0.0; sample_rate as usize * 10];
        for beat in (0..samples.len()).step_by(500) {
            for sample in samples.iter_mut().skip(beat).take(20) {
                *sample = 0.8;
            }
        }
        let analysis = analyze_samples(
            "click",
            &samples,
            sample_rate,
            1,
            Duration::from_secs(10),
            AnalysisConfig::default(),
        );
        assert!(analysis.bpm.is_some_and(|bpm| (bpm - 120.0).abs() < 1.0));
        assert!(analysis.bpm_confidence >= 0.6);
    }

    #[test]
    fn timeline_features_keep_absolute_tail_positions_and_vocal_regions() {
        let sample_rate = 8_000;
        let mut samples = Vec::with_capacity(sample_rate as usize * 10);
        for index in 0..sample_rate as usize * 5 {
            let time = index as f32 / sample_rate as f32;
            samples.push((time * 500.0 * std::f32::consts::TAU).sin() * 0.5);
        }
        for index in 0..sample_rate as usize * 5 {
            let time = index as f32 / sample_rate as f32;
            samples.push((time * 100.0 * std::f32::consts::TAU).sin() * 0.15);
        }
        let timeline = timeline_features(
            &samples,
            sample_rate,
            0.3,
            Duration::from_secs(90),
            Duration::from_secs(100),
        );
        assert!(
            timeline
                .energy_profile
                .iter()
                .all(|point| point.at >= Duration::from_secs(90))
        );
        assert_eq!(timeline.cut_out, Duration::from_secs(100));
        assert!(
            timeline
                .vocal_regions
                .iter()
                .all(|region| region.start >= Duration::from_secs(90))
        );
        assert!(timeline.vocals_confidence >= 0.55);
    }

    #[test]
    fn bounded_collection_stops_cooperatively_when_cancelled() {
        let checks = std::cell::Cell::new(0usize);
        let result = collect_samples(&mut std::iter::repeat(0.1), 20_000, &|| {
            checks.set(checks.get() + 1);
            checks.get() > 1
        });
        assert_eq!(result.unwrap_err(), "Automix analysis cancelled");
        assert!(checks.get() >= 2);
    }

    #[test]
    fn planner_requires_confidence_and_bounds_duration() {
        let mut current = fallback_analysis("a", Duration::from_secs(2));
        let next = fallback_analysis("b", Duration::from_secs(3));
        assert_eq!(
            plan_transition(&current, &next, Duration::from_secs(5)),
            Err(TransitionFallback::LowConfidence)
        );
        current.bpm = Some(120.0);
        current.bpm_confidence = 0.9;
        current.transition_confidence = 0.9;
        current.beat_period = Some(Duration::from_millis(500));
        current.first_beat = Some(Duration::from_millis(100));
        let mut next = next;
        next.bpm = Some(120.0);
        next.bpm_confidence = 0.9;
        next.transition_confidence = 0.9;
        next.beat_period = Some(Duration::from_millis(500));
        next.first_beat = Some(Duration::from_millis(100));
        let plan = plan_transition(&current, &next, Duration::from_secs(5)).unwrap();
        assert!(plan.duration <= Duration::from_secs(2));
        assert!(plan.beat_aligned);
        assert!(plan.bar_aligned);
    }

    #[test]
    fn automation_rate_policy_and_final_loudness_gain_are_bounded() {
        assert_eq!(loudness_gain_db(-20.0), -9.0);
        let mut current = fallback_analysis("a", Duration::from_secs(30));
        let mut next = fallback_analysis("b", Duration::from_secs(30));
        current.bpm = Some(100.0);
        next.bpm = Some(140.0);
        current.bpm_confidence = 0.9;
        next.bpm_confidence = 0.9;
        assert_eq!(automation_for_transition(&current, &next).rate, 1.0);
        current.lufs = Some(-5.0);
        next.lufs = Some(-25.0);
        assert_eq!(automation_for_transition(&current, &next).gain_db, 20.0);
    }

    #[test]
    fn effective_automix_gain_accounts_for_normalization_and_prior_automix() {
        assert!((effective_automix_gain_db(6.0, 1.0, 0.0, 1.0) - 6.0).abs() < 1e-5);

        let outgoing_gain = 10.0_f32.powf(-2.0 / 20.0);
        let incoming_gain = 10.0_f32.powf(4.0 / 20.0);
        assert!(effective_automix_gain_db(6.0, outgoing_gain, 0.0, incoming_gain).abs() < 1e-4);

        let strongly_normalized_incoming = 10.0_f32.powf(12.0 / 20.0);
        assert!(
            (effective_automix_gain_db(20.0, 1.0, 0.0, strongly_normalized_incoming) - 8.0).abs()
                < 1e-4
        );

        assert!((effective_automix_gain_db(2.0, 1.0, 3.0, 1.0) - 5.0).abs() < 1e-5);
    }

    #[test]
    fn effective_automix_gain_treats_invalid_linear_gains_as_unity_and_clamps() {
        assert_eq!(
            effective_automix_gain_db(20.0, 0.0, f32::NAN, f32::NAN),
            9.0
        );
        assert_eq!(
            effective_automix_gain_db(-20.0, f32::INFINITY, 0.0, -1.0),
            -9.0
        );
    }

    #[test]
    fn scheduler_cancels_stale_groups() {
        let mut scheduler = AudioClockScheduler::new(Duration::from_secs(1));
        let group = ScheduleGroup(1);
        scheduler.schedule(group, Duration::from_secs(1));
        scheduler.cancel(group);
        assert!(scheduler.poll(Duration::from_secs(2)).is_none());
    }

    #[test]
    fn scheduler_uses_audio_deadline_and_marks_underruns() {
        let mut scheduler = AudioClockScheduler::new(Duration::from_millis(1_500));
        let on_time = ScheduleGroup(1);
        scheduler.schedule(on_time, Duration::from_secs(10));
        assert!(scheduler.poll(Duration::from_millis(9_999)).is_none());
        let action = scheduler.poll(Duration::from_millis(10_075)).unwrap();
        assert_eq!(action.group, on_time);
        assert!(!action.underrun);

        let late = ScheduleGroup(2);
        scheduler.schedule(late, Duration::from_secs(20));
        let action = scheduler.poll(Duration::from_millis(21_501)).unwrap();
        assert_eq!(action.group, late);
        assert!(action.underrun);
    }

    #[test]
    fn aggressive_outro_uses_trusted_vocal_out_and_outro_energy() {
        let mut current = fallback_analysis("a", Duration::from_secs(40));
        let mut next = fallback_analysis("b", Duration::from_secs(40));
        for analysis in [&mut current, &mut next] {
            analysis.bpm = Some(120.0);
            analysis.bpm_confidence = 0.9;
            analysis.transition_confidence = 0.9;
            analysis.first_beat = Some(Duration::ZERO);
            analysis.beat_period = Some(Duration::from_millis(500));
            analysis.energy = 0.8;
        }
        current.cut_out = Duration::from_secs(40);
        current.recommended_exit = Duration::from_secs(35);
        current.vocals_confidence = 0.8;
        current.vocal_out = Some(Duration::from_secs(20));
        current.outro_energy_db = Some(-20.0);
        let plan = plan_transition(&current, &next, Duration::from_secs(5)).unwrap();
        assert!(plan.aggressive_outro);
        assert!(plan.duration <= Duration::from_secs(3));
        assert!(plan.exit < current.recommended_exit);

        current.vocals_confidence = 0.4;
        let plan = plan_transition(&current, &next, Duration::from_secs(5)).unwrap();
        assert!(!plan.aggressive_outro);
    }

    #[test]
    fn planner_snaps_exit_to_bar_and_entry_to_beat() {
        let mut current = fallback_analysis("a", Duration::from_secs(40));
        let mut next = fallback_analysis("b", Duration::from_secs(40));
        for analysis in [&mut current, &mut next] {
            analysis.bpm = Some(120.0);
            analysis.bpm_confidence = 0.9;
            analysis.transition_confidence = 0.9;
            analysis.first_beat = Some(Duration::from_millis(100));
            analysis.beat_period = Some(Duration::from_millis(500));
            analysis.bar_beats = 4;
        }
        current.recommended_exit = Duration::from_millis(13_300);
        next.recommended_entry = Duration::from_millis(1_320);
        let plan = plan_transition(&current, &next, Duration::from_secs(5)).unwrap();
        assert_eq!(plan.exit, Duration::from_millis(14_100));
        assert_eq!(plan.entry, Duration::from_millis(1_100));
        assert!(plan.beat_aligned);
        assert!(plan.bar_aligned);
    }

    #[test]
    fn planner_keeps_grid_alignment_when_the_nearest_point_exceeds_bounds() {
        let mut current = fallback_analysis("a", Duration::from_millis(10_100));
        let mut next = fallback_analysis("b", Duration::from_millis(10_100));
        for analysis in [&mut current, &mut next] {
            analysis.bpm = Some(120.0);
            analysis.bpm_confidence = 0.9;
            analysis.transition_confidence = 0.9;
            analysis.first_beat = Some(Duration::from_millis(100));
            analysis.beat_period = Some(Duration::from_millis(500));
            analysis.bar_beats = 4;
        }
        current.recommended_exit = Duration::from_millis(5_100);
        next.recommended_entry = Duration::from_millis(5_100);

        let plan = plan_transition(&current, &next, Duration::from_secs(5)).unwrap();
        assert_eq!(plan.exit, Duration::from_millis(4_100));
        assert_eq!(plan.entry, Duration::from_millis(5_100));
        assert_eq!(plan.duration, Duration::from_secs(5));
        assert!(plan.beat_aligned);
        assert!(plan.bar_aligned);
        assert!(plan.exit + plan.duration <= current.duration);
        assert!(plan.entry + plan.duration <= next.duration);
    }

    #[test]
    fn transition_directives_keep_manual_and_natural_policies_separate() {
        let group = ScheduleGroup(7);
        assert_eq!(
            TransitionDirective::manual(group).duration,
            Duration::from_millis(MANUAL_CROSSFADE_MS.into())
        );
        assert_eq!(
            TransitionDirective::baseline_natural(group).duration,
            Duration::from_millis(BASELINE_CROSSFADE_MS.into())
        );
    }
}
