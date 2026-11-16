/****
 * [ATOM]: inference
 * Chức năng: Quản lý 4 ONNX Session & thuật toán Flow-Matching (Euler ODE Solver)
 * Chạy Native In-Process 100% trong Rust, triệt tiêu hoàn toàn Subprocess CLI overhead
 ****/
use anyhow::{Result, bail};
use ndarray::{Array1, Array2};
use ndarray_npy::NpzReader;
use ort::{
    init, inputs, session::Session, session::builder::GraphOptimizationLevel, value::Tensor,
};
use rand::thread_rng;
use rand_distr::{Distribution, StandardNormal};
use std::fs::File;
use std::path::Path;

pub struct VietNEUEngine {
    s_text: Session,
    s_dur: Session,
    s_ve: Session,
    s_dec: Session,
    null_ctx: Vec<f32>,
    null_ctx_shape: Vec<i64>,
    null_mask: Vec<bool>,
    null_mask_shape: Vec<i64>,
    null_spk: Vec<f32>,
    null_spk_shape: Vec<i64>,
    null_style: Vec<f32>,
    null_style_shape: Vec<i64>,
    fps: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct NanoSynthesisConfig {
    pub speed: f32,
    pub pad_id: i64,
    pub steps: usize,
    pub guidance: f32,
    pub sway: f32,
}

impl VietNEUEngine {
    pub fn new(
        model_dir: &Path,
        threads: usize,
        bos: i64,
        eos: i64,
        pad: i64,
        fps: f32,
    ) -> Result<Self> {
        if threads == 0 {
            bail!("Số luồng ONNX phải lớn hơn 0");
        }
        let _ = init().with_name("aidub_vietneu").commit();

        let build_session = |name: &str| -> Result<Session> {
            let builder = Session::builder().map_err(|e| anyhow::anyhow!("{:?}", e))?;
            let builder = builder
                .with_optimization_level(GraphOptimizationLevel::Disable)
                .map_err(|e| anyhow::anyhow!("{:?}", e))?
                .with_intra_threads(threads)
                .map_err(|e| anyhow::anyhow!("{:?}", e))?;
            builder
                .commit_from_file(model_dir.join(name))
                .map_err(|e| anyhow::anyhow!("Không thể tải model ONNX '{}': {:?}", name, e))
        };

        let mut s_text = build_session("text_encoder.onnx")?;
        let s_dur = build_session("duration_predictor.onnx")?;
        let s_ve = build_session("vector_estimator.onnx")?;
        let s_dec = build_session("codec_decoder.onnx")?;

        // Nạp constants.npz
        let mut npz = NpzReader::new(File::open(model_dir.join("constants.npz"))?)?;
        let null_spk_1d: Array1<f32> = npz.by_name("null_spk.npy")?;
        let null_style_2d: Array2<f32> = npz.by_name("null_style.npy")?;
        let null_spk: Vec<f32> = null_spk_1d.iter().copied().collect();
        let null_style: Vec<f32> = null_style_2d.iter().copied().collect();

        let null_ids: Vec<i64> = vec![bos, eos];
        let null_ids_shape = vec![1, 2];
        let null_mask: Vec<bool> = null_ids.iter().map(|&x| x != pad).collect();
        let null_mask_shape = vec![1, 2];

        let null_ids_tensor = Tensor::from_array((null_ids_shape, null_ids))
            .map_err(|e| anyhow::anyhow!("{:?}", e))?;

        let null_style_shape = vec![1, 50, 256];
        let null_style_tensor = Tensor::from_array((null_style_shape.clone(), null_style.clone()))
            .map_err(|e| anyhow::anyhow!("{:?}", e))?;

        let null_ctx_shape: Vec<i64>;
        let null_ctx: Vec<f32>;
        {
            let outputs = s_text
                .run(inputs![
                    "ids" => null_ids_tensor,
                    "style" => null_style_tensor
                ])
                .map_err(|e| anyhow::anyhow!("{:?}", e))?;

            let (shape, slice) = outputs[0]
                .try_extract_tensor::<f32>()
                .map_err(|e| anyhow::anyhow!("{:?}", e))?;
            null_ctx_shape = shape.to_vec();
            null_ctx = slice.to_vec();
        }

        Ok(Self {
            s_text,
            s_dur,
            s_ve,
            s_dec,
            null_ctx,
            null_ctx_shape,
            null_mask,
            null_mask_shape,
            null_spk,
            null_spk_shape: vec![1, 192],
            null_style,
            null_style_shape,
            fps,
        })
    }

    pub fn synthesize(
        &mut self,
        input_ids: Vec<i64>,
        spk: Vec<f32>,
        style: Vec<f32>,
        config: NanoSynthesisConfig,
    ) -> Result<Vec<f32>> {
        if config.steps == 0 {
            bail!("Số bước suy luận Nano phải lớn hơn 0");
        }
        let seq_len = input_ids.len() as i64;
        let ids_shape = vec![1, seq_len];
        let mask: Vec<bool> = input_ids.iter().map(|&x| x != config.pad_id).collect();
        let mask_shape = vec![1, seq_len];
        let spk_shape = vec![1, 192];
        let style_shape = vec![1, 50, 256];

        // Chạy text encoder để lấy context biểu diễn âm vị
        let ctx_outputs = self
            .s_text
            .run(inputs![
                "ids" => Tensor::from_array((ids_shape, input_ids))
                    .map_err(|e| anyhow::anyhow!("{:?}", e))?,
                "style" => Tensor::from_array((style_shape.clone(), style.clone()))
                    .map_err(|e| anyhow::anyhow!("{:?}", e))?
            ])
            .map_err(|e| anyhow::anyhow!("{:?}", e))?;

        let (ctx_shape, ctx_slice) = ctx_outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| anyhow::anyhow!("{:?}", e))?;
        let ctx_shape = ctx_shape.to_vec();
        let ctx = ctx_slice.to_vec();

        // Dự đoán thời lượng phát âm (duration prediction)
        // LƯU Ý BẢO VỆ CON TRỎ BỘ NHỚ: Tuyệt đối không gọi .clone() trên ort::value::Tensor
        // vì OrtApi::CloneValue trên Windows x64 gây STATUS_ACCESS_VIOLATION (0xc0000005)
        // khi chia sẻ bộ nhớ giữa các session ONNX. Cấp phát fresh Tensor từ slice chỉ tốn <1µs.
        let dur_outputs = self
            .s_dur
            .run(inputs![
                "ctx" => Tensor::from_array((ctx_shape.clone(), ctx.clone()))
                    .map_err(|e| anyhow::anyhow!("{:?}", e))?,
                "ctx_mask" => Tensor::from_array((mask_shape.clone(), mask.clone()))
                    .map_err(|e| anyhow::anyhow!("{:?}", e))?,
                "spk" => Tensor::from_array((spk_shape.clone(), spk.clone()))
                    .map_err(|e| anyhow::anyhow!("{:?}", e))?
            ])
            .map_err(|e| anyhow::anyhow!("{:?}", e))?;
        let (_dur_shape, dur_slice) = dur_outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| anyhow::anyhow!("{:?}", e))?;
        let log_s = dur_slice[0];

        let spd = if config.speed < 1e-3 {
            1e-3
        } else {
            config.speed
        };
        let secs = (log_s.exp() / spd).min(15.0);
        let t_frames = 2.max((secs * self.fps).round() as i64);

        let mut rng = thread_rng();
        let x_shape = vec![1, 144, t_frames];
        let total_x_elems = (144 * t_frames) as usize;
        let mut x: Vec<f32> = Vec::with_capacity(total_x_elems);
        for _ in 0..total_x_elems {
            x.push(StandardNormal.sample(&mut rng));
        }

        // Sinh mốc thời gian thời gian giải Euler ODE tích hợp sway curve
        let mut tg = vec![0.0f32; config.steps + 1];
        for (i, time) in tg.iter_mut().enumerate() {
            let u = i as f32 / config.steps as f32;
            *time = u + config.sway * ((std::f32::consts::PI / 2.0 * u).cos() - 1.0 + u);
        }

        // Vòng lặp giải phương trình vi phân Flow-Matching ODE
        for i in 0..config.steps {
            let t_val = tg[i];

            let mut v: Vec<f32>;
            {
                let ve_outputs = self
                    .s_ve
                    .run(inputs![
                        "x" => Tensor::from_array((x_shape.clone(), x.clone()))
                            .map_err(|e| anyhow::anyhow!("{:?}", e))?,
                        "t" => Tensor::from_array((vec![1], vec![t_val]))
                            .map_err(|e| anyhow::anyhow!("{:?}", e))?,
                        "ctx" => Tensor::from_array((ctx_shape.clone(), ctx.clone()))
                            .map_err(|e| anyhow::anyhow!("{:?}", e))?,
                        "ctx_mask" => Tensor::from_array((mask_shape.clone(), mask.clone()))
                            .map_err(|e| anyhow::anyhow!("{:?}", e))?,
                        "spk" => Tensor::from_array((spk_shape.clone(), spk.clone()))
                            .map_err(|e| anyhow::anyhow!("{:?}", e))?,
                        "style" => Tensor::from_array((style_shape.clone(), style.clone()))
                            .map_err(|e| anyhow::anyhow!("{:?}", e))?
                    ])
                    .map_err(|e| anyhow::anyhow!("{:?}", e))?;
                let (_, v_slice) = ve_outputs[0]
                    .try_extract_tensor::<f32>()
                    .map_err(|e| anyhow::anyhow!("{:?}", e))?;
                v = v_slice.to_vec();
            }

            // Hướng dẫn phân loại không cần điều kiện (Classifier-Free Guidance - CFG)
            if config.guidance > 0.0 {
                let ve_null_outputs = self
                    .s_ve
                    .run(inputs![
                        "x" => Tensor::from_array((x_shape.clone(), x.clone()))
                            .map_err(|e| anyhow::anyhow!("{:?}", e))?,
                        "t" => Tensor::from_array((vec![1], vec![t_val]))
                            .map_err(|e| anyhow::anyhow!("{:?}", e))?,
                        "ctx" => Tensor::from_array((self.null_ctx_shape.clone(), self.null_ctx.clone()))
                            .map_err(|e| anyhow::anyhow!("{:?}", e))?,
                        "ctx_mask" => Tensor::from_array((self.null_mask_shape.clone(), self.null_mask.clone()))
                            .map_err(|e| anyhow::anyhow!("{:?}", e))?,
                        "spk" => Tensor::from_array((self.null_spk_shape.clone(), self.null_spk.clone()))
                            .map_err(|e| anyhow::anyhow!("{:?}", e))?,
                        "style" => Tensor::from_array((self.null_style_shape.clone(), self.null_style.clone()))
                            .map_err(|e| anyhow::anyhow!("{:?}", e))?
                    ])
                    .map_err(|e| anyhow::anyhow!("{:?}", e))?;
                let (_, vu_slice) = ve_null_outputs[0]
                    .try_extract_tensor::<f32>()
                    .map_err(|e| anyhow::anyhow!("{:?}", e))?;
                let vu = vu_slice.to_vec();

                for j in 0..v.len() {
                    v[j] = vu[j] + (v[j] - vu[j]) * config.guidance;
                }
            }

            let dt = tg[i + 1] - tg[i];
            for j in 0..x.len() {
                x[j] += v[j] * dt;
            }
        }

        // Giải mã Latent Representation thành sóng âm thanh hoàn chỉnh bằng Codec Decoder
        let dec_outputs = self
            .s_dec
            .run(inputs![
                "x" => Tensor::from_array((x_shape, x))
                    .map_err(|e| anyhow::anyhow!("{:?}", e))?
            ])
            .map_err(|e| anyhow::anyhow!("{:?}", e))?;
        let (_shape, slice) = dec_outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| anyhow::anyhow!("{:?}", e))?;

        let mut audio = slice.to_vec();
        for sample in &mut audio {
            if sample.is_nan() || sample.is_infinite() {
                *sample = 0.0;
                continue;
            }
            // Soft-Clipping (bo tròn sóng tanh bảo vệ thính giác và thiết bị ngoại vi)
            if *sample > 0.5 {
                *sample = 0.5 + (*sample - 0.5).tanh() * 0.5;
            } else if *sample < -0.5 {
                *sample = -0.5 + (*sample + 0.5).tanh() * 0.5;
            }
        }
        Ok(audio)
    }
}
