use rustle_domain::lyrics::{LyricLineOwned, MAX_LRC_TIMESTAMP};

/// Word-timed formats carry their own line envelope. Do not replace it with
/// first/last word times; translation sidecars can be anchored to that envelope.
pub(super) fn finish_timed_lines(lines: &mut [LyricLineOwned]) {
    for line in lines.iter_mut() {
        line.start_time = line.start_time.min(MAX_LRC_TIMESTAMP);
        line.end_time = line.end_time.min(MAX_LRC_TIMESTAMP).max(line.start_time);
        for word in &mut line.words {
            word.start_time = word.start_time.min(MAX_LRC_TIMESTAMP);
            word.end_time = word.end_time.min(MAX_LRC_TIMESTAMP).max(word.start_time);
        }
    }
    lines.sort_by_key(|line| line.start_time);
}
