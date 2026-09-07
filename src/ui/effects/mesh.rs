//! Bicubic Hermite Patch (BHP) 网格系统 - 基于 实现
//!
//! 用于生成网格渐变背景

use rand::RngExt;

/// 控制点配置
#[derive(Debug, Clone, Copy)]
pub struct ControlPoint {
    /// 实际位置 x (-1.0 到 1.0)
    pub x: f32,
    /// 实际位置 y (-1.0 到 1.0)
    pub y: f32,
    /// U 方向切线旋转角度 (度)
    pub ur: f32,
    /// V 方向切线旋转角度 (度)
    pub vr: f32,
    /// U 方向切线长度倍数
    pub up: f32,
    /// V 方向切线长度倍数
    pub vp: f32,
}

impl ControlPoint {
    pub fn with_tangents(x: f32, y: f32, ur: f32, vr: f32, up: f32, vp: f32) -> Self {
        Self {
            x,
            y,
            ur,
            vr,
            up,
            vp,
        }
    }

    /// 计算 U 方向切线 (一致)
    pub fn u_tangent(&self, base_length: f32) -> (f32, f32) {
        let angle = self.ur.to_radians();
        let scale = base_length * self.up;
        (angle.cos() * scale, angle.sin() * scale)
    }

    /// 计算 V 方向切线 (一致)
    pub fn v_tangent(&self, base_length: f32) -> (f32, f32) {
        let angle = self.vr.to_radians();
        let scale = base_length * self.vp;
        (-angle.sin() * scale, angle.cos() * scale)
    }
}

/// 控制点预设
#[derive(Debug, Clone)]
pub struct ControlPointPreset {
    pub width: usize,
    pub height: usize,
    pub points: Vec<ControlPoint>,
}

impl ControlPointPreset {
    pub fn new(width: usize, height: usize, points: Vec<ControlPoint>) -> Self {
        Self {
            width,
            height,
            points,
        }
    }

    pub fn get(&self, x: usize, y: usize) -> Option<&ControlPoint> {
        if x < self.width && y < self.height {
            self.points.get(y * self.width + x)
        } else {
            None
        }
    }
}

/// Hermite 基矩阵 - 使用列优先存储（与 gl-matrix 一致）
///
/// default: H = Mat4.fromValues(2, -2, 1, 1, -3, 3, -2, -1, 0, 0, 1, 0, 1, 0, 0, 0)
/// 直接按 gl-matrix 的列优先格式存储为一维数组
const H: [f32; 16] = [
    2.0, -2.0, 1.0, 1.0, -3.0, 3.0, -2.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0,
];

struct SurfacePatch<'a> {
    corners: [&'a ControlPoint; 4],
    tangent_power: [f32; 2],
}

/// 计算 Hermite 曲面上的点
///
/// 矩阵布局与 meshCoefficients 完全一致：
/// G = [
///   [l(p00), l(p10), u(p00), u(p10)],
///   [l(p01), l(p11), u(p01), u(p11)],
///   [v(p00), v(p10), 0,      0     ],
///   [v(p01), v(p11), 0,      0     ],
/// ]
///
/// 其中 l = location, u = uTangent, v = vTangent
fn surface_point(u: f32, v: f32, patch: SurfacePatch<'_>) -> (f32, f32) {
    let [p00, p01, p10, p11] = patch.corners;
    let [u_power, v_power] = patch.tangent_power;
    let u_vec = [u * u * u, u * u, u, 1.0];
    let v_vec = [v * v * v, v * v, v, 1.0];

    let (u00x, u00y) = p00.u_tangent(u_power);
    let (u01x, u01y) = p01.u_tangent(u_power);
    let (u10x, u10y) = p10.u_tangent(u_power);
    let (u11x, u11y) = p11.u_tangent(u_power);
    let (v00x, v00y) = p00.v_tangent(v_power);
    let (v01x, v01y) = p01.v_tangent(v_power);
    let (v10x, v10y) = p10.v_tangent(v_power);
    let (v11x, v11y) = p11.v_tangent(v_power);

    // meshCoefficients 的矩阵布局 (gl-matrix 列优先转换为行优先)
    // 行0: [l(p00), l(p10), u(p00), u(p10)]
    // 行1: [l(p01), l(p11), u(p01), u(p11)]
    // 行2: [v(p00), v(p10), 0,      0     ]
    // 行3: [v(p01), v(p11), 0,      0     ]
    let gx = [
        [p00.x, p10.x, u00x, u10x],
        [p01.x, p11.x, u01x, u11x],
        [v00x, v10x, 0.0, 0.0],
        [v01x, v11x, 0.0, 0.0],
    ];

    let gy = [
        [p00.y, p10.y, u00y, u10y],
        [p01.y, p11.y, u01y, u11y],
        [v00y, v10y, 0.0, 0.0],
        [v01y, v11y, 0.0, 0.0],
    ];

    let x = hermite_surface_eval(&u_vec, &v_vec, &gx);
    let y = hermite_surface_eval(&u_vec, &v_vec, &gy);

    (x, y)
}

/// Hermite 曲面求值 - 完全模拟 gl-matrix 的列优先存储计算
///
/// surfacePoint 的计算流程:
/// 1. acc = G.transpose()
/// 2. acc = acc * H
/// 3. acc = H_T * acc
/// 4. result_u = acc * u (transformMat4)
/// 5. result = v · result_u
fn hermite_surface_eval(u: &[f32; 4], v: &[f32; 4], g: &[[f32; 4]; 4]) -> f32 {
    // 将 g (行优先 4x4) 转换为列优先一维数组
    let mut g_col = [0.0f32; 16];
    for row in 0..4 {
        for col in 0..4 {
            g_col[col * 4 + row] = g[row][col];
        }
    }

    // Step 1: transpose (gl-matrix 风格)
    let mut acc = glm_transpose(&g_col);

    // Step 2: acc = acc * H
    acc = glm_mul(&acc, &H);

    // Step 3: acc = H_T * acc
    let h_t = glm_transpose(&H);
    acc = glm_mul(&h_t, &acc);

    // Step 4: result_u = transformMat4(u, acc)
    let result_u = glm_transform_mat4(u, &acc);

    // Step 5: dot product
    v[0] * result_u[0] + v[1] * result_u[1] + v[2] * result_u[2] + v[3] * result_u[3]
}

/// gl-matrix 风格的矩阵转置 (列优先存储)
fn glm_transpose(m: &[f32; 16]) -> [f32; 16] {
    [
        m[0], m[4], m[8], m[12], m[1], m[5], m[9], m[13], m[2], m[6], m[10], m[14], m[3], m[7],
        m[11], m[15],
    ]
}

/// gl-matrix 风格的矩阵乘法 (列优先存储)
/// Mat4.mul(out, a, b) = a * b
fn glm_mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut out = [0.0f32; 16];

    let a00 = a[0];
    let a01 = a[1];
    let a02 = a[2];
    let a03 = a[3];
    let a10 = a[4];
    let a11 = a[5];
    let a12 = a[6];
    let a13 = a[7];
    let a20 = a[8];
    let a21 = a[9];
    let a22 = a[10];
    let a23 = a[11];
    let a30 = a[12];
    let a31 = a[13];
    let a32 = a[14];
    let a33 = a[15];

    // 列 0
    let b0 = b[0];
    let b1 = b[1];
    let b2 = b[2];
    let b3 = b[3];
    out[0] = b0 * a00 + b1 * a10 + b2 * a20 + b3 * a30;
    out[1] = b0 * a01 + b1 * a11 + b2 * a21 + b3 * a31;
    out[2] = b0 * a02 + b1 * a12 + b2 * a22 + b3 * a32;
    out[3] = b0 * a03 + b1 * a13 + b2 * a23 + b3 * a33;

    // 列 1
    let b0 = b[4];
    let b1 = b[5];
    let b2 = b[6];
    let b3 = b[7];
    out[4] = b0 * a00 + b1 * a10 + b2 * a20 + b3 * a30;
    out[5] = b0 * a01 + b1 * a11 + b2 * a21 + b3 * a31;
    out[6] = b0 * a02 + b1 * a12 + b2 * a22 + b3 * a32;
    out[7] = b0 * a03 + b1 * a13 + b2 * a23 + b3 * a33;

    // 列 2
    let b0 = b[8];
    let b1 = b[9];
    let b2 = b[10];
    let b3 = b[11];
    out[8] = b0 * a00 + b1 * a10 + b2 * a20 + b3 * a30;
    out[9] = b0 * a01 + b1 * a11 + b2 * a21 + b3 * a31;
    out[10] = b0 * a02 + b1 * a12 + b2 * a22 + b3 * a32;
    out[11] = b0 * a03 + b1 * a13 + b2 * a23 + b3 * a33;

    // 列 3
    let b0 = b[12];
    let b1 = b[13];
    let b2 = b[14];
    let b3 = b[15];
    out[12] = b0 * a00 + b1 * a10 + b2 * a20 + b3 * a30;
    out[13] = b0 * a01 + b1 * a11 + b2 * a21 + b3 * a31;
    out[14] = b0 * a02 + b1 * a12 + b2 * a22 + b3 * a32;
    out[15] = b0 * a03 + b1 * a13 + b2 * a23 + b3 * a33;

    out
}

/// gl-matrix 风格的 Vec4.transformMat4 (列优先存储)
/// out = m * v
fn glm_transform_mat4(v: &[f32; 4], m: &[f32; 16]) -> [f32; 4] {
    let x = v[0];
    let y = v[1];
    let z = v[2];
    let w = v[3];
    [
        m[0] * x + m[4] * y + m[8] * z + m[12] * w,
        m[1] * x + m[5] * y + m[9] * z + m[13] * w,
        m[2] * x + m[6] * y + m[10] * z + m[14] * w,
        m[3] * x + m[7] * y + m[11] * z + m[15] * w,
    ]
}

fn color_point(
    u: f32,
    v: f32,
    c00: [f32; 3],
    c01: [f32; 3],
    c10: [f32; 3],
    c11: [f32; 3],
) -> [f32; 3] {
    let u_vec = [u * u * u, u * u, u, 1.0];
    let v_vec = [v * v * v, v * v, v, 1.0];

    let mut result = [0.0f32; 3];
    for channel in 0..3 {
        // colorCoefficients 布局 (gl-matrix 列优先转行优先):
        // 行0: [c(p00), c(p10), 0, 0]
        // 行1: [c(p01), c(p11), 0, 0]
        let g = [
            [c00[channel], c10[channel], 0.0, 0.0],
            [c01[channel], c11[channel], 0.0, 0.0],
            [0.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 0.0],
        ];
        result[channel] = hermite_surface_eval(&u_vec, &v_vec, &g);
    }
    result
}

/// BHP 网格顶点数据
#[derive(Debug, Clone)]
pub struct MeshVertex {
    pub position: [f32; 2],
    pub color: [f32; 3],
    pub uv: [f32; 2],
}

/// BHP 网格
#[derive(Debug, Clone)]
pub struct BhpMesh {
    pub vertices: Vec<MeshVertex>,
    pub indices: Vec<u32>,
}

impl BhpMesh {
    /// 从控制点预设生成网格 (完全一致)
    ///
    /// 的实现方式：
    /// 1. 创建一个 vertexWidth x vertexHeight 的顶点网格
    /// 2. 对于每个控制点 patch (x, y)，计算其内部的细分顶点
    /// 3. 顶点按照 setVertexData(vx, vy) 存储，索引 = vx + vy * vertexWidth
    pub fn from_preset(
        preset: &ControlPointPreset,
        subdivisions: usize,
        colors: &[[f32; 3]],
    ) -> Self {
        let cp_w = preset.width;
        let cp_h = preset.height;

        let u_power = 2.0 / (cp_w - 1) as f32;
        let v_power = 2.0 / (cp_h - 1) as f32;

        // default: vertexWidth = (controlPointsWidth - 1) * subDivisions
        // default: vertexHeight = (controlPointsHeight - 1) * subDivisions
        let vertex_w = (cp_w - 1) * subdivisions;
        let vertex_h = (cp_h - 1) * subdivisions;

        // 预分配顶点数组
        let total_vertices = vertex_w * vertex_h;
        let mut vertices = vec![
            MeshVertex {
                position: [0.0, 0.0],
                color: [1.0, 1.0, 1.0],
                uv: [0.0, 0.0],
            };
            total_vertices
        ];

        let subdiv_m1 = (subdivisions - 1).max(1) as f32;
        let inv_subdiv_m1 = 1.0 / subdiv_m1;
        // default: tW = subDivM1 * (controlPointsHeight - 1)
        // default: tH = subDivM1 * (controlPointsWidth - 1)
        let inv_th = 1.0 / ((cp_w - 1) as f32 * subdiv_m1);
        let inv_tw = 1.0 / ((cp_h - 1) as f32 * subdiv_m1);

        // 按照 的方式生成顶点
        // default: 外层 x (0..controlPointsWidth-1)，内层 y (0..controlPointsHeight-1)
        for x in 0..(cp_w - 1) {
            for y in 0..(cp_h - 1) {
                let p00 = preset.get(x, y).unwrap();
                let p01 = preset.get(x, y + 1).unwrap();
                let p10 = preset.get(x + 1, y).unwrap();
                let p11 = preset.get(x + 1, y + 1).unwrap();

                let c00 = colors.get(y * cp_w + x).copied().unwrap_or([1.0, 1.0, 1.0]);
                let c01 = colors
                    .get((y + 1) * cp_w + x)
                    .copied()
                    .unwrap_or([1.0, 1.0, 1.0]);
                let c10 = colors
                    .get(y * cp_w + x + 1)
                    .copied()
                    .unwrap_or([1.0, 1.0, 1.0]);
                let c11 = colors
                    .get((y + 1) * cp_w + x + 1)
                    .copied()
                    .unwrap_or([1.0, 1.0, 1.0]);

                // default: sX = x / (controlPointsWidth - 1)
                // default: sY = y / (controlPointsHeight - 1)
                let sx = x as f32 / (cp_w - 1) as f32;
                let sy = y as f32 / (cp_h - 1) as f32;
                // default: baseVx = y * subDivisions
                // default: baseVy = x * subDivisions
                let base_vx = y * subdivisions;
                let base_vy = x * subdivisions;

                for u in 0..subdivisions {
                    let u_norm = u as f32 * inv_subdiv_m1;
                    // default: vxOffset = baseVx + u
                    let vx_offset = base_vx + u;

                    for v in 0..subdivisions {
                        let v_norm = v as f32 * inv_subdiv_m1;
                        // default: vy = baseVy + v
                        let vy = base_vy + v;

                        let (px, py) = surface_point(
                            u_norm,
                            v_norm,
                            SurfacePatch {
                                corners: [p00, p01, p10, p11],
                                tangent_power: [u_power, v_power],
                            },
                        );
                        let color = color_point(u_norm, v_norm, c00, c01, c10, c11);

                        // default: uvX = sX + v * invTH
                        // default: uvY = 1 - sY - u * invTW
                        let uv_x = sx + v as f32 * inv_th;
                        let uv_y = 1.0 - sy - u as f32 * inv_tw;

                        // default: setVertexData(vxOffset, vy, ...)
                        // 其中 idx = (vx + vy * vertexWidth) * 7
                        // 所以顶点索引 = vxOffset + vy * vertexWidth
                        let vertex_idx = vx_offset + vy * vertex_w;
                        if vertex_idx < total_vertices {
                            vertices[vertex_idx] = MeshVertex {
                                position: [px, py],
                                color,
                                uv: [uv_x, uv_y],
                            };
                        }
                    }
                }
            }
        }

        // 生成索引 (与 resize 方法一致)
        // default: for y in 0..(vertexHeight-1), for x in 0..(vertexWidth-1)
        // idx = y * vertexWidth + x
        let mut indices = Vec::with_capacity((vertex_w - 1) * (vertex_h - 1) * 6);
        for y in 0..(vertex_h - 1) {
            for x in 0..(vertex_w - 1) {
                let i00 = (y * vertex_w + x) as u32;
                let i01 = (y * vertex_w + x + 1) as u32;
                let i10 = ((y + 1) * vertex_w + x) as u32;
                let i11 = ((y + 1) * vertex_w + x + 1) as u32;

                // 的三角形顺序:
                // indexData[idx] = y * vertexWidth + x;
                // indexData[idx + 1] = y * vertexWidth + x + 1;
                // indexData[idx + 2] = (y + 1) * vertexWidth + x;
                // indexData[idx + 3] = y * vertexWidth + x + 1;
                // indexData[idx + 4] = (y + 1) * vertexWidth + x + 1;
                // indexData[idx + 5] = (y + 1) * vertexWidth + x;
                indices.extend_from_slice(&[i00, i01, i10]);
                indices.extend_from_slice(&[i01, i11, i10]);
            }
        }

        Self { vertices, indices }
    }
}

/// 计算噪声梯度 (参考 computeNoiseGradient)
/// 返回归一化的梯度向量
fn compute_noise_gradient(x: f32, y: f32, epsilon: f32) -> (f32, f32) {
    let n1 = smooth_noise(x + epsilon, y);
    let n2 = smooth_noise(x - epsilon, y);
    let n3 = smooth_noise(x, y + epsilon);
    let n4 = smooth_noise(x, y - epsilon);

    let dx = (n1 - n2) / (2.0 * epsilon);
    let dy = (n3 - n4) / (2.0 * epsilon);

    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    (dx / len, dy / len)
}

/// 随机生成控制点 (移植自 cp-generate.ts generateControlPoints)
pub fn generate_control_points(width: usize, height: usize) -> ControlPointPreset {
    let mut rng = rand::rng();

    // 原版的随机参数范围
    let variation_fraction = rng.random_range(0.4..0.6);
    let normal_offset = rng.random_range(0.3..0.6);
    let blend_factor = 0.8;
    let smooth_iters = rng.random_range(3..5) as usize;
    let smooth_factor = rng.random_range(0.2..0.3);
    let smooth_modifier = rng.random_range(-0.1..-0.05);

    let dx = if width == 1 {
        0.0
    } else {
        2.0 / (width - 1) as f32
    };
    let dy = if height == 1 {
        0.0
    } else {
        2.0 / (height - 1) as f32
    };

    let mut points = Vec::with_capacity(width * height);

    for j in 0..height {
        for i in 0..width {
            let base_x = if width == 1 {
                0.0
            } else {
                (i as f32 / (width - 1) as f32) * 2.0 - 1.0
            };
            let base_y = if height == 1 {
                0.0
            } else {
                (j as f32 / (height - 1) as f32) * 2.0 - 1.0
            };

            let is_border = i == 0 || i == width - 1 || j == 0 || j == height - 1;

            let (x, y, ur, vr, up, vp) = if is_border {
                (base_x, base_y, 0.0, 0.0, 1.0, 1.0)
            } else {
                let pert_x = rng.random_range(-variation_fraction * dx..variation_fraction * dx);
                let pert_y = rng.random_range(-variation_fraction * dy..variation_fraction * dy);
                let mut x = base_x + pert_x;
                let mut y = base_y + pert_y;

                let u_norm = (base_x + 1.0) / 2.0;
                let v_norm = (base_y + 1.0) / 2.0;

                // default: 使用噪声梯度而非直接噪声值
                let (nx, ny) = compute_noise_gradient(u_norm, v_norm, 0.001);
                let mut offset_x = nx * normal_offset;
                let mut offset_y = ny * normal_offset;

                // default: distToBorder in [0, 0.5]
                let dist_to_border = u_norm.min(1.0 - u_norm).min(v_norm).min(1.0 - v_norm);
                let weight = smoothstep(0.0, 1.0, dist_to_border);
                offset_x *= weight;
                offset_y *= weight;

                x = x * (1.0 - blend_factor) + (x + offset_x) * blend_factor;
                y = y * (1.0 - blend_factor) + (y + offset_y) * blend_factor;

                let ur = rng.random_range(-60.0..60.0);
                let vr = rng.random_range(-60.0..60.0);
                let up = rng.random_range(0.8..1.2);
                let vp = rng.random_range(0.8..1.2);

                (x, y, ur, vr, up, vp)
            };

            points.push(ControlPoint::with_tangents(x, y, ur, vr, up, vp));
        }
    }

    smoothify_control_points(
        &mut points,
        width,
        height,
        smooth_iters,
        smooth_factor,
        smooth_modifier,
    );
    ControlPointPreset::new(width, height, points)
}

/// 平滑控制点 (移植自 cp-generate.ts smoothifyControlPoints)
///
/// - iterations: 迭代次数
/// - factor: 初始平滑因子
/// - factor_iteration_modifier: 每次迭代后 factor 的变化量（可正可负）
fn smoothify_control_points(
    points: &mut [ControlPoint],
    w: usize,
    h: usize,
    iterations: usize,
    factor: f32,
    factor_iteration_modifier: f32,
) {
    let kernel = [[1.0, 2.0, 1.0], [2.0, 4.0, 2.0], [1.0, 2.0, 1.0]];
    let kernel_sum = 16.0;
    let mut f = factor;

    for _ in 0..iterations {
        let old_points: Vec<ControlPoint> = points.to_vec();

        for j in 1..(h - 1) {
            for i in 1..(w - 1) {
                let mut sum_x = 0.0;
                let mut sum_y = 0.0;
                let mut sum_ur = 0.0;
                let mut sum_vr = 0.0;
                let mut sum_up = 0.0;
                let mut sum_vp = 0.0;

                for (dj, kernel_row) in kernel.iter().enumerate() {
                    for (di, weight) in kernel_row.iter().copied().enumerate() {
                        let idx = (j + dj - 1) * w + (i + di - 1);
                        let nb = &old_points[idx];
                        sum_x += nb.x * weight;
                        sum_y += nb.y * weight;
                        sum_ur += nb.ur * weight;
                        sum_vr += nb.vr * weight;
                        sum_up += nb.up * weight;
                        sum_vp += nb.vp * weight;
                    }
                }

                let avg_x = sum_x / kernel_sum;
                let avg_y = sum_y / kernel_sum;
                let avg_ur = sum_ur / kernel_sum;
                let avg_vr = sum_vr / kernel_sum;
                let avg_up = sum_up / kernel_sum;
                let avg_vp = sum_vp / kernel_sum;

                let idx = j * w + i;
                let cur = &old_points[idx];
                points[idx].x = cur.x * (1.0 - f) + avg_x * f;
                points[idx].y = cur.y * (1.0 - f) + avg_y * f;
                points[idx].ur = cur.ur * (1.0 - f) + avg_ur * f;
                points[idx].vr = cur.vr * (1.0 - f) + avg_vr * f;
                points[idx].up = cur.up * (1.0 - f) + avg_up * f;
                points[idx].vp = cur.vp * (1.0 - f) + avg_vp * f;
            }
        }

        // default: f = Math.min(1, Math.max(f + factorIterationModifier, 0))
        f = (f + factor_iteration_modifier).clamp(0.0, 1.0);
    }
}

fn noise(x: f32, y: f32) -> f32 {
    fract((x * 12.9898 + y * 78.233).sin() * 43_758.547)
}

fn fract(x: f32) -> f32 {
    x - x.floor()
}

fn smooth_noise(x: f32, y: f32) -> f32 {
    let x0 = x.floor();
    let y0 = y.floor();
    let x1 = x0 + 1.0;
    let y1 = y0 + 1.0;

    let xf = x - x0;
    let yf = y - y0;

    let u = xf * xf * (3.0 - 2.0 * xf);
    let v = yf * yf * (3.0 - 2.0 * yf);

    let n00 = noise(x0, y0);
    let n10 = noise(x1, y0);
    let n01 = noise(x0, y1);
    let n11 = noise(x1, y1);

    let nx0 = n00 * (1.0 - u) + n10 * u;
    let nx1 = n01 * (1.0 - u) + n11 * u;

    nx0 * (1.0 - v) + nx1 * v
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// 所有预设控制点 (来自 cp-presets.ts)
/// 返回预设数组，调用者可随机选择
pub fn get_all_presets() -> Vec<ControlPointPreset> {
    vec![
        // 预设 1: 5x5 竖屏推荐
        preset_5x5_portrait(),
        // 预设 2: 4x4 横屏推荐
        preset_4x4_landscape_1(),
        // 预设 3: 4x4
        preset_4x4_landscape_2(),
        // 预设 4: 5x5
        preset_5x5_complex(),
    ]
}

/// 随机选择一个预设或生成随机控制点 (与策略一致)
/// 80% 概率使用预设，20% 概率随机生成
pub fn choose_preset_or_random() -> ControlPointPreset {
    let mut rng = rand::rng();
    if rng.random_range(0.0..1.0) > 0.8 {
        // 20% 概率：随机生成 6x6
        generate_control_points(6, 6)
    } else {
        // 80% 概率：从预设中随机选择
        let presets = get_all_presets();
        let idx = rng.random_range(0..presets.len());
        presets.into_iter().nth(idx).unwrap()
    }
}

/// 预设 1: 5x5 竖屏推荐
fn preset_5x5_portrait() -> ControlPointPreset {
    ControlPointPreset::new(
        5,
        5,
        vec![
            // Row 0
            ControlPoint::with_tangents(-1.0, -1.0, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(-0.5, -1.0, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(0.0, -1.0, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(0.5, -1.0, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(1.0, -1.0, 0.0, 0.0, 1.0, 1.0),
            // Row 1
            ControlPoint::with_tangents(-1.0, -0.5, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(-0.5, -0.5, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(-0.005_202_968_6, -0.613_142_1, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(0.588_422_7, -0.399_080_51, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(1.0, -0.5, 0.0, 0.0, 1.0, 1.0),
            // Row 2
            ControlPoint::with_tangents(-1.0, 0.0, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(-0.421_002_48, -0.118_950_58, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(-0.101_961_344, -0.023_812_119, 0.0, -47.0, 0.629, 0.849),
            ControlPoint::with_tangents(0.402_751_27, -0.063_453_145, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(1.0, 0.0, 0.0, 0.0, 1.0, 1.0),
            // Row 3
            ControlPoint::with_tangents(-1.0, 0.5, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(0.068_019_584, 0.520_591_3, -31.0, -45.0, 1.0, 1.0),
            ControlPoint::with_tangents(0.214_464_7, 0.293_316_1, 6.0, -56.0, 0.566, 1.321),
            ControlPoint::with_tangents(0.5, 0.5, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(1.0, 0.5, 0.0, 0.0, 1.0, 1.0),
            // Row 4
            ControlPoint::with_tangents(-1.0, 1.0, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(-0.313_783_74, 1.0, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(0.261_536_33, 1.0, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(0.5, 1.0, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(1.0, 1.0, 0.0, 0.0, 1.0, 1.0),
        ],
    )
}

/// 预设 2: 4x4 横屏推荐
fn preset_4x4_landscape_1() -> ControlPointPreset {
    ControlPointPreset::new(
        4,
        4,
        vec![
            // Row 0
            ControlPoint::with_tangents(-1.0, -1.0, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(-0.333_333_34, -1.0, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(0.333_333_34, -1.0, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(1.0, -1.0, 0.0, 0.0, 1.0, 1.0),
            // Row 1
            ControlPoint::with_tangents(-1.0, -0.044_954, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(-0.240_561_17, -0.224_66, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(0.334_758_88, -0.005_312_972, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(0.998_992, -0.338_297_6, 8.0, 0.0, 0.566, 1.792),
            // Row 2
            ControlPoint::with_tangents(-1.0, 0.333_333_34, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(-0.342_549_74, -0.000_027_501_608, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(0.332_143_78, 0.198_177_64, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(1.0, 0.076_611_82, 0.0, 0.0, 1.0, 1.0),
            // Row 3
            ControlPoint::with_tangents(-1.0, 1.0, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(-0.333_333_34, 1.0, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(0.333_333_34, 1.0, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(1.0, 1.0, 0.0, 0.0, 1.0, 1.0),
        ],
    )
}

/// 预设 3: 4x4
fn preset_4x4_landscape_2() -> ControlPointPreset {
    ControlPointPreset::new(
        4,
        4,
        vec![
            // Row 0
            ControlPoint::with_tangents(-1.0, -1.0, 0.0, 0.0, 1.0, 2.075),
            ControlPoint::with_tangents(-0.333_333_34, -1.0, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(0.333_333_34, -1.0, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(1.0, -1.0, 0.0, 0.0, 1.0, 1.0),
            // Row 1
            ControlPoint::with_tangents(-1.0, -0.454_577_95, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(-0.333_333_34, -0.333_333_34, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(0.088_940_315, -0.602_571_1, -32.0, 45.0, 1.0, 1.0),
            ControlPoint::with_tangents(1.0, -0.333_333_34, 0.0, 0.0, 1.0, 1.0),
            // Row 2
            ControlPoint::with_tangents(-1.0, -0.074_024_09, 1.0, 0.0, 1.0, 0.094),
            ControlPoint::with_tangents(-0.271_942_26, 0.097_753_696, 25.0, -18.0, 1.321, 0.0),
            ControlPoint::with_tangents(0.198_774_14, 0.430_738_33, 48.0, -40.0, 0.755, 0.975),
            ControlPoint::with_tangents(1.0, 0.333_333_34, -37.0, 0.0, 1.0, 1.0),
            // Row 3
            ControlPoint::with_tangents(-1.0, 1.0, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(-0.333_333_34, 1.0, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(0.512_585_1, 1.0, -20.0, -18.0, 0.0, 1.604),
            ControlPoint::with_tangents(1.0, 1.0, 0.0, 0.0, 1.0, 1.0),
        ],
    )
}

/// 预设 4: 5x5 复杂
fn preset_5x5_complex() -> ControlPointPreset {
    ControlPointPreset::new(
        5,
        5,
        vec![
            // Row 0
            ControlPoint::with_tangents(-1.0, -1.0, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(-0.450_195_3, -1.0, 0.0, 55.0, 1.0, 2.075),
            ControlPoint::with_tangents(0.1953125, -1.0, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(0.458_007_8, -1.0, 0.0, -25.0, 1.0, 1.0),
            ControlPoint::with_tangents(1.0, -1.0, 0.0, 0.0, 1.0, 1.0),
            // Row 1
            ControlPoint::with_tangents(-1.0, -0.251_447_53, -16.0, 0.0, 2.327, 0.943),
            ControlPoint::with_tangents(-0.55859375, -0.660_932_6, 47.0, 0.0, 2.358, 0.377),
            ControlPoint::with_tangents(0.232_421_88, -0.524_437_55, -66.0, -25.0, 1.855, 1.164),
            ControlPoint::with_tangents(0.685_546_9, -0.375_370_65, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(1.0, -0.669_912_5, 0.0, 0.0, 1.0, 1.0),
            // Row 2
            ControlPoint::with_tangents(-1.0, 0.035_910_398, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(-0.4921875, 0.005_378_616_5, 90.0, 23.0, 1.0, 1.981),
            ControlPoint::with_tangents(0.021484375, -0.136_504_37, 0.0, 42.0, 1.0, 1.0),
            ControlPoint::with_tangents(0.4765625, 0.059_258_23, -30.0, 0.0, 1.95, 0.44),
            ControlPoint::with_tangents(1.0, 0.251_428_84, 0.0, 0.0, 1.0, 1.0),
            // Row 3
            ControlPoint::with_tangents(-1.0, 0.696_833_67, -68.0, 0.0, 1.0, 0.786),
            ControlPoint::with_tangents(-0.690_429_7, 0.589_074_43, -68.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(0.184_570_31, 0.387_923_87, 61.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(0.60546875, 0.463_355_33, -47.0, -59.0, 0.849, 1.73),
            ControlPoint::with_tangents(1.0, 0.621_402_2, -33.0, 0.0, 0.377, 1.604),
            // Row 4
            ControlPoint::with_tangents(-1.0, 1.0, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(-0.5, 1.0, 0.0, -73.0, 1.0, 1.0),
            ControlPoint::with_tangents(-0.327_148_44, 1.0, 0.0, -24.0, 0.314, 2.704),
            ControlPoint::with_tangents(0.5, 1.0, 0.0, 0.0, 1.0, 1.0),
            ControlPoint::with_tangents(1.0, 1.0, 0.0, 0.0, 1.0, 1.0),
        ],
    )
}
