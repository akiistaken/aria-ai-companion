//! Speech-to-text through the free web-speech endpoint the original app used.

use std::time::Duration;

// The Chromium web-speech key the Python original shipped with.
const ENDPOINT: &str =
    "https://www.google.com/speech-api/v2/recognize?client=chromium&lang=en-US&key=AIzaSyBOti4mM-6x9WDnZIjIeyEU21OpBXqWBgw";

/// The best transcript of a 16 kHz mono WAV, or "" when nothing was recognised.
pub fn transcribe_wav(wav: &[u8]) -> String {
    if wav.len() <= 44 {
        return String::new();
    }
    let response = ureq::post(ENDPOINT)
        .header("Content-Type", "audio/l16; rate=16000")
        .config()
        .timeout_global(Some(Duration::from_secs(12)))
        .build()
        .send(&wav[44..]);
    let Ok(mut response) = response else {
        log::warn!("speech recognition request failed");
        return String::new();
    };
    response.body_mut().read_to_string().map(|body| best_transcript(&body)).unwrap_or_default()
}

/// The endpoint answers with one JSON object per line; the first non-empty transcript wins.
fn best_transcript(body: &str) -> String {
    body.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line.trim()).ok())
        .find_map(|v| {
            let t = v.pointer("/result/0/alternative/0/transcript")?.as_str()?.trim();
            (!t.is_empty()).then(|| t.to_string())
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcripts_are_read_from_line_delimited_json() {
        let body = "{\"result\":[]}\n{\"result\":[{\"alternative\":[{\"transcript\":\" open spotify \",\"confidence\":0.9}],\"final\":true}],\"result_index\":0}\n";
        assert_eq!(best_transcript(body), "open spotify");
        assert_eq!(best_transcript("{\"result\":[]}"), "");
    }
}
