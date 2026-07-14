pub fn normalized_search(input: &str) -> String {
    input
        .to_lowercase()
        .chars()
        .filter(|ch| !matches!(ch, ' ' | '_' | '-' | '/' | '\\' | '(' | ')' | '[' | ']'))
        .collect()
}

pub fn format_time(seconds: f64) -> String {
    if !seconds.is_finite() || seconds < 0.0 {
        return "0:00".to_string();
    }
    let total = seconds.floor() as u64;
    let hours = total / 3600;
    let mins = (total % 3600) / 60;
    let secs = total % 60;
    if hours > 0 {
        format!("{hours}:{mins:02}:{secs:02}")
    } else {
        format!("{mins}:{secs:02}")
    }
}

pub fn format_volume(volume: f64) -> String {
    format!("{:.0}%", volume.clamp(0.0, 150.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_formatting() {
        assert_eq!(format_time(0.0), "0:00");
        assert_eq!(format_time(65.2), "1:05");
        assert_eq!(format_time(3723.0), "1:02:03");
    }

    #[test]
    fn search_normalization() {
        assert_eq!(normalized_search("HD-600 / A"), "hd600a");
    }
}
