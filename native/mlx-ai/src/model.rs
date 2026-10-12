//! Qwen3.5 text model (hybrid Gated DeltaNet + gated full attention), ported from
//! mlx_lm/models/qwen3_5.py and qwen3_next.py. Weights are MLX-quantized safetensors.
//! Derived from mlx-lm (Copyright © 2023 Apple Inc., MIT License); see THIRD_PARTY_NOTICES.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use mlx_rs::fast::{self, ScaledDotProductAttentionMask};
use mlx_rs::nn::{silu, softplus};
use mlx_rs::ops::{
    concatenate, conv1d, dequantize, quantized_matmul, sigmoid, split_at_indices, zeros_dtype,
};
use mlx_rs::{Array, Dtype};
use serde_json::Value;

use crate::kernel::{GatedDeltaKernel, gated_delta_ops};

pub struct Config {
    #[allow(dead_code)]
    hidden_size: i32,
    num_layers: usize,
    num_heads: i32,
    num_kv_heads: i32,
    head_dim: i32,
    lin_k_heads: i32,
    lin_v_heads: i32,
    lin_k_dim: i32,
    lin_v_dim: i32,
    conv_kernel: i32,
    full_attention_interval: usize,
    rms_eps: f32,
    rope_theta: f32,
    rope_dims: i32,
    tie_embeddings: bool,
    group_size: i32,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let root: Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
        let t = root.get("text_config").unwrap_or(&root);
        let int = |k: &str| {
            t[k].as_i64()
                .with_context(|| format!("config missing {k}"))
                .map(|v| v as i32)
        };
        let rope = &t["rope_parameters"];
        let head_dim = int("head_dim")?;
        let partial = rope["partial_rotary_factor"].as_f64().unwrap_or(0.25);
        Ok(Self {
            hidden_size: int("hidden_size")?,
            num_layers: int("num_hidden_layers")? as usize,
            num_heads: int("num_attention_heads")?,
            num_kv_heads: int("num_key_value_heads")?,
            head_dim,
            lin_k_heads: int("linear_num_key_heads")?,
            lin_v_heads: int("linear_num_value_heads")?,
            lin_k_dim: int("linear_key_head_dim")?,
            lin_v_dim: int("linear_value_head_dim")?,
            conv_kernel: int("linear_conv_kernel_dim")?,
            full_attention_interval: int("full_attention_interval")? as usize,
            rms_eps: t["rms_norm_eps"].as_f64().unwrap_or(1e-6) as f32,
            rope_theta: rope["rope_theta"].as_f64().unwrap_or(1e7) as f32,
            rope_dims: (head_dim as f64 * partial) as i32,
            tie_embeddings: t["tie_word_embeddings"].as_bool().unwrap_or(false),
            group_size: root["quantization"]["group_size"].as_i64().unwrap_or(64) as i32,
        })
    }
}

/// Multiply a scalar into `x` without promoting bf16 activations to f32.
fn scale(x: &Array, s: f32) -> Result<Array> {
    Ok(x.multiply(Array::from_f32(s).as_dtype(x.dtype())?)?)
}

struct Weights(HashMap<String, Array>);

impl Weights {
    fn take(&mut self, key: &str) -> Result<Array> {
        self.0
            .remove(key)
            .with_context(|| format!("missing weight {key}"))
    }
}

struct QLinear {
    w: Array,
    scales: Array,
    biases: Array,
    group_size: i32,
    bits: i32,
}

impl QLinear {
    fn load(ws: &mut Weights, prefix: &str, group_size: i32) -> Result<Self> {
        let w = ws.take(&format!("{prefix}.weight"))?;
        let scales = ws.take(&format!("{prefix}.scales"))?;
        let biases = ws.take(&format!("{prefix}.biases"))?;
        // packed uint32 columns * 32 bits == input features * bits
        let bits = w.dim(-1) * 32 / (scales.dim(-1) * group_size);
        Ok(Self {
            w,
            scales,
            biases,
            group_size,
            bits,
        })
    }

    fn forward(&self, x: &Array) -> Result<Array> {
        Ok(quantized_matmul(
            x,
            &self.w,
            &self.scales,
            &self.biases,
            true,
            self.group_size,
            self.bits,
        )?)
    }

    /// Embedding lookup: gather the quantized rows then dequantize only those.
    fn embed(&self, ids: &Array) -> Result<Array> {
        let w = self.w.take_axis(ids, 0)?;
        let s = self.scales.take_axis(ids, 0)?;
        let b = self.biases.take_axis(ids, 0)?;
        Ok(dequantize(&w, &s, &b, self.group_size, self.bits)?)
    }
}

struct Mlp {
    gate: QLinear,
    up: QLinear,
    down: QLinear,
}

impl Mlp {
    fn forward(&self, x: &Array) -> Result<Array> {
        let h = silu(self.gate.forward(x)?)?.multiply(self.up.forward(x)?)?;
        self.down.forward(&h)
    }
}

struct Attention {
    q_proj: QLinear,
    k_proj: QLinear,
    v_proj: QLinear,
    o_proj: QLinear,
    q_norm: Array,
    k_norm: Array,
}

struct GatedDeltaNet {
    in_proj_qkv: QLinear,
    in_proj_z: QLinear,
    in_proj_b: QLinear,
    in_proj_a: QLinear,
    out_proj: QLinear,
    conv1d: Array,
    a_log: Array,
    dt_bias: Array,
    norm: Array,
}

enum Mixer {
    Linear(GatedDeltaNet),
    Full(Attention),
}

struct Layer {
    mixer: Mixer,
    input_norm: Array,
    post_norm: Array,
    mlp: Mlp,
}

/// Per-layer state: conv + recurrent state for linear layers, KV for attention layers.
pub enum LayerCache {
    Linear {
        conv: Option<Array>,
        state: Option<Array>,
    },
    Full {
        keys: Option<Array>,
        values: Option<Array>,
        offset: i32,
    },
}

pub struct Model {
    cfg: Config,
    embed: QLinear,
    layers: Vec<Layer>,
    norm: Array,
    lm_head: Option<QLinear>,
    kernel: Option<GatedDeltaKernel>,
}

impl Model {
    pub fn load(dir: &Path, use_kernel: bool) -> Result<Self> {
        let cfg = Config::load(&dir.join("config.json"))?;
        let mut map = HashMap::new();
        for entry in std::fs::read_dir(dir)? {
            let p = entry?.path();
            if p.extension().is_some_and(|e| e == "safetensors") {
                map.extend(Array::load_safetensors(&p)?);
            }
        }
        // Keep only the language model; drop the vision tower.
        map.retain(|k, _| k.starts_with("language_model."));
        let mut ws = Weights(map);
        let gs = cfg.group_size;
        let p = "language_model.model";

        let embed = QLinear::load(&mut ws, &format!("{p}.embed_tokens"), gs)?;
        let mut layers = Vec::with_capacity(cfg.num_layers);
        for i in 0..cfg.num_layers {
            let lp = format!("{p}.layers.{i}");
            let is_linear = (i + 1) % cfg.full_attention_interval != 0;
            let mixer = if is_linear {
                let a = format!("{lp}.linear_attn");
                Mixer::Linear(GatedDeltaNet {
                    in_proj_qkv: QLinear::load(&mut ws, &format!("{a}.in_proj_qkv"), gs)?,
                    in_proj_z: QLinear::load(&mut ws, &format!("{a}.in_proj_z"), gs)?,
                    in_proj_b: QLinear::load(&mut ws, &format!("{a}.in_proj_b"), gs)?,
                    in_proj_a: QLinear::load(&mut ws, &format!("{a}.in_proj_a"), gs)?,
                    out_proj: QLinear::load(&mut ws, &format!("{a}.out_proj"), gs)?,
                    conv1d: ws.take(&format!("{a}.conv1d.weight"))?,
                    a_log: ws.take(&format!("{a}.A_log"))?,
                    dt_bias: ws.take(&format!("{a}.dt_bias"))?,
                    norm: ws.take(&format!("{a}.norm.weight"))?,
                })
            } else {
                let a = format!("{lp}.self_attn");
                Mixer::Full(Attention {
                    q_proj: QLinear::load(&mut ws, &format!("{a}.q_proj"), gs)?,
                    k_proj: QLinear::load(&mut ws, &format!("{a}.k_proj"), gs)?,
                    v_proj: QLinear::load(&mut ws, &format!("{a}.v_proj"), gs)?,
                    o_proj: QLinear::load(&mut ws, &format!("{a}.o_proj"), gs)?,
                    q_norm: ws.take(&format!("{a}.q_norm.weight"))?,
                    k_norm: ws.take(&format!("{a}.k_norm.weight"))?,
                })
            };
            layers.push(Layer {
                mixer,
                input_norm: ws.take(&format!("{lp}.input_layernorm.weight"))?,
                post_norm: ws.take(&format!("{lp}.post_attention_layernorm.weight"))?,
                mlp: Mlp {
                    gate: QLinear::load(&mut ws, &format!("{lp}.mlp.gate_proj"), gs)?,
                    up: QLinear::load(&mut ws, &format!("{lp}.mlp.up_proj"), gs)?,
                    down: QLinear::load(&mut ws, &format!("{lp}.mlp.down_proj"), gs)?,
                },
            });
        }
        let norm = ws.take(&format!("{p}.norm.weight"))?;
        let lm_head = if cfg.tie_embeddings {
            None
        } else {
            Some(QLinear::load(&mut ws, "language_model.lm_head", gs)?)
        };

        let model = Self {
            cfg,
            embed,
            layers,
            norm,
            lm_head,
            kernel: use_kernel.then(GatedDeltaKernel::new),
        };
        model.eval_params()?;
        Ok(model)
    }

    /// Materialize all weights on the GPU up front so the first token isn't slow.
    fn eval_params(&self) -> Result<()> {
        let mut all: Vec<&Array> = vec![
            &self.embed.w,
            &self.embed.scales,
            &self.embed.biases,
            &self.norm,
        ];
        if let Some(h) = &self.lm_head {
            all.extend([&h.w, &h.scales, &h.biases]);
        }
        for l in &self.layers {
            all.extend([&l.input_norm, &l.post_norm]);
            let mut qls: Vec<&QLinear> = vec![&l.mlp.gate, &l.mlp.up, &l.mlp.down];
            match &l.mixer {
                Mixer::Linear(m) => {
                    qls.extend([
                        &m.in_proj_qkv,
                        &m.in_proj_z,
                        &m.in_proj_b,
                        &m.in_proj_a,
                        &m.out_proj,
                    ]);
                    all.extend([&m.conv1d, &m.a_log, &m.dt_bias, &m.norm]);
                }
                Mixer::Full(a) => {
                    qls.extend([&a.q_proj, &a.k_proj, &a.v_proj, &a.o_proj]);
                    all.extend([&a.q_norm, &a.k_norm]);
                }
            }
            for ql in qls {
                all.extend([&ql.w, &ql.scales, &ql.biases]);
            }
        }
        mlx_rs::transforms::eval(all)?;
        Ok(())
    }

    pub fn make_cache(&self) -> Vec<LayerCache> {
        self.layers
            .iter()
            .map(|l| match l.mixer {
                Mixer::Linear(_) => LayerCache::Linear {
                    conv: None,
                    state: None,
                },
                Mixer::Full(_) => LayerCache::Full {
                    keys: None,
                    values: None,
                    offset: 0,
                },
            })
            .collect()
    }

    /// tokens: [1, S] int32. Returns logits for the last position: [1, vocab].
    pub fn forward(&self, tokens: &Array, cache: &mut [LayerCache]) -> Result<Array> {
        let mut h = self.embed.embed(tokens)?;
        for (layer, c) in self.layers.iter().zip(cache.iter_mut()) {
            let x = fast::rms_norm(&h, Some(&layer.input_norm), self.cfg.rms_eps)?;
            let r = match (&layer.mixer, c) {
                (Mixer::Linear(m), LayerCache::Linear { conv, state }) => {
                    self.linear_attn(m, &x, conv, state)?
                }
                (
                    Mixer::Full(a),
                    LayerCache::Full {
                        keys,
                        values,
                        offset,
                    },
                ) => self.full_attn(a, &x, keys, values, offset)?,
                _ => unreachable!("cache/layer type mismatch"),
            };
            h = h.add(r)?;
            let x = fast::rms_norm(&h, Some(&layer.post_norm), self.cfg.rms_eps)?;
            h = h.add(layer.mlp.forward(&x)?)?;
        }
        // Only the last position feeds the (large) vocab projection.
        let s = h.dim(1);
        let last = if s > 1 {
            split_at_indices(&h, &[s - 1], 1)?.pop().unwrap()
        } else {
            h
        };
        let last = fast::rms_norm(&last, Some(&self.norm), self.cfg.rms_eps)?;
        let logits = match &self.lm_head {
            Some(head) => head.forward(&last)?,
            None => quantized_matmul(
                &last,
                &self.embed.w,
                &self.embed.scales,
                &self.embed.biases,
                true,
                self.embed.group_size,
                self.embed.bits,
            )?,
        };
        Ok(logits.reshape(&[1, -1])?)
    }

    fn linear_attn(
        &self,
        m: &GatedDeltaNet,
        x: &Array,
        conv_cache: &mut Option<Array>,
        state_cache: &mut Option<Array>,
    ) -> Result<Array> {
        let c = &self.cfg;
        let (b, s) = (x.dim(0), x.dim(1));
        let key_dim = c.lin_k_heads * c.lin_k_dim;
        let value_dim = c.lin_v_heads * c.lin_v_dim;
        let conv_dim = key_dim * 2 + value_dim;
        let n_keep = c.conv_kernel - 1;

        let qkv = m.in_proj_qkv.forward(x)?;
        let z = m
            .in_proj_z
            .forward(x)?
            .reshape(&[b, s, c.lin_v_heads, c.lin_v_dim])?;
        let beta_in = m.in_proj_b.forward(x)?;
        let a = m.in_proj_a.forward(x)?;

        let conv_state = match conv_cache.take() {
            Some(cs) => cs,
            None => zeros_dtype(&[b, n_keep, conv_dim], x.dtype())?,
        };
        let conv_input = concatenate(&[conv_state, qkv], 1)?;
        let total = conv_input.dim(1);
        *conv_cache = Some(
            split_at_indices(&conv_input, &[total - n_keep], 1)?
                .pop()
                .unwrap()
                .contiguous()?,
        );
        let conv_out = silu(conv1d(&conv_input, &m.conv1d, 1, 0, 1, conv_dim)?)?;

        let parts = split_at_indices(&conv_out, &[key_dim, 2 * key_dim], -1)?;
        let q = parts[0].reshape(&[b, s, c.lin_k_heads, c.lin_k_dim])?;
        let k = parts[1].reshape(&[b, s, c.lin_k_heads, c.lin_k_dim])?;
        let v = parts[2].reshape(&[b, s, c.lin_v_heads, c.lin_v_dim])?;

        let inv_scale = (c.lin_k_dim as f32).powf(-0.5);
        let q = scale(&fast::rms_norm(&q, None, 1e-6)?, inv_scale * inv_scale)?;
        let k = scale(&fast::rms_norm(&k, None, 1e-6)?, inv_scale)?;

        let beta = sigmoid(&beta_in)?;
        // g = exp(-exp(A_log) * softplus(a + dt_bias)), computed in f32
        let decay = m.a_log.as_dtype(Dtype::Float32)?.exp()?;
        let g = decay
            .multiply(softplus(a.add(&m.dt_bias)?)?)?
            .negative()?
            .exp()?;

        let state = match state_cache.take() {
            Some(st) => st,
            None => zeros_dtype(
                &[b, c.lin_v_heads, c.lin_v_dim, c.lin_k_dim],
                Dtype::Float32,
            )?,
        };
        let (out, state) = match &self.kernel {
            Some(kern) => kern.apply(&q, &k, &v, &g, &beta, &state)?,
            None => gated_delta_ops(&q, &k, &v, &g, &beta, state)?,
        };
        *state_cache = Some(state);

        // Gated RMSNorm: rms_norm(out) * silu(z), in f32
        let normed = fast::rms_norm(&out, Some(&m.norm), c.rms_eps)?;
        let gated = silu(z.as_dtype(Dtype::Float32)?)?
            .multiply(normed.as_dtype(Dtype::Float32)?)?
            .as_dtype(out.dtype())?;
        m.out_proj.forward(&gated.reshape(&[b, s, -1])?)
    }

    fn full_attn(
        &self,
        a: &Attention,
        x: &Array,
        k_cache: &mut Option<Array>,
        v_cache: &mut Option<Array>,
        offset: &mut i32,
    ) -> Result<Array> {
        let c = &self.cfg;
        let (b, l) = (x.dim(0), x.dim(1));

        let qg = a
            .q_proj
            .forward(x)?
            .reshape(&[b, l, c.num_heads, 2 * c.head_dim])?;
        let mut qg = split_at_indices(&qg, &[c.head_dim], -1)?;
        let gate = qg.pop().unwrap().reshape(&[b, l, -1])?;
        let queries = qg.pop().unwrap();
        let keys = a
            .k_proj
            .forward(x)?
            .reshape(&[b, l, c.num_kv_heads, c.head_dim])?;
        let values = a
            .v_proj
            .forward(x)?
            .reshape(&[b, l, c.num_kv_heads, c.head_dim])?;

        let queries =
            fast::rms_norm(&queries, Some(&a.q_norm), c.rms_eps)?.transpose_axes(&[0, 2, 1, 3])?;
        let keys =
            fast::rms_norm(&keys, Some(&a.k_norm), c.rms_eps)?.transpose_axes(&[0, 2, 1, 3])?;
        let values = values.transpose_axes(&[0, 2, 1, 3])?;

        let queries = fast::rope(
            &queries,
            c.rope_dims,
            false,
            c.rope_theta,
            1.0,
            *offset,
            None,
        )?;
        let keys = fast::rope(&keys, c.rope_dims, false, c.rope_theta, 1.0, *offset, None)?;

        let keys = match k_cache.take() {
            Some(prev) => concatenate(&[prev, keys], 2)?,
            None => keys,
        };
        let values = match v_cache.take() {
            Some(prev) => concatenate(&[prev, values], 2)?,
            None => values,
        };
        *offset += l;

        let scale = (c.head_dim as f32).powf(-0.5);
        let mask = (l > 1).then_some(ScaledDotProductAttentionMask::Causal);
        let out = fast::scaled_dot_product_attention(&queries, &keys, &values, scale, mask, None)?;
        *k_cache = Some(keys);
        *v_cache = Some(values);

        let out = out.transpose_axes(&[0, 2, 1, 3])?.reshape(&[b, l, -1])?;
        a.o_proj.forward(&out.multiply(sigmoid(&gate)?)?)
    }
}
