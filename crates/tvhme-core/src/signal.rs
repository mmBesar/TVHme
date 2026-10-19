//! Signal status from HTSP `signalStatus` messages.
use crate::htsmsg::{Map, MapExt};

#[derive(Clone, Debug, PartialEq)]
pub struct SignalStatus {
    pub subscription_id: u32,
    pub status: String,
    /// SNR percent (0..=100), if reported.
    pub snr_pct: Option<f32>,
    /// Signal strength percent (0..=100), if reported.
    pub signal_pct: Option<f32>,
    pub ber: Option<u32>,
    pub unc: Option<u32>,
}

/// HTSP reports relative values on a 0..=65535 scale (65535 = 100%).
pub fn relative_to_pct(v: u32) -> f32 {
    (v.min(65535) as f32 / 65535.0 * 100.0).clamp(0.0, 100.0)
}

impl SignalStatus {
    pub fn from_msg(m: &Map) -> Option<Self> {
        if m.str("method") != Some("signalStatus") { return None; }
        Some(Self {
            subscription_id: m.u32("subscriptionId")?,
            status: m.str("feStatus").unwrap_or("").to_string(),
            snr_pct: m.u32("feSNR").map(relative_to_pct),
            signal_pct: m.u32("feSignal").map(relative_to_pct),
            ber: m.u32("feBER"),
            unc: m.u32("feUNC"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::htsmsg::Value;
    #[test]
    fn parses_signal_status() {
        let mut m = Map::new();
        m.insert("method".into(), Value::Str("signalStatus".into()));
        m.insert("subscriptionId".into(), Value::S64(3));
        m.insert("feStatus".into(), Value::Str("GOOD".into()));
        m.insert("feSNR".into(), Value::S64(65535));
        m.insert("feSignal".into(), Value::S64(32768));
        let s = SignalStatus::from_msg(&m).unwrap();
        assert_eq!(s.snr_pct, Some(100.0));
        assert!((s.signal_pct.unwrap() - 50.0).abs() < 0.1);
        assert_eq!(s.ber, None);
    }
    #[test]
    fn ignores_other_methods() {
        let mut m = Map::new();
        m.insert("method".into(), Value::Str("channelAdd".into()));
        assert!(SignalStatus::from_msg(&m).is_none());
    }
}
