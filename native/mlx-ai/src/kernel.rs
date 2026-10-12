//! Gated DeltaNet recurrence as a custom Metal kernel (port of mlx_lm/models/gated_delta.py).
//! mlx-rs has no safe wrapper for `mx.fast.metal_kernel`, so this goes through mlx-sys.
//! Derived from mlx-lm (Copyright © 2023 Apple Inc., MIT License); see THIRD_PARTY_NOTICES.

use std::ffi::CString;

use anyhow::{Result, bail};
use mlx_rs::{Array, Stream};
use mlx_sys as sys;

const SOURCE: &str = r#"
    auto n = thread_position_in_grid.z;
    auto b_idx = n / Hv;
    auto hv_idx = n % Hv;
    auto hk_idx = hv_idx / (Hv / Hk);
    constexpr int n_per_t = Dk / 32;

    // q, k: [B, T, Hk, Dk]
    auto q_ = q + b_idx * T * Hk * Dk + hk_idx * Dk;
    auto k_ = k + b_idx * T * Hk * Dk + hk_idx * Dk;

    // v, y: [B, T, Hv, Dv]
    auto v_ = v + b_idx * T * Hv * Dv + hv_idx * Dv;
    y += b_idx * T * Hv * Dv + hv_idx * Dv;

    auto dk_idx = thread_position_in_threadgroup.x;
    auto dv_idx = thread_position_in_grid.y;

    // state_in, state_out: [B, Hv, Dv, Dk]
    auto i_state = state_in + (n * Dv + dv_idx) * Dk;
    auto o_state = state_out + (n * Dv + dv_idx) * Dk;

    float state[n_per_t];
    for (int i = 0; i < n_per_t; ++i) {
      auto s_idx = n_per_t * dk_idx + i;
      state[i] = static_cast<float>(i_state[s_idx]);
    }

    // g: [B, T, Hv]
    auto g_ = g + b_idx * T * Hv;
    auto beta_ = beta + b_idx * T * Hv;

    for (int t = 0; t < T; ++t) {
      float kv_mem = 0.0f;
      for (int i = 0; i < n_per_t; ++i) {
        auto s_idx = n_per_t * dk_idx + i;
        state[i] = state[i] * g_[hv_idx];
        kv_mem += state[i] * k_[s_idx];
      }
      kv_mem = simd_sum(kv_mem);

      auto delta = (v_[dv_idx] - kv_mem) * beta_[hv_idx];

      float out = 0.0f;
      for (int i = 0; i < n_per_t; ++i) {
        auto s_idx = n_per_t * dk_idx + i;
        state[i] = state[i] + k_[s_idx] * delta;
        out += state[i] * q_[s_idx];
      }
      out = simd_sum(out);
      if (thread_index_in_simdgroup == 0) {
        y[dv_idx] = static_cast<InT>(out);
      }
      q_ += Hk * Dk;
      k_ += Hk * Dk;
      v_ += Hv * Dv;
      y += Hv * Dv;
      g_ += Hv;
      beta_ += Hv;
    }
    for (int i = 0; i < n_per_t; ++i) {
      auto s_idx = n_per_t * dk_idx + i;
      o_state[s_idx] = static_cast<StT>(state[i]);
    }
"#;

fn check(rc: i32, what: &str) -> Result<()> {
    if rc != 0 {
        bail!("mlx-c call failed: {what}");
    }
    Ok(())
}

fn vec_string(names: &[&str]) -> sys::mlx_vector_string {
    unsafe {
        let v = sys::mlx_vector_string_new();
        for n in names {
            let c = CString::new(*n).unwrap();
            sys::mlx_vector_string_append_value(v, c.as_ptr());
        }
        v
    }
}

pub struct GatedDeltaKernel {
    kernel: sys::mlx_fast_metal_kernel,
}

impl GatedDeltaKernel {
    pub fn new() -> Self {
        let name = CString::new("gated_delta_step_rs").unwrap();
        let source = CString::new(SOURCE).unwrap();
        let header = CString::new("").unwrap();
        unsafe {
            let inputs = vec_string(&["q", "k", "v", "g", "beta", "state_in", "T"]);
            let outputs = vec_string(&["y", "state_out"]);
            let kernel = sys::mlx_fast_metal_kernel_new(
                name.as_ptr(),
                inputs,
                outputs,
                source.as_ptr(),
                header.as_ptr(),
                true,
                false,
            );
            sys::mlx_vector_string_free(inputs);
            sys::mlx_vector_string_free(outputs);
            Self { kernel }
        }
    }

    /// q, k: [B, T, Hk, Dk]; v: [B, T, Hv, Dv]; g, beta: [B, T, Hv] (f32); state: [B, Hv, Dv, Dk] (f32).
    /// Returns (y: [B, T, Hv, Dv], new_state).
    #[allow(clippy::too_many_arguments)]
    pub fn apply(
        &self,
        q: &Array,
        k: &Array,
        v: &Array,
        g: &Array,
        beta: &Array,
        state: &Array,
    ) -> Result<(Array, Array)> {
        let (b, t, hk, dk) = (k.dim(0), k.dim(1), k.dim(2), k.dim(3));
        let (hv, dv) = (v.dim(2), v.dim(3));
        let in_t = q.dtype();
        let st_t = state.dtype();
        let t_arr = Array::from_int(t);
        let stream = Stream::thread_local_or_default();

        unsafe {
            let cfg = sys::mlx_fast_metal_kernel_config_new();
            let set = |name: &str, val: i32| {
                let c = CString::new(name).unwrap();
                sys::mlx_fast_metal_kernel_config_add_template_arg_int(cfg, c.as_ptr(), val)
            };
            let in_name = CString::new("InT").unwrap();
            let st_name = CString::new("StT").unwrap();
            check(
                sys::mlx_fast_metal_kernel_config_add_template_arg_dtype(
                    cfg,
                    in_name.as_ptr(),
                    in_t as _,
                ),
                "template InT",
            )?;
            check(
                sys::mlx_fast_metal_kernel_config_add_template_arg_dtype(
                    cfg,
                    st_name.as_ptr(),
                    st_t as _,
                ),
                "template StT",
            )?;
            for (n, val) in [("Dk", dk), ("Dv", dv), ("Hk", hk), ("Hv", hv)] {
                check(set(n, val), "template int")?;
            }
            check(
                sys::mlx_fast_metal_kernel_config_set_grid(cfg, 32, dv, b * hv),
                "grid",
            )?;
            check(
                sys::mlx_fast_metal_kernel_config_set_thread_group(cfg, 32, 4, 1),
                "threadgroup",
            )?;
            let y_shape = [b, t, hv, dv];
            check(
                sys::mlx_fast_metal_kernel_config_add_output_arg(
                    cfg,
                    y_shape.as_ptr(),
                    4,
                    in_t as _,
                ),
                "output y",
            )?;
            let s_shape = state.shape().to_vec();
            check(
                sys::mlx_fast_metal_kernel_config_add_output_arg(
                    cfg,
                    s_shape.as_ptr(),
                    s_shape.len(),
                    st_t as _,
                ),
                "output state",
            )?;

            let inputs = sys::mlx_vector_array_new();
            for a in [q, k, v, g, beta, state, &t_arr] {
                sys::mlx_vector_array_append_value(inputs, a.as_ptr());
            }
            let mut outputs = sys::mlx_vector_array_new();
            let rc = sys::mlx_fast_metal_kernel_apply(
                &mut outputs,
                self.kernel,
                inputs,
                cfg,
                stream.as_ptr(),
            );
            sys::mlx_vector_array_free(inputs);
            sys::mlx_fast_metal_kernel_config_free(cfg);
            check(rc, "metal kernel apply")?;

            let mut y = sys::mlx_array_new();
            let mut s = sys::mlx_array_new();
            sys::mlx_vector_array_get(&mut y, outputs, 0);
            sys::mlx_vector_array_get(&mut s, outputs, 1);
            sys::mlx_vector_array_free(outputs);
            Ok((Array::from_ptr(y), Array::from_ptr(s)))
        }
    }
}

impl Drop for GatedDeltaKernel {
    fn drop(&mut self) {
        unsafe { sys::mlx_fast_metal_kernel_free(self.kernel) };
    }
}

/// Reference ops implementation (same math, one step at a time). Used by `--no-kernel`.
pub fn gated_delta_ops(
    q: &Array,
    k: &Array,
    v: &Array,
    g: &Array,
    beta: &Array,
    mut state: Array,
) -> Result<(Array, Array)> {
    use mlx_rs::ops::{repeat_axis, split_equal, stack};
    let (t, hk) = (q.dim(1), q.dim(2));
    let hv = v.dim(2);
    let rep = hv / hk;
    let (q, k) = if rep > 1 {
        (
            repeat_axis::<f32>(q.clone(), rep, 2)?,
            repeat_axis::<f32>(k.clone(), rep, 2)?,
        )
    } else {
        (q.clone(), k.clone())
    };
    let qs = split_equal(&q, t, 1)?;
    let ks = split_equal(&k, t, 1)?;
    let vs = split_equal(v, t, 1)?;
    let gs = split_equal(g, t, 1)?;
    let bs = split_equal(beta, t, 1)?;
    let mut ys = Vec::with_capacity(t as usize);
    for i in 0..t as usize {
        // squeeze time axis: q,k [B,H,Dk], v [B,H,Dv], g,beta [B,H]
        let sq = |a: &Array| -> Result<Array> {
            let mut s = a.shape().to_vec();
            s.remove(1);
            Ok(a.reshape(&s)?)
        };
        let (qt, kt, vt, gt, bt) = (
            sq(&qs[i])?,
            sq(&ks[i])?,
            sq(&vs[i])?,
            sq(&gs[i])?,
            sq(&bs[i])?,
        );
        state = state.multiply(gt.expand_dims_axes(&[-1, -2])?)?;
        let k_e = kt.expand_dims(-2)?; // [B,H,1,Dk]
        let kv_mem = state.multiply(&k_e)?.sum_axis(-1, None)?; // [B,H,Dv]
        let delta = vt.subtract(&kv_mem)?.multiply(bt.expand_dims(-1)?)?;
        state = state.add(k_e.multiply(delta.expand_dims(-1)?)?)?;
        let y = state.multiply(qt.expand_dims(-2)?)?.sum_axis(-1, None)?;
        ys.push(y.as_dtype(q.dtype())?);
    }
    Ok((stack(&ys, 1)?, state))
}
