use anyhow::{Context, Result, bail};

#[derive(Clone, Debug, PartialEq)]
pub struct SubtitleCue {
    pub index: usize,
    pub start_sec: f64,
    pub end_sec: f64,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VttParseResult {
    pub cues: Vec<SubtitleCue>,
    pub total_cues: usize,
    pub filtered_captions: usize,
    pub malformed_line_settings: usize,
}

pub fn parse_vtt(input: &str) -> Result<Vec<SubtitleCue>> {
    Ok(parse_vtt_with_stats(input)?.cues)
}

pub fn parse_vtt_with_stats(input: &str) -> Result<VttParseResult> {
    let mut result = VttParseResult::default();
    let normalized = input.replace("\r\n", "\n");
    let mut cue_sequence = 0;

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
        let end_parts: Vec<&str> = end.split_whitespace().collect();
        let end_sec = parse_timestamp(end_parts.first().copied().unwrap_or(""))?;
        if end_sec < start_sec {
            bail!("Thời điểm kết thúc cue sớm hơn thời điểm bắt đầu");
        }
        cue_sequence += 1;
        result.total_cues = cue_sequence;

        let mut line_position = None;
        let mut malformed_line_setting = false;
        for setting in end_parts
            .iter()
            .skip(1)
            .filter(|part| part.starts_with("line:"))
        {
            let value = setting
                .strip_prefix("line:")
                .and_then(|value| value.split(',').next())
                .and_then(|value| value.strip_suffix('%'))
                .and_then(|value| value.parse::<f64>().ok())
                .filter(|value| value.is_finite());
            if let Some(value) = value {
                line_position = Some(value);
            } else {
                malformed_line_setting = true;
            }
        }
        if malformed_line_setting {
            result.malformed_line_settings += 1;
        } else if line_position.is_some_and(|position| position < 50.0) {
            result.filtered_captions += 1;
            continue;
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
                .unwrap_or(cue_sequence)
        } else {
            cue_sequence
        };
        result.cues.push(SubtitleCue {
            index,
            start_sec,
            end_sec,
            text,
        });
    }

    Ok(result)
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

    #[test]
    fn filters_captions_and_keeps_malformed_line_settings_with_warning_count() {
        let input = concat!(
            "WEBVTT\n\n",
            "00:00:00.000 --> 00:00:01.000 line:0%\ncaption\n\n",
            "00:00:01.000 --> 00:00:02.000 line:49.9% position:50%\ncaption\n\n",
            "00:00:02.000 --> 00:00:03.000 line:50%\nspoken\n\n",
            "00:00:03.000 --> 00:00:04.000 line:85% align:start\nspoken\n\n",
            "00:00:04.000 --> 00:00:05.000\nspoken\n\n",
            "00:00:05.000 --> 00:00:06.000 line:abc%\nkept with warning"
        );
        let result = parse_vtt_with_stats(input).unwrap();
        assert_eq!(result.filtered_captions, 2);
        assert_eq!(result.total_cues, 6);
        assert_eq!(result.malformed_line_settings, 1);
        assert_eq!(result.cues.len(), 4);
        assert_eq!(result.cues[0].index, 3);
        assert_eq!(result.cues[3].index, 6);
    }

    proptest! {
        #[test]
        fn line_percentage_filters_only_values_below_fifty(position in 0.0f64..100.0) {
            let input = format!(
                "WEBVTT\n\n00:00:00.000 --> 00:00:01.000 line:{position}%\ntext"
            );
            let result = parse_vtt_with_stats(&input).unwrap();
            prop_assert_eq!(result.filtered_captions, usize::from(position < 50.0));
            prop_assert_eq!(result.cues.len(), usize::from(position >= 50.0));
        }

        #[test]
        fn arbitrary_input_never_panics(input in "([\\x00-\\x7f]|[\\u{80}-\\u{ffff}])*") {
            let _ = parse_vtt_with_stats(&input);
        }
    }
}
