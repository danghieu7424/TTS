/****
 * [ATOM]: dsp
 * Chức năng: Xử lý tín hiệu số âm thanh (DSP)
 * Bao gồm: Trim silence, Fade-in, Fade-out, và Dynamic Time-Stretching (anti-bleeding)
 ****/

/// Đo khoảng lặng hai đầu bằng năng lượng trung bình cửa sổ 10ms (Windowed Envelope Mean)
/// Chuẩn xác 100% theo thuật toán edge_silence trong core_utils.py của VieNeu-TTS
pub fn edge_silence(
    samples: &[f32],
    sample_rate: usize,
    thresh_db: f32,
    win_sec: f32,
) -> (usize, usize) {
    let n_samp = samples.len();
    if n_samp == 0 {
        return (0, 0);
    }

    let win = ((win_sec * sample_rate as f32).round() as usize).max(1);
    let n_win = n_samp / win;
    if n_win == 0 {
        return (n_samp, 0);
    }

    let threshold_amp = 10.0f32.powf(thresh_db / 20.0);

    let mut first_win = None;
    let mut last_win = None;

    for w in 0..n_win {
        let start = w * win;
        let end = start + win;
        let mut sum_abs = 0.0f32;
        for &s in &samples[start..end] {
            sum_abs += s.abs();
        }
        let mean = sum_abs / win as f32;

        if mean > threshold_amp {
            if first_win.is_none() {
                first_win = Some(w);
            }
            last_win = Some(w);
        }
    }

    match (first_win, last_win) {
        (Some(first), Some(last)) => {
            let lead = first * win;
            let tail = n_samp.saturating_sub((last + 1) * win);
            (lead, tail)
        }
        _ => (n_samp, 0), // Toàn bộ là im lặng
    }
}

/// Cắt khoảng lặng hai đầu và áp dụng Fade Cosine theo chuẩn VieNeu-TTS Python
/// Giữ lại keep_sec (mặc định 40ms) để giữ formant tự nhiên, fade cosine 15ms ở 2 mép
pub fn trim_and_fade(
    samples: &[f32],
    sample_rate: usize,
    thresh_db: f32,
    keep_sec: f32,
    fade_sec: f32,
) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }

    let (lead, tail) = edge_silence(samples, sample_rate, thresh_db, 0.01);
    let keep = (keep_sec * sample_rate as f32).round() as usize;

    let a = lead.saturating_sub(keep);
    let b = samples.len().saturating_sub(tail.saturating_sub(keep));

    if a >= b {
        return Vec::new();
    }

    let mut out = samples[a..b].to_vec();
    let n = ((fade_sec * sample_rate as f32).round() as usize).min(out.len() / 2);

    if n > 0 {
        let len = out.len();
        for i in 0..n {
            // Cosine ramp: 0.5 - 0.5 * cos(pi * i / n)
            let phase = std::f32::consts::PI * (i as f32) / (n as f32);
            let ramp = 0.5 - 0.5 * phase.cos();
            out[i] *= ramp;
            out[len - 1 - i] *= ramp;
        }
    }

    out
}

pub const MAX_COMPRESSION_RATIO: f64 = 1.35;

/// Chuyển đổi mảng f32 sang i16 PCM với soft limiter và chống wrap-around.
pub fn f32_to_i16_pcm(samples: &[f32]) -> Vec<i16> {
    samples
        .iter()
        .map(|&s| {
            let finite = if s.is_finite() { s } else { 0.0 };
            let magnitude = finite.abs();
            let limited = if magnitude <= 0.98 {
                finite
            } else {
                let knee = 0.02;
                let softened = 0.98 + knee * (1.0 - (-(magnitude - 0.98) / knee).exp());
                finite.signum() * softened.min(1.0)
            };
            (limited.clamp(-1.0, 1.0) * 32767.0) as i16
        })
        .collect()
}

/// WSOLA bypasses changes below 3% and limits compression to 1.35x.
pub fn wsola_time_stretch(samples: &[f32], target_len: usize, sample_rate: usize) -> Vec<f32> {
    if samples.is_empty() || target_len == 0 {
        return Vec::new();
    }
    let speed_ratio = samples.len() as f64 / target_len as f64;
    if (speed_ratio - 1.0).abs() < 0.03 {
        return samples.to_vec();
    }
    if samples.len() < 2 {
        return vec![samples[0]; target_len];
    }

    let minimum_target_len = (samples.len() as f64 / MAX_COMPRESSION_RATIO).ceil() as usize;
    let target_len = target_len.max(minimum_target_len);
    let window = samples.len().min((sample_rate / 25).max(2));
    let overlap = (window / 2).max(1);
    let synthesis_hop = window - overlap;
    let analysis_hop = synthesis_hop as f64 * samples.len() as f64 / target_len as f64;
    let search_radius = (sample_rate / 200).min(window / 4);
    let max_source_start = samples.len() - window;
    let mut output = vec![0.0; target_len];
    let first_len = window.min(target_len);
    output[..first_len].copy_from_slice(&samples[..first_len]);

    let mut output_start = synthesis_hop;
    let mut expected_source_start = analysis_hop;
    while output_start < target_len {
        let expected = expected_source_start.round() as usize;
        let search_start = expected.saturating_sub(search_radius).min(max_source_start);
        let search_end = expected.saturating_add(search_radius).min(max_source_start);
        let mut best_source_start = search_start;
        let mut best_correlation = f64::NEG_INFINITY;

        for candidate in (search_start..=search_end).step_by(8) {
            let compared = overlap.min(target_len - output_start);
            let (mut correlation, mut output_energy, mut input_energy) = (0.0, 0.0, 0.0);
            for offset in (0..compared).step_by(8) {
                let outgoing = f64::from(output[output_start + offset]);
                let incoming = f64::from(samples[candidate + offset]);
                correlation += outgoing * incoming;
                output_energy += outgoing * outgoing;
                input_energy += incoming * incoming;
            }
            let correlation =
                correlation / (output_energy * input_energy).sqrt().max(f64::MIN_POSITIVE);
            if correlation > best_correlation {
                best_correlation = correlation;
                best_source_start = candidate;
            }
        }

        let segment_len = window.min(target_len - output_start);
        let crossfade_len = overlap.min(segment_len);
        for offset in 0..crossfade_len {
            let blend = if crossfade_len == 1 {
                1.0
            } else {
                let phase = std::f32::consts::PI * offset as f32 / (crossfade_len - 1) as f32;
                0.5 - 0.5 * phase.cos()
            };
            let current = output[output_start + offset];
            let incoming = samples[best_source_start + offset];
            output[output_start + offset] = current * (1.0 - blend) + incoming * blend;
        }
        output[output_start + crossfade_len..output_start + segment_len].copy_from_slice(
            &samples[best_source_start + crossfade_len..best_source_start + segment_len],
        );

        output_start += synthesis_hop;
        expected_source_start += analysis_hop;
    }

    output
}

/****
 * [FUNCTION]: gentle_lowpass
 * Chức năng: Bộ lọc thông thấp IIR Biquad Butterworth bậc 2.
 * Đầu vào: samples (&[f32]), sample_rate (usize), cutoff_hz (f32)
 * Đầu ra: Vec<f32> sạch nhiễu
 * Cắt tần số phía trên cutoff_hz bằng biquad Butterworth bậc 2.
 ****/
pub fn gentle_lowpass(samples: &[f32], sample_rate: usize, cutoff_hz: f32) -> Vec<f32> {
    if samples.is_empty() || sample_rate == 0 {
        return Vec::new();
    }

    // Giới hạn an toàn: Cutoff nằm trong dải (100Hz .. Nyquist - 100Hz)
    let nyquist = sample_rate as f32 * 0.5;
    let requested_cutoff = if cutoff_hz.is_finite() {
        cutoff_hz
    } else {
        nyquist * 0.5
    };
    let cutoff = requested_cutoff.clamp((nyquist * 0.01).max(1.0), (nyquist * 0.99).max(1.0));

    // Tính toán góc quay tần số số học
    let omega = 2.0 * std::f32::consts::PI * cutoff / (sample_rate as f32);
    let sin_omega = omega.sin();
    let cos_omega = omega.cos();

    // RBJ biquad: alpha = sin(omega) / (2Q), với Q = 1/sqrt(2).
    let alpha = sin_omega * std::f32::consts::FRAC_1_SQRT_2;

    let inv_a0 = 1.0 / (1.0 + alpha);
    let b0 = ((1.0 - cos_omega) * 0.5) * inv_a0;
    let b1 = (1.0 - cos_omega) * inv_a0;
    let b2 = b0;
    let a1 = (-2.0 * cos_omega) * inv_a0;
    let a2 = (1.0 - alpha) * inv_a0;

    let mut out = Vec::with_capacity(samples.len());
    let mut x1 = 0.0f32;
    let mut x2 = 0.0f32;
    let mut y1 = 0.0f32;
    let mut y2 = 0.0f32;

    for &sample in samples {
        let x = if sample.is_finite() {
            sample.clamp(-1.0e6, 1.0e6)
        } else {
            0.0
        };
        let output = b0 * x + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
        let y = if output.is_finite() { output } else { 0.0 };
        x2 = x1;
        x1 = x;
        y2 = y1;
        y1 = y;
        out.push(y);
    }

    out
}

/****
 * [FUNCTION]: highpass_filter
 * Chức năng: Bộ lọc thông cao IIR Biquad Butterworth bậc 2
 * Đầu vào: samples (&[f32]), sample_rate (usize), cutoff_hz (f32)
 * Đầu ra: Vec<f32> đã khử dải trầm bùn (sub-bass rumble / mud)
 * Ranh giới bảo vệ: Triệt tiêu các tần số dưới cutoff_hz để chống cảm giác ngợp, tức ngực.
 ****/
pub fn highpass_filter(samples: &[f32], sample_rate: usize, cutoff_hz: f32) -> Vec<f32> {
    if samples.is_empty() || sample_rate == 0 {
        return Vec::new();
    }

    let nyquist = sample_rate as f32 * 0.5;
    let requested_cutoff = if cutoff_hz.is_finite() {
        cutoff_hz
    } else {
        nyquist * 0.5
    };
    let cutoff = requested_cutoff.clamp((nyquist * 0.01).max(1.0), (nyquist * 0.99).max(1.0));

    let omega = 2.0 * std::f32::consts::PI * cutoff / (sample_rate as f32);
    let sin_omega = omega.sin();
    let cos_omega = omega.cos();

    // Q = 1/sqrt(2) cho Butterworth maximally flat
    let alpha = sin_omega * std::f32::consts::FRAC_1_SQRT_2;

    let inv_a0 = 1.0 / (1.0 + alpha);
    let b0 = ((1.0 + cos_omega) * 0.5) * inv_a0;
    let b1 = -(1.0 + cos_omega) * inv_a0;
    let b2 = b0;
    let a1 = (-2.0 * cos_omega) * inv_a0;
    let a2 = (1.0 - alpha) * inv_a0;

    let mut out = Vec::with_capacity(samples.len());
    let mut x1 = 0.0f32;
    let mut x2 = 0.0f32;
    let mut y1 = 0.0f32;
    let mut y2 = 0.0f32;

    for &sample in samples {
        let x = if sample.is_finite() {
            sample.clamp(-1.0e6, 1.0e6)
        } else {
            0.0
        };
        let output = b0 * x + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
        let y = if output.is_finite() { output } else { 0.0 };
        x2 = x1;
        x1 = x;
        y2 = y1;
        y1 = y;
        out.push(y);
    }

    out
}

/****
 * [FUNCTION]: vocal_clarity_filter
 * Chức năng: Chuỗi xử lý âm thanh phòng thu (Studio Vocal Shaping)
 * 1. High-pass Filter theo low_cut_hz
 * 2. Low-pass Filter theo high_cut_hz
 * 3. Hòa trộn tín hiệu đã lọc với tín hiệu gốc theo wet_ratio
 ****/
pub fn vocal_clarity_filter(
    samples: &[f32],
    sample_rate: usize,
    low_cut_hz: f32,
    high_cut_hz: f32,
    wet_ratio: f32,
) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }

    // Bước 1: Khử dải trầm bùn
    let hp = highpass_filter(samples, sample_rate, low_cut_hz);

    // Bước 2: Khử dải cao xẹt xẹt
    let lp = gentle_lowpass(&hp, sample_rate, high_cut_hz);

    // Bước 3: Hòa trộn tỷ lệ Wet/Dry (ví dụ 60% Wet / 40% Dry) để giữ lại độ tự nhiên
    let wet = if wet_ratio.is_finite() {
        wet_ratio.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let dry = 1.0 - wet;

    samples
        .iter()
        .zip(lp.iter())
        .map(|(&s_dry, &s_wet)| {
            let dry_sample = if s_dry.is_finite() { s_dry } else { 0.0 };
            let wet_sample = if s_wet.is_finite() { s_wet } else { 0.0 };
            dry * dry_sample + wet * wet_sample
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn wsola_produces_requested_length_and_finite_samples(
            samples in prop::collection::vec(-1.0f32..1.0, 0..4096),
            target_len in 0usize..8192,
        ) {
            let output = wsola_time_stretch(&samples, target_len, 48_000);
            let expected_len = if samples.is_empty() || target_len == 0 {
                0
            } else {
                let speed_ratio = samples.len() as f64 / target_len as f64;
                if (speed_ratio - 1.0).abs() < 0.03 {
                    samples.len()
                } else if speed_ratio > MAX_COMPRESSION_RATIO && samples.len() >= 2 {
                    (samples.len() as f64 / MAX_COMPRESSION_RATIO).ceil() as usize
                } else {
                    target_len
                }
            };
            prop_assert_eq!(output.len(), expected_len);
            prop_assert!(output.iter().all(|sample| sample.is_finite()));
        }

        #[test]
        fn filters_keep_finite_samples(
            samples in prop::collection::vec(any::<f32>(), 0..512),
            sample_rate in 0usize..96001,
            cutoff in any::<f32>(),
            wet in any::<f32>(),
        ) {
            let low = gentle_lowpass(&samples, sample_rate, cutoff);
            let high = highpass_filter(&samples, sample_rate, cutoff);
            let vocal = vocal_clarity_filter(&samples, sample_rate, 60.0, cutoff, wet);
            prop_assert!(low.iter().chain(&high).chain(&vocal).all(|s| s.is_finite()));
        }

        #[test]
        fn trim_and_fade_preserve_valid_bounds(
            samples in prop::collection::vec(-1.0f32..1.0, 0..2048),
            sample_rate in 1usize..96001,
        ) {
            let output = trim_and_fade(&samples, sample_rate, -45.0, 0.04, 0.015);
            prop_assert!(output.len() <= samples.len());
            prop_assert!(output.iter().all(|sample| sample.is_finite()));
        }

        #[test]
        fn pcm_conversion_always_clamps_to_i16_range(samples in prop::collection::vec(any::<f32>(), 0..2048)) {
            let output = f32_to_i16_pcm(&samples);
            prop_assert_eq!(output.len(), samples.len());
            prop_assert!(output.iter().all(|sample| (-32767..=32767).contains(sample)));
        }
    }

    #[test]
    fn invalid_sample_rate_is_handled_without_non_finite_output() {
        assert!(gentle_lowpass(&[0.5], 0, 1000.0).is_empty());
        assert!(highpass_filter(&[0.5], 0, 100.0).is_empty());
    }

    #[test]
    fn filters_sanitize_non_finite_inputs_and_parameters() {
        let samples = [f32::NAN, f32::INFINITY, 0.25];
        let filtered = vocal_clarity_filter(&samples, 48_000, f32::NAN, 4_000.0, f32::NAN);
        assert!(filtered.iter().all(|sample| sample.is_finite()));
    }

    #[test]
    fn fade_reaches_zero_at_both_edges() {
        let output = trim_and_fade(&vec![0.5; 100], 1_000, -45.0, 0.0, 0.01);
        assert_eq!(output.first(), Some(&0.0));
        assert_eq!(output.last(), Some(&0.0));
    }

    #[test]
    fn wsola_bypasses_near_unity_and_caps_extreme_compression() {
        let samples = sine_tone(48_000, 2.0, 440.0, 0.5);
        assert_eq!(wsola_time_stretch(&samples, samples.len(), 48_000), samples);

        let near_unity_len = (samples.len() as f64 / 1.02).round() as usize;
        assert_eq!(
            wsola_time_stretch(&samples, near_unity_len, 48_000),
            samples
        );

        let extreme_target = samples.len() / 2;
        let capped = wsola_time_stretch(&samples, extreme_target, 48_000);
        assert_eq!(
            capped.len(),
            (samples.len() as f64 / MAX_COMPRESSION_RATIO).ceil() as usize
        );
    }

    #[test]
    fn wsola_preserves_sine_level_and_limits_harmonics_at_1_15x() {
        let sample_rate = 48_000;
        let frequency = 440.0;
        let samples = sine_tone(sample_rate, 4.0, frequency, 0.5);
        let target_len = (samples.len() as f64 / 1.15).round() as usize;
        let stretched = wsola_time_stretch(&samples, target_len, sample_rate);
        assert_eq!(stretched.len(), target_len);

        let edge = sample_rate / 10;
        let stable = &stretched[edge..stretched.len() - edge];
        let block_rms: Vec<f32> = stable.chunks_exact(sample_rate / 100).map(rms).collect();
        let min_rms = block_rms.iter().copied().fold(f32::INFINITY, f32::min);
        let max_rms = block_rms.iter().copied().fold(0.0, f32::max);
        let mean_rms = rms(stable);
        assert!(
            (max_rms - min_rms) / mean_rms < 0.20,
            "unexpected 10 ms RMS ripple: min={min_rms}, max={max_rms}, mean={mean_rms}"
        );

        let fundamental = amplitude_at(stable, frequency, sample_rate);
        let harmonic_energy = (2..=6)
            .map(|harmonic| amplitude_at(stable, frequency * harmonic as f32, sample_rate).powi(2))
            .sum::<f32>()
            .sqrt();
        assert!(
            harmonic_energy / fundamental < 0.15,
            "unexpected harmonic energy ratio: {}",
            harmonic_energy / fundamental
        );
    }

    #[test]
    fn butterworth_filters_have_minus_three_db_gain_at_cutoff() {
        let sample_rate = 48_000;
        let frequency = 1_000.0;
        let samples = sine_tone(sample_rate, 1.0, frequency, 0.5);
        let skip = sample_rate / 10;
        let lowpass = gentle_lowpass(&samples, sample_rate, frequency);
        let highpass = highpass_filter(&samples, sample_rate, frequency);
        let input_rms = rms(&samples[skip..]);
        let lowpass_gain = rms(&lowpass[skip..]) / input_rms;
        let highpass_gain = rms(&highpass[skip..]) / input_rms;
        assert!(
            (0.68..=0.73).contains(&lowpass_gain),
            "lowpass gain={lowpass_gain}"
        );
        assert!(
            (0.68..=0.73).contains(&highpass_gain),
            "highpass gain={highpass_gain}"
        );
    }

    #[test]
    fn pcm_conversion_soft_limits_without_integer_wraparound() {
        let converted = f32_to_i16_pcm(&[-2.0, -1.0, -0.98, 0.0, 0.98, 1.0, 2.0]);
        assert!(
            converted
                .iter()
                .all(|sample| (-32767..=32767).contains(sample))
        );
        assert!(converted[0] < 0);
        assert!(converted[6] > 0);
        assert_eq!(converted[0], -converted[6]);
        assert!(converted[4] > 0 && converted[4] < 32767);
        assert!(converted[5] < 32767);
    }

    fn sine_tone(
        sample_rate: usize,
        duration_sec: f32,
        frequency: f32,
        amplitude: f32,
    ) -> Vec<f32> {
        let len = (sample_rate as f32 * duration_sec) as usize;
        (0..len)
            .map(|index| {
                amplitude
                    * (std::f32::consts::TAU * frequency * index as f32 / sample_rate as f32).sin()
            })
            .collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
    }

    fn amplitude_at(samples: &[f32], frequency: f32, sample_rate: usize) -> f32 {
        let (mut sin_sum, mut cos_sum) = (0.0, 0.0);
        for (index, sample) in samples.iter().enumerate() {
            let phase = std::f32::consts::TAU * frequency * index as f32 / sample_rate as f32;
            sin_sum += sample * phase.sin();
            cos_sum += sample * phase.cos();
        }
        2.0 * (sin_sum * sin_sum + cos_sum * cos_sum).sqrt() / samples.len() as f32
    }
}
