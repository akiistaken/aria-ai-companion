//! Own-player observations and deterministic metrics. Unknown values stay absent.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Unknown,
    Lobby,
    InMatch,
    Results,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Win,
    Loss,
    Draw,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stats {
    pub kills: Option<u32>,
    pub deaths: Option<u32>,
    pub assists: Option<u32>,
    pub rounds_played: Option<u32>,
    pub total_combat_score: Option<f64>,
    pub total_damage: Option<f64>,
    pub headshots: Option<u32>,
    pub bodyshots: Option<u32>,
    pub legshots: Option<u32>,
    /// Displayed averages from a results screen; never inferred from a partial scoreboard.
    pub displayed_acs: Option<f64>,
    pub displayed_adr: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Metrics {
    pub kd: Option<f64>,
    pub acs: Option<f64>,
    pub adr: Option<f64>,
    /// Hit-based percentage: head hits / all recorded hits, not headshot kills / kills.
    pub headshot_percentage: Option<f64>,
}

impl Stats {
    pub fn metrics(&self) -> Metrics {
        let rounds = self.rounds_played.filter(|n| *n > 0).map(f64::from);
        let hits = self
            .headshots
            .zip(self.bodyshots)
            .zip(self.legshots)
            .map(|((h, b), l)| u64::from(h) + u64::from(b) + u64::from(l));
        Metrics {
            kd: self
                .kills
                .zip(self.deaths)
                .filter(|(_, d)| *d > 0)
                .map(|(k, d)| f64::from(k) / f64::from(d)),
            acs: self
                .total_combat_score
                .zip(rounds)
                .map(|(s, r)| s / r)
                .or(self.displayed_acs),
            adr: self
                .total_damage
                .zip(rounds)
                .map(|(d, r)| d / r)
                .or(self.displayed_adr),
            headshot_percentage: self
                .headshots
                .zip(hits)
                .filter(|(_, n)| *n > 0)
                .map(|(h, n)| f64::from(h) * 100.0 / n as f64),
        }
    }

    fn screen_sanitize(&mut self, results: bool) {
        for value in [&mut self.kills, &mut self.deaths, &mut self.assists] {
            if value.is_some_and(|v| v > 1000) {
                *value = None;
            }
        }
        // Frame extraction cannot establish total damage, hit counts or rounds participated in.
        self.rounds_played = None;
        self.total_combat_score = None;
        self.total_damage = None;
        self.headshots = None;
        self.bodyshots = None;
        self.legshots = None;
        for average in [&mut self.displayed_acs, &mut self.displayed_adr] {
            if !results || average.is_some_and(|n| !n.is_finite() || !(0.0..=10000.0).contains(&n))
            {
                *average = None;
            }
        }
    }
}

/// The model returns only this schema. Source, account and timestamps are assigned locally.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct FrameReading {
    pub phase: Phase,
    pub phase_confidence: f64,
    pub own_row_confirmed: bool,
    pub own_row_confidence: f64,
    pub map: Option<String>,
    pub agent: Option<String>,
    pub team_score: Option<[u32; 2]>,
    pub score_confidence: f64,
    pub outcome: Option<Outcome>,
    pub stats: Stats,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    pub account: String,
    pub source: String,
    pub observed_at: u64,
    pub phase: Phase,
    pub confidence: f64,
    pub map: Option<String>,
    pub agent: Option<String>,
    pub team_score: Option<[u32; 2]>,
    pub outcome: Option<Outcome>,
    pub stats: Stats,
}

fn confident(value: f64) -> bool {
    value.is_finite() && (0.85..=1.0).contains(&value)
}

fn label(value: Option<String>) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s.chars().count() <= 64 && !s.chars().any(char::is_control))
}

impl FrameReading {
    pub fn observation(mut self, account: &str, at: u64) -> Observation {
        if !confident(self.phase_confidence) {
            self.phase = Phase::Unknown;
        }
        let own = self.own_row_confirmed
            && confident(self.own_row_confidence)
            && matches!(self.phase, Phase::InMatch | Phase::Results);
        if !own {
            self.stats = Stats::default();
            self.agent = None;
        }
        self.stats.screen_sanitize(self.phase == Phase::Results);
        if !confident(self.score_confidence)
            || self.phase == Phase::Unknown
            || self.team_score.is_some_and(|s| s.iter().any(|n| *n > 100))
        {
            self.team_score = None;
        }
        // An outcome is useful only on an identified own-player results screen.
        if self.phase != Phase::Results || !own {
            self.outcome = None;
        }
        if self.phase == Phase::Unknown {
            self.map = None;
            self.agent = None;
        }
        Observation {
            account: account.into(),
            source: "visible_screen".into(),
            observed_at: at,
            phase: self.phase,
            confidence: if self.phase == Phase::Unknown {
                0.0
            } else {
                self.phase_confidence
            },
            map: label(self.map),
            agent: label(self.agent),
            team_score: self.team_score,
            outcome: self.outcome,
            stats: self.stats,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchRecord {
    pub id: String,
    pub account: String,
    pub source: String,
    pub recorded_at: u64,
    pub map: Option<String>,
    pub agent: Option<String>,
    pub team_score: Option<[u32; 2]>,
    pub outcome: Option<Outcome>,
    pub stats: Stats,
}

impl Observation {
    pub fn completed(&self, id: String) -> Option<MatchRecord> {
        if self.phase != Phase::Results
            || self.stats.kills.is_none()
            || self.stats.deaths.is_none()
            || self.stats.assists.is_none()
        {
            return None;
        }
        Some(MatchRecord {
            id,
            account: self.account.clone(),
            source: self.source.clone(),
            recorded_at: self.observed_at,
            map: self.map.clone(),
            agent: self.agent.clone(),
            team_score: self.team_score,
            outcome: self.outcome,
            stats: self.stats.clone(),
        })
    }
}

pub fn recap(record: &MatchRecord) -> String {
    let mut parts = vec![record.map.clone().unwrap_or_else(|| "Match".into())];
    if let Some(outcome) = record.outcome {
        parts.push(format!("{outcome:?}").to_lowercase());
    }
    if let Some([own, other]) = record.team_score {
        parts.push(format!("{own}-{other}"));
    }
    let stat = |n: Option<u32>| n.map(|n| n.to_string()).unwrap_or_else(|| "?".into());
    parts.push(format!(
        "K/D/A {}/{}/{}",
        stat(record.stats.kills),
        stat(record.stats.deaths),
        stat(record.stats.assists)
    ));
    let metrics = record.stats.metrics();
    if let Some(kd) = metrics.kd {
        parts.push(format!("K/D {kd:.2}"));
    }
    if let Some(acs) = metrics.acs {
        parts.push(format!("ACS {acs:.1}"));
    }
    if let Some(adr) = metrics.adr {
        parts.push(format!("ADR {adr:.1}"));
    }
    if let Some(hs) = metrics.headshot_percentage {
        parts.push(format!("headshot hits {hs:.1}%"));
    }
    let review = if record.stats.deaths.is_some_and(|d| d > 0) {
        "For your next review, check the replay of your deaths for positioning and opportunities to trade."
    } else {
        "For your next review, check the replay for utility timing and team coordination."
    };
    format!("{}. {review} Source: {}.", parts.join(" · "), record.source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_and_zero_death_metrics_do_not_become_fake_zeroes() {
        assert_eq!(Stats::default().metrics(), Metrics::default());
        assert!(Stats {
            kills: Some(10),
            deaths: Some(0),
            ..Default::default()
        }
        .metrics()
        .kd
        .is_none());
    }

    #[test]
    fn metrics_use_rounds_and_hit_counts_with_correct_denominators() {
        let stats = Stats {
            kills: Some(20),
            deaths: Some(10),
            rounds_played: Some(20),
            total_combat_score: Some(4000.0),
            total_damage: Some(3000.0),
            headshots: Some(10),
            bodyshots: Some(25),
            legshots: Some(5),
            ..Default::default()
        };
        assert_eq!(
            stats.metrics(),
            Metrics {
                kd: Some(2.0),
                acs: Some(200.0),
                adr: Some(150.0),
                headshot_percentage: Some(25.0)
            }
        );
    }

    #[test]
    fn ambiguous_own_row_and_in_match_averages_are_discarded() {
        let mut frame = FrameReading {
            phase: Phase::InMatch,
            phase_confidence: 0.99,
            own_row_confirmed: false,
            own_row_confidence: 0.99,
            stats: Stats {
                kills: Some(30),
                displayed_acs: Some(300.0),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            frame.clone().observation("test#tag", 1).stats,
            Stats::default()
        );
        frame.own_row_confirmed = true;
        let observation = frame.observation("test#tag", 1);
        assert_eq!(observation.stats.kills, Some(30));
        assert!(observation.stats.displayed_acs.is_none());
        assert!(observation.completed("x".into()).is_none());
    }

    #[test]
    fn uncertain_phase_never_creates_a_completed_match() {
        let frame = FrameReading {
            phase: Phase::Results,
            phase_confidence: 0.6,
            own_row_confirmed: true,
            own_row_confidence: 0.99,
            stats: Stats {
                kills: Some(20),
                deaths: Some(10),
                assists: Some(5),
                ..Default::default()
            },
            ..Default::default()
        };
        let observation = frame.observation("test#tag", 1);
        assert_eq!(observation.phase, Phase::Unknown);
        assert!(observation.completed("x".into()).is_none());
    }
}
