/****
 * [ATOM]: v3turbo_engine
 * Chức năng: Bộ suy luận Native In-Process cho VieNeu-TTS v3 Turbo ONNX INT8 (48kHz)
 * Quản lý 4 ONNX Session: Prefill, DecodeStep, AcousticCached, và MOSS Audio Codec Decoder.
 * Ranh giới bảo vệ: Tách biệt hoàn toàn (Isolation) khỏi engine Nano cũ, xuất PCM 48kHz nguyên bản.
 ****/
use crate::log_event;
use anyhow::{Result, anyhow};
use ndarray::{Array1, Array2, Array3, ArrayD};
use ndarray_npy::NpzReader;
use ort::{inputs, session::Session, session::builder::GraphOptimizationLevel, value::Tensor};
use rand::thread_rng;
use rand_distr::{Distribution, Uniform};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use tokenizers::Tokenizer;
use vieneu_tts_core::events::EventCallback;

#[derive(Debug, Deserialize, Clone)]
pub struct PresetVoiceData {
    pub description: Option<String>,
    pub gender: Option<String>,
    pub speaker_emb: Option<Vec<f32>>,
    pub codes: Option<Vec<Vec<i64>>>,
}

#[derive(Debug, Deserialize)]
struct VoicesJson {
    default_voice: Option<String>,
    presets: HashMap<String, PresetVoiceData>,
}

pub struct VietNEUTurboEngine {
    // ONNX Sessions
    s_pre: Session,
    s_dec: Session,
    s_ac: Session,
    s_codec_dec: Session,

    // Tokenizer
    tokenizer: Tokenizer,

    // Heads & Projections từ vieneu_v3_heads.npz
    text_emb: Array2<f32>,  // [419, 768]
    audio_emb: Array3<f32>, // [16, 1024, 768]
    xvec_w: Array2<f32>,    // [768, 192]
    xvec_b: Array1<f32>,    // [768]
    xvec_ln_w: Array1<f32>, // [768]
    xvec_ln_b: Array1<f32>, // [768]
    xvec_ln_eps: f32,

    // Config tokens
    hidden_size: usize,
    num_layers: usize,
    audio_pad: i64,
    tps: i64,
    tpe: i64,
    sgs: i64,
    eos_speech: i64,
    ref_slot: i64,
    default_style_id: i64,
    n_vq: usize,

    // Presets
    pub default_voice: String,
    pub voices: HashMap<String, PresetVoiceData>,
}

impl VietNEUTurboEngine {
    pub fn new(
        model_dir: &Path,
        threads: usize,
        use_gpu: bool,
        events: &Option<EventCallback>,
    ) -> Result<Self> {
        #[cfg(windows)]
        if use_gpu {
            log_event(events, "Đang thử DirectML execution provider.");
        }

        #[cfg(not(windows))]
        if use_gpu {
            log_event(events, "DirectML chỉ khả dụng trên Windows; dùng CPU.");
        }

        let build_session = |filename: &str| -> Result<Session> {
            let model_path = model_dir.join(filename);

            // Thử khởi tạo với DirectML EP nếu GPU khả dụng
            if use_gpu {
                let dml_ep = ort::execution_providers::DirectMLExecutionProvider::default().build();
                let dml_session = Session::builder()
                    .map_err(|e| anyhow!("{:?}", e))?
                    .with_optimization_level(GraphOptimizationLevel::Level1)
                    .map_err(|e| anyhow!("{:?}", e))?
                    .with_execution_providers([dml_ep])
                    .map_err(|e| anyhow!("{:?}", e))?
                    .commit_from_file(&model_path);

                match dml_session {
                    Ok(s) => return Ok(s),
                    Err(err) => {
                        // WHY: Graceful degradation đảm bảo pipeline không bao giờ bị dừng đột ngột
                        log_event(
                            events,
                            format!(
                                "DirectML không thể nạp session '{filename}' ({err:?}); fallback về CPU."
                            ),
                        );
                    }
                }
            }

            // Fallback CPU Execution Provider
            let builder = Session::builder().map_err(|e| anyhow!("{:?}", e))?;
            let builder = builder
                .with_optimization_level(GraphOptimizationLevel::Level1)
                .map_err(|e| anyhow!("{:?}", e))?
                .with_intra_threads(threads)
                .map_err(|e| anyhow!("{:?}", e))?;
            builder
                .commit_from_file(&model_path)
                .map_err(|e| anyhow!("Không thể tải session ONNX '{}': {:?}", filename, e))
        };

        log_event(events, "Khởi tạo ONNX sessions cho v3-Turbo INT8.");
        let s_pre = build_session("vieneu_prefill.onnx")?;
        let s_dec = build_session("vieneu_decode_step.onnx")?;
        let s_ac = build_session("vieneu_acoustic_cached.onnx")?;
        let s_codec_dec = build_session("moss_audio_tokenizer_decode_full.onnx")?;

        // Nạp tokenizer
        let tokenizer_path = model_dir.join("tokenizer.json");
        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| anyhow!("Không thể tải tokenizer.json: {:?}", e))?;

        // Nạp config.json
        let config_path = model_dir.join("config.json");
        let config_val: serde_json::Value = serde_json::from_reader(File::open(config_path)?)?;
        let hidden_size = config_val["hidden_size"].as_u64().unwrap_or(768) as usize;
        let num_layers = config_val["num_hidden_layers"].as_u64().unwrap_or(12) as usize;
        let audio_pad = config_val["audio_pad_token_id"].as_i64().unwrap_or(1024);
        let tps = config_val["text_prompt_start_token_id"]
            .as_i64()
            .unwrap_or(3);
        let tpe = config_val["text_prompt_end_token_id"].as_i64().unwrap_or(4);
        let sgs = config_val["speech_generation_start_token_id"]
            .as_i64()
            .unwrap_or(5);
        let eos_speech = config_val["speech_generation_end_token_id"]
            .as_i64()
            .unwrap_or(6);
        let ref_slot = config_val["audio_ref_slot_token_id"].as_i64().unwrap_or(7);
        let default_style_id = config_val["default_style_token_id"].as_i64().unwrap_or(16);
        let n_vq = config_val["n_vq"].as_u64().unwrap_or(16) as usize;

        // Nạp vieneu_v3_heads.npz
        log_event(events, "Nạp embeddings và projection heads.");
        let mut npz = NpzReader::new(File::open(model_dir.join("vieneu_v3_heads.npz"))?)?;
        let text_emb: Array2<f32> = npz.by_name("text_emb.npy")?;
        let audio_emb: Array3<f32> = npz.by_name("audio_emb.npy")?;
        let xvec_w: Array2<f32> = npz.by_name("xvec_w.npy")?;
        let xvec_b: Array1<f32> = npz.by_name("xvec_b.npy")?;
        let xvec_ln_w: Array1<f32> = npz.by_name("xvec_ln_w.npy")?;
        let xvec_ln_b: Array1<f32> = npz.by_name("xvec_ln_b.npy")?;
        let xvec_ln_eps_arr: ArrayD<f32> = npz.by_name("xvec_ln_eps.npy")?;
        let xvec_ln_eps = *xvec_ln_eps_arr.first().unwrap_or(&1e-5);

        // Nạp voices_v3_turbo.json
        let voices_path = model_dir.join("voices_v3_turbo.json");
        let (default_voice, voices) = if voices_path.exists() {
            let file = File::open(&voices_path)?;
            let v_json: VoicesJson = serde_json::from_reader(file)?;
            (
                v_json
                    .default_voice
                    .unwrap_or_else(|| "Trúc Ly".to_string()),
                v_json.presets,
            )
        } else {
            ("Trúc Ly".to_string(), HashMap::new())
        };

        log_event(
            events,
            format!(
                "V3-Turbo sẵn sàng với {} giọng (mặc định: '{}').",
                voices.len(),
                default_voice
            ),
        );

        Ok(Self {
            s_pre,
            s_dec,
            s_ac,
            s_codec_dec,
            tokenizer,
            text_emb,
            audio_emb,
            xvec_w,
            xvec_b,
            xvec_ln_w,
            xvec_ln_b,
            xvec_ln_eps,
            hidden_size,
            num_layers,
            audio_pad,
            tps,
            tpe,
            sgs,
            eos_speech,
            ref_slot,
            default_style_id,
            n_vq,
            default_voice,
            voices,
        })
    }

    /// Tính speaker anchor từ 192-d x-vector: x @ W.T + b -> LayerNorm
    fn compute_speaker_anchor(&self, spk_emb: &[f32]) -> Result<Vec<f32>> {
        if spk_emb.len() != 192 {
            return Err(anyhow!(
                "Độ dài speaker_emb phải là 192, nhận được: {}",
                spk_emb.len()
            ));
        }
        let h = self.hidden_size;
        let mut v = vec![0.0f32; h];

        // Linear: v = xvec_w (h, 192) * spk_emb (192) + xvec_b (h)
        for (i, output) in v.iter_mut().enumerate() {
            let mut sum = self.xvec_b[i];
            for (j, &embedding) in spk_emb.iter().enumerate() {
                sum += self.xvec_w[[i, j]] * embedding;
            }
            *output = sum;
        }

        // LayerNorm: (v - mean) / sqrt(var + eps) * ln_w + ln_b
        let mean = v.iter().sum::<f32>() / (h as f32);
        let var = v.iter().map(|&x| (x - mean) * (x - mean)).sum::<f32>() / (h as f32);
        let std = (var + self.xvec_ln_eps).sqrt();

        for (i, output) in v.iter_mut().enumerate() {
            let norm = (*output - mean) / std;
            *output = norm * self.xvec_ln_w[i] + self.xvec_ln_b[i];
        }

        Ok(v)
    }

    /// Tạo ma trận prompt embedding [1, T, hidden_size]
    fn build_prompt_embeds(
        &self,
        phonemes: &str,
        ref_codes: Option<&[Vec<i64>]>,
        anchor: Option<&[f32]>,
    ) -> Result<(Array3<f32>, usize)> {
        let encoding = self
            .tokenizer
            .encode(phonemes, false)
            .map_err(|e| anyhow!("Tokenize phonemes thất bại: {:?}", e))?;
        let phone_ids = encoding.get_ids();

        let mut text_ids = vec![self.default_style_id, self.tps];
        for &id in phone_ids {
            text_ids.push(id as i64);
        }
        text_ids.push(self.tpe);

        let t_text = text_ids.len();
        let t_ref = ref_codes.map(|c| c.len()).unwrap_or(0);
        let total_t = t_text + t_ref;
        let h = self.hidden_size;

        // Ma trận embedding [1, total_t, h]
        let mut embeds = Array3::<f32>::zeros((1, total_t, h));

        // 1. Nhúng phần text tokens
        for (t, &token_id) in text_ids.iter().enumerate() {
            let tok_idx = token_id as usize;
            for d in 0..h {
                let mut val = self.text_emb[[tok_idx, d]];
                if let Some(anc) = anchor {
                    val += anc[d];
                }
                embeds[[0, t, d]] = val;
            }
        }

        // 2. Nhúng phần reference codes (nếu có)
        if let Some(codes) = ref_codes {
            for (i, frame_codes) in codes.iter().enumerate() {
                let t = t_text + i;
                // Vị trí text token là ref_slot
                let ref_slot_idx = self.ref_slot as usize;
                for d in 0..h {
                    let mut val = self.text_emb[[ref_slot_idx, d]];
                    // Cộng dồn các codebooks
                    for (ch, &code) in frame_codes.iter().take(self.n_vq).enumerate() {
                        if code != self.audio_pad && code >= 0 && (code as usize) < 1024 {
                            val += self.audio_emb[[ch, code as usize, d]];
                        }
                    }
                    if let Some(anc) = anchor {
                        val += anc[d];
                    }
                    embeds[[0, t, d]] = val;
                }
            }
        }

        Ok((embeds, total_t))
    }

    /// Lấy mẫu sampling (top_k, top_p, temperature)
    fn sample_token(logits: &[f32], temperature: f32, top_k: usize, top_p: f32) -> usize {
        if temperature <= 0.0 {
            return logits
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
                .map(|(idx, _)| idx)
                .unwrap_or(0);
        }

        let mut indexed_logits: Vec<(usize, f32)> = logits
            .iter()
            .enumerate()
            .map(|(i, &v)| (i, v / temperature))
            .collect();

        // Top-k filtering
        if top_k > 0 && top_k < indexed_logits.len() {
            indexed_logits
                .select_nth_unstable_by(top_k, |(_, a), (_, b)| b.partial_cmp(a).unwrap());
            indexed_logits.truncate(top_k);
        }

        // Sort descending
        indexed_logits.sort_by(|(_, a), (_, b)| b.partial_cmp(a).unwrap());

        // Softmax & Top-p filtering
        let max_val = indexed_logits[0].1;
        let mut exps: Vec<f32> = indexed_logits
            .iter()
            .map(|(_, l)| (l - max_val).exp())
            .collect();
        let sum_exp: f32 = exps.iter().sum();
        for e in &mut exps {
            *e /= sum_exp;
        }

        let mut cumsum = 0.0;
        let mut filtered: Vec<(usize, f32)> = Vec::new();
        for (idx, &(orig_i, _)) in indexed_logits.iter().enumerate() {
            let p = exps[idx];
            filtered.push((orig_i, p));
            cumsum += p;
            if cumsum >= top_p {
                break;
            }
        }

        // Re-normalize probabilities
        let sum_filtered: f32 = filtered.iter().map(|(_, p)| p).sum();
        let mut rng = thread_rng();
        let r: f32 = Uniform::new(0.0, sum_filtered).sample(&mut rng);

        let mut acc = 0.0;
        for (orig_i, p) in filtered {
            acc += p;
            if acc >= r {
                return orig_i;
            }
        }

        indexed_logits[0].0
    }

    /// Tổng hợp giọng nói Native Rust theo Preset: Trả về PCM 48.000 Hz (f32 mono)
    pub fn synthesize(&mut self, phonemes: &str, voice_name: &str) -> Result<Vec<f32>> {
        let (spk_emb, ref_codes) = {
            let voice_entry = self
                .voices
                .get(voice_name)
                .or_else(|| self.voices.get(&self.default_voice))
                .ok_or_else(|| anyhow!("Không tìm thấy preset giọng: {}", voice_name))?;
            (voice_entry.speaker_emb.clone(), voice_entry.codes.clone())
        };

        self.synthesize_advanced(phonemes, spk_emb.as_deref(), ref_codes.as_deref())
    }

    /// [CHỐT MỞ RỘNG]: API suy luận cấp cao, nhận trực tiếp speaker_emb và ref_codes
    /// Sẵn sàng kích hoạt tính năng Zero-shot Voice Cloning trong tương lai mà không cần đổi core logic.
    pub fn synthesize_advanced(
        &mut self,
        phonemes: &str,
        speaker_emb: Option<&[f32]>,
        ref_codes: Option<&[Vec<i64>]>,
    ) -> Result<Vec<f32>> {
        let anchor = if let Some(spk_emb) = speaker_emb {
            Some(self.compute_speaker_anchor(spk_emb)?)
        } else {
            None
        };

        // 1. Build prompt embeddings [1, T, 768]
        let (prompt_embeds, prompt_len) =
            self.build_prompt_embeds(phonemes, ref_codes, anchor.as_deref())?;

        let prompt_shape = vec![1i64, prompt_len as i64, self.hidden_size as i64];
        let prompt_flat: Vec<f32> = prompt_embeds.iter().copied().collect();
        let prompt_tensor =
            Tensor::from_array((prompt_shape, prompt_flat)).map_err(|e| anyhow!("{:?}", e))?;

        // 2. Chạy Prefill Session
        let (current_h, mut past_k, mut past_v) = {
            let pre_outputs = self
                .s_pre
                .run(inputs![
                    "inputs_embeds" => prompt_tensor
                ])
                .map_err(|e| anyhow!("Lỗi chạy prefill session: {:?}", e))?;

            let (_h_shape, h_slice) = pre_outputs[0]
                .try_extract_tensor::<f32>()
                .map_err(|e| anyhow!("{:?}", e))?;

            // Lấy vector hidden của token cuối cùng: [1, hidden_size]
            let current_h = h_slice[h_slice.len() - self.hidden_size..].to_vec();

            // Trích xuất KV cache từ prefill
            let mut past_k: Vec<Tensor<f32>> = Vec::new();
            let mut past_v: Vec<Tensor<f32>> = Vec::new();

            for i in 0..self.num_layers {
                let (k_shape, k_slice) = pre_outputs[1 + i]
                    .try_extract_tensor::<f32>()
                    .map_err(|e| anyhow!("{:?}", e))?;
                let (v_shape, v_slice) = pre_outputs[1 + self.num_layers + i]
                    .try_extract_tensor::<f32>()
                    .map_err(|e| anyhow!("{:?}", e))?;
                past_k.push(
                    Tensor::from_array((k_shape.to_vec(), k_slice.to_vec()))
                        .map_err(|e| anyhow!("{:?}", e))?,
                );
                past_v.push(
                    Tensor::from_array((v_shape.to_vec(), v_slice.to_vec()))
                        .map_err(|e| anyhow!("{:?}", e))?,
                );
            }
            (current_h, past_k, past_v)
        };
        let mut current_h = current_h;

        // 3. Vòng lặp Autoregressive sinh frames (Tối đa 300 frames ~ 24s)
        let max_frames = 300;
        let mut generated_frames: Vec<Vec<i32>> = Vec::new();

        for t in 0..max_frames {
            // A. Bước 1: Chạy Acoustic Cached Decoder để sinh 16 codebook tokens
            let (frame_codes, is_eos) = self.acoustic_step(&current_h)?;
            generated_frames.push(frame_codes.clone());

            if is_eos {
                break;
            }

            // B. Bước 2: Chạy Decode Step Session để bước tiếp một frame mới
            let mut step_embed = vec![0.0f32; self.hidden_size];
            // SGS token embedding
            let sgs_idx = self.sgs as usize;
            for (d, value) in step_embed.iter_mut().enumerate() {
                *value += self.text_emb[[sgs_idx, d]];
            }
            // Codebook embeddings
            for (ch, &code) in frame_codes.iter().take(self.n_vq).enumerate() {
                if code >= 0 && (code as usize) < 1024 {
                    for (d, value) in step_embed.iter_mut().enumerate() {
                        *value += self.audio_emb[[ch, code as usize, d]];
                    }
                }
            }
            if let Some(ref anc) = anchor {
                for (value, anchor_value) in step_embed.iter_mut().zip(anc) {
                    *value += anchor_value;
                }
            }

            let step_embed_tensor =
                Tensor::from_array((vec![1i64, 1i64, self.hidden_size as i64], step_embed))
                    .map_err(|e| anyhow!("{:?}", e))?;
            let pos_val = (prompt_len + t) as i64;
            let pos_tensor = Tensor::from_array((vec![1i64, 1i64], vec![pos_val]))
                .map_err(|e| anyhow!("{:?}", e))?;

            // Chuẩn bị feed inputs cho Decode Step
            let mut dec_inputs = inputs![
                "inputs_embeds" => step_embed_tensor,
                "position_ids" => pos_tensor
            ];

            for (i, past) in past_k.iter().enumerate() {
                let pk_name = format!("past_k_{}", i);
                dec_inputs.push((pk_name.into(), past.into()));
            }
            for (i, past) in past_v.iter().enumerate() {
                let pv_name = format!("past_v_{}", i);
                dec_inputs.push((pv_name.into(), past.into()));
            }

            let dec_out = self
                .s_dec
                .run(dec_inputs)
                .map_err(|e| anyhow!("Lỗi chạy decode_step session tại frame {}: {:?}", t, e))?;

            let (_dh_shape, dh_slice) = dec_out[0]
                .try_extract_tensor::<f32>()
                .map_err(|e| anyhow!("{:?}", e))?;
            current_h = dh_slice.to_vec();

            // Cập nhật KV cache
            for i in 0..self.num_layers {
                let (k_shape, k_slice) = dec_out[1 + i]
                    .try_extract_tensor::<f32>()
                    .map_err(|e| anyhow!("{:?}", e))?;
                let (v_shape, v_slice) = dec_out[1 + self.num_layers + i]
                    .try_extract_tensor::<f32>()
                    .map_err(|e| anyhow!("{:?}", e))?;
                past_k[i] = Tensor::from_array((k_shape.to_vec(), k_slice.to_vec()))
                    .map_err(|e| anyhow!("{:?}", e))?;
                past_v[i] = Tensor::from_array((v_shape.to_vec(), v_slice.to_vec()))
                    .map_err(|e| anyhow!("{:?}", e))?;
            }
        }

        if generated_frames.is_empty() {
            return Ok(Vec::new());
        }

        // 4. Giải mã toàn bộ frames qua MOSS Audio Codec Decoder (48kHz)
        let total_frames = generated_frames.len();
        let mut flat_codes = Vec::with_capacity(total_frames * self.n_vq);
        for frame in &generated_frames {
            for &code in frame.iter().take(self.n_vq) {
                flat_codes.push(code);
            }
        }

        let codes_tensor = Tensor::from_array((
            vec![1i64, total_frames as i64, self.n_vq as i64],
            flat_codes,
        ))
        .map_err(|e| anyhow!("{:?}", e))?;

        let lens_tensor = Tensor::from_array((vec![1i64], vec![total_frames as i32]))
            .map_err(|e| anyhow!("{:?}", e))?;

        let codec_out = self
            .s_codec_dec
            .run(inputs![
                "audio_codes" => codes_tensor,
                "audio_code_lengths" => lens_tensor
            ])
            .map_err(|e| anyhow!("Lỗi chạy MOSS Audio Codec decoder: {:?}", e))?;

        let (audio_shape, audio_slice) = codec_out[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| anyhow!("{:?}", e))?;

        // audio_shape: [1, 2, n_samples] -> Lấy trung bình 2 kênh để ra mono 48kHz
        let n_samples = if audio_shape.len() == 3 {
            audio_shape[2] as usize
        } else {
            audio_slice.len()
        };
        let mut mono_audio = Vec::with_capacity(n_samples);

        if audio_shape.len() == 3 && audio_shape[1] == 2 {
            let ch0 = &audio_slice[0..n_samples];
            let ch1 = &audio_slice[n_samples..2 * n_samples];
            for i in 0..n_samples {
                mono_audio.push((ch0[i] + ch1[i]) * 0.5);
            }
        } else {
            mono_audio.extend_from_slice(&audio_slice[0..n_samples]);
        }

        Ok(mono_audio)
    }

    /// Chạy 1 bước acoustic decoder để sinh 16 codes và kiểm tra EOS
    fn acoustic_step(&mut self, h: &[f32]) -> Result<(Vec<i32>, bool)> {
        let mut token_emb = Vec::with_capacity(2 * self.hidden_size);
        token_emb.extend_from_slice(h);
        let sgs_idx = self.sgs as usize;
        for d in 0..self.hidden_size {
            token_emb.push(self.text_emb[[sgs_idx, d]]);
        }

        let tok_tensor = Tensor::from_array((vec![1i64, 2i64, self.hidden_size as i64], token_emb))
            .map_err(|e| anyhow!("Lỗi tạo tok_tensor: {:?}", e))?;

        let pos_tensor = Tensor::from_array((vec![1i64, 2i64], vec![0i64, 1i64]))
            .map_err(|e| anyhow!("Lỗi tạo pos_tensor: {:?}", e))?;

        // Past KV rỗng ban đầu [1, 8, 0, 96] cấp phát qua Allocator::default()
        let alloc = ort::memory::Allocator::default();
        let empty_past_k = Tensor::<f32>::new(&alloc, [1usize, 8, 0, 96])
            .map_err(|e| anyhow!("Lỗi tạo empty_past_k: {:?}", e))?;
        let empty_past_v = Tensor::<f32>::new(&alloc, [1usize, 8, 0, 96])
            .map_err(|e| anyhow!("Lỗi tạo empty_past_v: {:?}", e))?;

        let ac_inputs = inputs![
            "token_emb" => tok_tensor,
            "position_ids" => pos_tensor,
            "past_k_0" => empty_past_k,
            "past_v_0" => empty_past_v
        ];

        let (slot0, slot1, mut pk_shape, mut pk_slice, mut pv_shape, mut pv_slice) = {
            let out = self
                .s_ac
                .run(ac_inputs)
                .map_err(|e| anyhow!("Lỗi chạy acoustic session: {:?}", e))?;

            let (_hid_shape, hid_slice) = out[0]
                .try_extract_tensor::<f32>()
                .map_err(|e| anyhow!("{:?}", e))?;
            let (pk_s, pk_d) = out[1]
                .try_extract_tensor::<f32>()
                .map_err(|e| anyhow!("{:?}", e))?;
            let (pv_s, pv_d) = out[2]
                .try_extract_tensor::<f32>()
                .map_err(|e| anyhow!("{:?}", e))?;

            let slot0 = hid_slice[0..self.hidden_size].to_vec();
            let slot1 = hid_slice[self.hidden_size..2 * self.hidden_size].to_vec();

            (
                slot0,
                slot1,
                pk_s.to_vec(),
                pk_d.to_vec(),
                pv_s.to_vec(),
                pv_d.to_vec(),
            )
        };

        // Lấy code 0 từ slot1 chiếu lên audio_emb[0]
        let mut codes: Vec<i32> = Vec::with_capacity(self.n_vq);
        {
            let mut logits = vec![0.0f32; 1024];
            for (c, logit) in logits.iter_mut().enumerate() {
                *logit = slot1
                    .iter()
                    .enumerate()
                    .map(|(d, value)| value * self.audio_emb[[0, c, d]])
                    .sum();
            }
            let code0 = Self::sample_token(&logits, 0.8, 25, 0.95);
            codes.push(code0 as i32);
        }

        // Lặp qua 15 codebook còn lại
        for ch in 1..self.n_vq {
            let prev_code = codes[ch - 1] as usize;
            let mut single_emb = vec![0.0f32; self.hidden_size];
            for (d, value) in single_emb.iter_mut().enumerate() {
                *value = self.audio_emb[[ch - 1, prev_code, d]];
            }

            let s_tok = Tensor::from_array((vec![1i64, 1i64, self.hidden_size as i64], single_emb))
                .map_err(|e| anyhow!("{:?}", e))?;
            let s_pos = Tensor::from_array((vec![1i64, 1i64], vec![(ch + 1) as i64]))
                .map_err(|e| anyhow!("{:?}", e))?;
            let s_pk = Tensor::from_array((pk_shape.clone(), pk_slice.clone()))
                .map_err(|e| anyhow!("{:?}", e))?;
            let s_pv = Tensor::from_array((pv_shape.clone(), pv_slice.clone()))
                .map_err(|e| anyhow!("{:?}", e))?;

            let sub_h_slice = {
                let sub_out = self
                    .s_ac
                    .run(inputs![
                        "token_emb" => s_tok,
                        "position_ids" => s_pos,
                        "past_k_0" => s_pk,
                        "past_v_0" => s_pv
                    ])
                    .map_err(|e| anyhow!("Lỗi chạy acoustic sub-step {}: {:?}", ch, e))?;

                let (_sub_h_shape, sub_h_slice) = sub_out[0]
                    .try_extract_tensor::<f32>()
                    .map_err(|e| anyhow!("{:?}", e))?;
                let (new_pk_s, new_pk_d) = sub_out[1]
                    .try_extract_tensor::<f32>()
                    .map_err(|e| anyhow!("{:?}", e))?;
                let (new_pv_s, new_pv_d) = sub_out[2]
                    .try_extract_tensor::<f32>()
                    .map_err(|e| anyhow!("{:?}", e))?;
                pk_shape = new_pk_s.to_vec();
                pk_slice = new_pk_d.to_vec();
                pv_shape = new_pv_s.to_vec();
                pv_slice = new_pv_d.to_vec();
                sub_h_slice.to_vec()
            };

            let mut logits = vec![0.0f32; 1024];
            for (c, logit) in logits.iter_mut().enumerate() {
                *logit = sub_h_slice
                    .iter()
                    .enumerate()
                    .map(|(d, value)| value * self.audio_emb[[ch, c, d]])
                    .sum();
            }
            let next_code = Self::sample_token(&logits, 0.8, 25, 0.95);
            codes.push(next_code as i32);
        }

        // Kiểm tra EOS: slot0 chiếu lên text_emb
        let num_text_vocab = self.text_emb.shape()[0];
        let mut max_idx = 0;
        let mut max_val = f32::NEG_INFINITY;
        for c in 0..num_text_vocab {
            let dot = slot0
                .iter()
                .enumerate()
                .map(|(d, value)| value * self.text_emb[[c, d]])
                .sum::<f32>();
            if dot > max_val {
                max_val = dot;
                max_idx = c;
            }
        }
        let is_eos = max_idx as i64 == self.eos_speech;

        Ok((codes, is_eos))
    }
}
