use once_cell::sync::Lazy;
use regex::Regex;

static PREAMP_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^Preamp:\s*([+-]?\d+(?:\.\d+)?)\s*dB").expect("preamp regex")
});
static FILTER_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"^Filter\s+\d+:\s+(ON|OFF)\s+([A-Z]+)\s+Fc\s+([\d.]+)\s+Hz\s+Gain\s+([+-]?[\d.]+)\s+dB\s+Q\s+([\d.]+)",
    )
    .expect("filter regex")
});

#[derive(Clone, Debug)]
pub enum FilterType {
    Peak,
    LowShelf,
    HighShelf,
}

#[derive(Clone, Debug)]
pub struct EqFilter {
    pub freq: f32,
    pub gain: f32,
    pub q: f32,
    pub kind: FilterType,
}

#[derive(Clone, Debug)]
pub struct EqProfile {
    pub preamp_db: f32,
    pub filters: Vec<EqFilter>,
}

impl EqProfile {
    pub fn to_mpv_af(&self) -> String {
        let mut chain: Vec<String> = Vec::new();
        if self.preamp_db.abs() > 0.01 {
            chain.push(format!("volume={}dB", fmt_f32(self.preamp_db)));
        }
        for filter in &self.filters {
            let base = match filter.kind {
                FilterType::Peak => "equalizer",
                FilterType::LowShelf => "lowshelf",
                FilterType::HighShelf => "highshelf",
            };
            chain.push(format!(
                "{}=f={}:t=q:w={}:g={}",
                base,
                fmt_f32(filter.freq),
                fmt_f32(filter.q),
                fmt_f32(filter.gain)
            ));
        }
        if chain.is_empty() {
            String::new()
        } else {
            format!("lavfi=[{}]", chain.join(","))
        }
    }
}

#[derive(Clone, Debug)]
pub struct Preset {
    pub name: String,
    pub eq: EqProfile,
}

pub fn parse_eq_profile(input: &str) -> EqProfile {
    let mut preamp_db = 0.0;
    let mut filters = Vec::new();

    for line in input.lines() {
        if let Some(caps) = PREAMP_RE.captures(line) {
            preamp_db = caps[1].parse::<f32>().unwrap_or(0.0);
            continue;
        }
        let Some(caps) = FILTER_RE.captures(line) else {
            continue;
        };
        if &caps[1] != "ON" {
            continue;
        }
        let kind_raw = &caps[2];
        let kind = if kind_raw.starts_with("PK") {
            FilterType::Peak
        } else if kind_raw.starts_with("LS") {
            FilterType::LowShelf
        } else if kind_raw.starts_with("HS") {
            FilterType::HighShelf
        } else {
            continue;
        };
        let freq = caps[3].parse::<f32>().unwrap_or(0.0);
        let gain = caps[4].parse::<f32>().unwrap_or(0.0);
        let q = caps[5].parse::<f32>().unwrap_or(0.0);
        filters.push(EqFilter {
            freq,
            gain,
            q,
            kind,
        });
    }

    EqProfile { preamp_db, filters }
}

fn fmt_f32(value: f32) -> String {
    if value.abs() < 0.0005 {
        "0".to_string()
    } else {
        format!("{:.3}", value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_autoeq_style_profile() {
        let sample = "\
Preamp: -6.2 dB
Filter 1: ON PK Fc 105.0 Hz Gain 2.5 dB Q 0.700
Filter 2: ON LS Fc 105.0 Hz Gain 6.0 dB Q 0.700
Filter 3: ON HS Fc 10000.0 Hz Gain -1.5 dB Q 0.700
Filter 4: OFF PK Fc 200.0 Hz Gain 1.0 dB Q 1.000
";
        let eq = parse_eq_profile(sample);
        assert!((eq.preamp_db - (-6.2)).abs() < 0.001);
        assert_eq!(eq.filters.len(), 3);
        assert!(eq.to_mpv_af().contains("lavfi="));
        assert!(eq.to_mpv_af().contains("volume=-6.200dB"));
    }
}
