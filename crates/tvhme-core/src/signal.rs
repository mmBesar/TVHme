//! Signal quality, from HTSP `signalStatus` (any user) or `status/inputs` (admin only).
use crate::api::InputStatus;
use crate::htsmsg::{Map, MapExt};

#[derive(Clone, Debug, PartialEq, Default)]
pub struct SignalStatus {
    pub status: String,
    /// Percent 0..=100 when the server reports a relative value.
    pub snr_pct: Option<f32>,
    pub signal_pct: Option<f32>,
    /// Decibels when the server reports an absolute value.
    pub snr_db: Option<f32>,
    pub signal_db: Option<f32>,
    pub ber: Option<u32>,
    pub unc: Option<u32>,
}

/// Relative scale: 65535 = 100%.
pub fn relative_to_pct(v: u32) -> f32 { (v.min(65535) as f32 / 65535.0 * 100.0).clamp(0.0, 100.0) }

impl SignalStatus {
    /// HTSP `signalStatus`. Values are treated as the relative 0..=65535 scale (as other HTSP
    /// clients do); the HTSP docs do not state the unit. Set TVHME_DEBUG_SIGNAL=1 to print raw values.
    pub fn from_htsp(m: &Map) -> Option<(u32, Self)> {
        if m.str("method") != Some("signalStatus") { return None; }
        if std::env::var_os("TVHME_DEBUG_SIGNAL").is_some() { eprintln!("signalStatus raw: {m:?}"); }
        let id = m.u32("subscriptionId")?;
        Some((id, Self {
            status: m.str("feStatus").unwrap_or("").to_string(),
            snr_pct: m.u32("feSNR").map(relative_to_pct),
            signal_pct: m.u32("feSignal").map(relative_to_pct),
            ber: m.u32("feBER"),
            unc: m.u32("feUNC"),
            ..Default::default()
        }))
    }

    /// `status/inputs`: scale 1 = relative (65535 = 100%), 2 = absolute (1000 = 1 dB), 0 = unavailable.
    pub fn from_input(i: &InputStatus) -> Self {
        let conv = |v: Option<u32>, scale: u32| -> (Option<f32>, Option<f32>) {
            match (v, scale) {
                (Some(v), 1) => (Some(relative_to_pct(v)), None),
                (Some(v), 2) => (None, Some(v as f32 / 1000.0)),
                _ => (None, None),
            }
        };
        let (snr_pct, snr_db) = conv(i.snr, i.snr_scale);
        let (signal_pct, signal_db) = conv(i.signal, i.signal_scale);
        Self { status: i.stream.clone(), snr_pct, signal_pct, snr_db, signal_db, ber: i.ber, unc: i.unc }
    }

    pub fn has_values(&self) -> bool {
        self.snr_pct.is_some() || self.signal_pct.is_some() || self.snr_db.is_some() || self.signal_db.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::htsmsg::Value;

    #[test]
    fn htsp_signal_status_relative() {
        let mut m = Map::new();
        m.insert("method".into(), Value::Str("signalStatus".into()));
        m.insert("subscriptionId".into(), Value::S64(3));
        m.insert("feStatus".into(), Value::Str("GOOD".into()));
        m.insert("feSNR".into(), Value::S64(65535));
        m.insert("feSignal".into(), Value::S64(32768));
        let (id, s) = SignalStatus::from_htsp(&m).unwrap();
        assert_eq!(id, 3);
        assert_eq!(s.snr_pct, Some(100.0));
        assert!((s.signal_pct.unwrap() - 50.0).abs() < 0.1);
        assert_eq!(s.ber, None);
    }
    #[test]
    fn ignores_other_htsp_methods() {
        let mut m = Map::new();
        m.insert("method".into(), Value::Str("channelAdd".into()));
        assert!(SignalStatus::from_htsp(&m).is_none());
    }
    #[test]
    fn status_inputs_scales() {
        let rel = InputStatus { input: "a".into(), stream: "mux".into(), subs: 1, signal: Some(32768), signal_scale: 1, snr: Some(18500), snr_scale: 2, ber: None, unc: Some(0) };
        let s = SignalStatus::from_input(&rel);
        assert!((s.signal_pct.unwrap() - 50.0).abs() < 0.1);
        assert_eq!(s.snr_pct, None);
        assert!((s.snr_db.unwrap() - 18.5).abs() < 0.001);
        let none = InputStatus { signal_scale: 0, snr_scale: 0, ..rel };
        assert!(!SignalStatus::from_input(&none).has_values());
    }
}
