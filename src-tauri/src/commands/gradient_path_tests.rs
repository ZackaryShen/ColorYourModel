//! gradient_path_paint 的语义分支测试（0.2.0-P2 渐变 v3）。
//! 放在独立测试模块文件以避免 commands/paint.rs 过长；
//! 通过 pub(crate) 可见性复用命令内部逻辑。

use crate::commands::paint::gradient_path_color as piecewise_color;

#[test]
fn knee_semantics_t_zero_is_color_a() {
    let c = piecewise_color(0.0, 0.4, [0, 0, 255, 255], [255, 255, 0, 255]);
    assert_eq!(c, [0, 0, 255, 255]);
}

#[test]
fn knee_semantics_at_knee_is_50_50_mix() {
    let c = piecewise_color(0.4, 0.4, [0, 0, 255, 255], [255, 255, 0, 255]);
    // 50/50 蓝/黄 mix → RGB 中性灰 (127,127,127)——证伪报告 M2 预警的感知局限：
    // sRGB lerp 下蓝↔黄的中间是灰，鲜亮过渡需 OkLab 插值（backlog）
    assert_eq!(c[0], 127);
    assert_eq!(c[1], 127);
    assert_eq!(c[2], 127);
    assert_eq!(c[3], 255);
}

#[test]
fn knee_semantics_t_one_is_color_b() {
    let c = piecewise_color(1.0, 0.4, [0, 0, 255, 255], [255, 255, 0, 255]);
    assert_eq!(c, [255, 255, 0, 255]);
}

#[test]
fn color_is_monotonic_across_knee_boundary() {
    // 拐点两侧连续无跳变：0.39 与 0.41 的通道差应小于两档
    let before = piecewise_color(0.39, 0.4, [0, 0, 255, 255], [255, 255, 0, 255]);
    let at = piecewise_color(0.4, 0.4, [0, 0, 255, 255], [255, 255, 0, 255]);
    let after = piecewise_color(0.41, 0.4, [0, 0, 255, 255], [255, 255, 0, 255]);
    assert!((before[0] as i32 - at[0] as i32).abs() <= 3);
    assert!((at[0] as i32 - after[0] as i32).abs() <= 3);
}

#[test]
fn t_beyond_one_clamps_to_color_b() {
    let c = piecewise_color(1.5, 0.4, [0, 0, 255, 255], [255, 255, 0, 255]);
    assert_eq!(c, [255, 255, 0, 255]);
}

#[test]
fn samples_below_knee_stay_on_first_segment() {
    // t=0.2（knee=0.4 的第一段中点）应为 A 与 mix50 的中点
    let c = piecewise_color(0.2, 0.4, [0, 0, 255, 255], [255, 255, 0, 255]);
    assert_eq!(c[0], 64);
    assert_eq!(c[1], 64);
}
