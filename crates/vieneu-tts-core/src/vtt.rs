use anyhow::{Context, Result, bail};

#[derive(Clone, Debug, PartialEq)]
pub struct SubtitleCue {
    pub index: usize,
    pub start_sec: f64,
    pub end_sec: f64,
    pub text: String,
}

pub fn parse_vtt(input: &str) -> Result<Vec<SubtitleCue>> {
    let mut cues = Vec::new();
    let normalized = input.replace("\r\n", "\n");

    for block in normalized.split("\n\n") {
        let lines: Vec<&str> = block.lines().map(str::trim).collect();
        if lines.is_empty()
            || lines[0].is_empty()
            || lines[0].starts_with("WEBVTT")
            || lines[0].starts_with("NOTE")
        {
            continue;
        }

        let Some((timing_index, timing)) = lines
            .iter()
            .enumerate()
            .find(|(_, line)| line.contains("-->"))
        else {
            continue;
        };
        let (start, end) = timing
            .split_once("-->")
            .context("Mốc thời gian WebVTT không hợp lệ")?;
        let start_sec = parse_timestamp(start.split_whitespace().next().unwrap_or(""))?;
        let end_sec = parse_timestamp(end.split_whitespace().next().unwrap_or(""))?;
        if end_sec < start_sec {
            bail!("Thời điểm kết thúc cue sớm hơn thời điểm bắt đầu");
        }

        let text = lines[timing_index + 1..]
            .iter()
            .filter(|line| !line.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" ");
        if text.is_empty() {
            continue;
        }

        let index = if timing_index > 0 {
            lines[..timing_index]
                .iter()
                .rev()
                .find_map(|line| line.parse::<usize>().ok())
                .unwrap_or(cues.len() + 1)
        } else {
            cues.len() + 1
        };
        cues.push(SubtitleCue {
            index,
            start_sec,
            end_sec,
            text,
        });
    }

    Ok(cues)
}

fn parse_timestamp(value: &str) -> Result<f64> {
    let parts: Vec<&str> = value.split(':').collect();
    let seconds = match parts.as_slice() {
        [minutes, seconds] => minutes.parse::<f64>()? * 60.0 + seconds.parse::<f64>()?,
        [hours, minutes, seconds] => {
            hours.parse::<f64>()? * 3600.0
                + minutes.parse::<f64>()? * 60.0
                + seconds.parse::<f64>()?
        }
        _ => bail!("Mốc thời gian WebVTT không hợp lệ: {value}"),
    };
    if !seconds.is_finite() || seconds < 0.0 {
        bail!("Mốc thời gian WebVTT nằm ngoài phạm vi hợp lệ: {value}");
    }
    Ok(seconds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn parses_crlf_cues_with_timing_settings_and_optional_identifiers() {
        let input = "WEBVTT\r\n\r\n1\r\n00:00:01.000 --> 00:00:02.500 line:90%\r\nXin chào\r\nthế giới\r\n\r\n00:02.500 --> 00:03.000\r\nTạm biệt";
        let cues = parse_vtt(input).unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].index, 1);
        assert_eq!(cues[0].text, "Xin chào thế giới");
        assert_eq!(cues[1].index, 2);
        assert_eq!(cues[1].start_sec, 2.5);
    }

    proptest! {
        #[test]
        fn arbitrary_input_never_panics(input in "([\\x00-\\x7f]|[\\u{80}-\\u{ffff}])*") {
            let _ = parse_vtt(&input);
        }
    }
}
