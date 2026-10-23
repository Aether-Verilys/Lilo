//! 迷你浮窗的贴边吸附与折叠。
//!
//! 拖到屏幕左/右边缘附近松手 → 吸附贴平；鼠标移开 → 滑出屏幕只留一条
//! 窄边可见；鼠标移到那条窄边上 → 滑回来展开。
//!
//! 几何计算全部用物理像素，避免高 DPI 下逻辑像素与物理像素混算导致偏移。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Left,
    Right,
}

/// 吸附判定的容差（逻辑像素）。拖到离边缘这么近就算贴边。
const SNAP_THRESHOLD: f64 = 24.0;

/// 折叠后留在屏幕内的窄边宽度（逻辑像素）。
/// 太窄会难以用鼠标命中，6px 是手感和「不挡视线」的折中。
const PEEK: f64 = 14.0;

/// 屏幕的水平范围（物理像素）
#[derive(Debug, Clone, Copy)]
pub struct Bounds {
    pub x: i32,
    pub width: i32,
}

/// 判断窗口当前是否够近以触发吸附。返回吸附到哪一边，None 表示不吸附。
///
/// `win_x` / `win_width` 是窗口的物理位置与宽度，`scale` 是显示器缩放。
pub fn compute_snap(b: Bounds, win_x: i32, win_width: i32, scale: f64) -> Option<Side> {
    let threshold = (SNAP_THRESHOLD * scale).round() as i32;
    let left_gap = win_x - b.x;
    let right_gap = (b.x + b.width) - (win_x + win_width);

    // 两边都够近时取更近的一边；负数（已越界）也算贴边
    // 只在屏幕边缘附近吸附；窗口已经远远越过边缘时不能继续被判定为贴边，
    // 否则一次错误坐标就会把窗口甩到很远的位置。
    let near_left = (-threshold..=threshold).contains(&left_gap);
    let near_right = (-threshold..=threshold).contains(&right_gap);
    match (near_left, near_right) {
        (true, true) => Some(if left_gap <= right_gap {
            Side::Left
        } else {
            Side::Right
        }),
        (true, false) => Some(Side::Left),
        (false, true) => Some(Side::Right),
        (false, false) => None,
    }
}

/// 吸附后（展开态）窗口应该在的 x：紧贴屏幕边缘
pub fn expanded_x(b: Bounds, side: Side, win_width: i32) -> i32 {
    match side {
        Side::Left => b.x,
        Side::Right => b.x + b.width - win_width,
    }
}

/// 折叠态窗口应该在的 x：滑出屏幕，只留 PEEK 那条窄边
pub fn collapsed_x(b: Bounds, side: Side, win_width: i32, scale: f64) -> i32 {
    let peek = (PEEK * scale).round() as i32;
    match side {
        Side::Left => b.x - (win_width - peek),
        Side::Right => b.x + b.width - peek,
    }
}

/// 把 y 夹在屏幕内，防止吸附后窗口跑到看不见的地方
pub fn clamp_y(mon_y: i32, mon_height: i32, win_y: i32, win_height: i32) -> i32 {
    let max = mon_y + mon_height - win_height;
    if max < mon_y {
        return mon_y;
    }
    win_y.clamp(mon_y, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MON: Bounds = Bounds { x: 0, width: 1920 };

    #[test]
    fn snaps_when_near_left_edge() {
        assert_eq!(compute_snap(MON, 10, 280, 1.0), Some(Side::Left));
    }

    #[test]
    fn snaps_when_near_right_edge() {
        // 右边缘 1920，窗口右沿 1920-10=1910 → 间隙 10，在容差内
        assert_eq!(compute_snap(MON, 1630, 280, 1.0), Some(Side::Right));
    }

    #[test]
    fn does_not_snap_in_the_middle() {
        assert_eq!(compute_snap(MON, 800, 280, 1.0), None);
    }

    #[test]
    fn snaps_to_nearer_edge_on_narrow_screen() {
        // 窗口比屏幕还宽时两边都在容差内，应取更近的一边
        let narrow = Bounds { x: 0, width: 300 };
        assert_eq!(compute_snap(narrow, 2, 280, 1.0), Some(Side::Left));
        assert_eq!(compute_snap(narrow, 18, 280, 1.0), Some(Side::Right));
    }

    #[test]
    fn threshold_scales_with_dpi() {
        // 150% 缩放下容差是 36 物理像素，30px 的间隙仍应吸附
        assert_eq!(compute_snap(MON, 30, 280, 1.5), Some(Side::Left));
        // 1.0 缩放下容差 24，30px 就不该吸附
        assert_eq!(compute_snap(MON, 30, 280, 1.0), None);
    }

    #[test]
    fn expanded_sits_flush_against_edge() {
        assert_eq!(expanded_x(MON, Side::Left, 280), 0);
        assert_eq!(expanded_x(MON, Side::Right, 280), 1640);
    }

    #[test]
    fn collapsed_leaves_only_peek_visible() {
        // 左吸附折叠：窗口左移到 -266，右沿在 x=14，留出可悬浮的窄边
        let x = collapsed_x(MON, Side::Left, 280, 1.0);
        assert_eq!(x, -266);
        assert_eq!(x + 280, 14, "右沿应留 14px 在屏内");

        // 右吸附折叠：左沿在 1906，屏内可见 14px
        let x = collapsed_x(MON, Side::Right, 280, 1.0);
        assert_eq!(x, 1906);
        assert_eq!(1920 - x, 14, "左沿应留 14px 在屏内");
    }

    #[test]
    fn collapse_expand_roundtrip_is_stable() {
        // 折叠再展开必须回到原位，否则反复 hover 会让窗口越走越偏
        for side in [Side::Left, Side::Right] {
            let e = expanded_x(MON, side, 280);
            let c = collapsed_x(MON, side, 280, 1.0);
            assert_ne!(e, c, "折叠与展开位置应不同");
            assert_eq!(expanded_x(MON, side, 280), e, "展开位置只由屏幕和宽度决定");
        }
    }

    #[test]
    fn works_on_secondary_monitor_with_offset() {
        // 副屏起点不是 0，吸附必须按该屏的坐标算
        let second = Bounds {
            x: 1920,
            width: 1920,
        };
        assert_eq!(compute_snap(second, 1930, 280, 1.0), Some(Side::Left));
        assert_eq!(expanded_x(second, Side::Left, 280), 1920);
        assert_eq!(expanded_x(second, Side::Right, 280), 3560);
        assert_eq!(collapsed_x(second, Side::Right, 280, 1.0), 3826);
    }

    #[test]
    fn works_on_monitor_with_negative_offset() {
        // 左侧副屏在 Windows 上坐标是负的（实测该机器：X=-2560，2560x1440）。
        // 用无符号或 clamp 到 0 的写法会在这里把窗口甩到主屏上。
        let left_screen = Bounds {
            x: -2560,
            width: 2560,
        };
        assert_eq!(
            compute_snap(left_screen, -2550, 280, 1.0),
            Some(Side::Left),
            "副屏左边缘应能吸附"
        );
        assert_eq!(expanded_x(left_screen, Side::Left, 280), -2560);
        assert_eq!(expanded_x(left_screen, Side::Right, 280), -280);
        // 左吸附折叠：留 14px 在该屏最左侧
        let c = collapsed_x(left_screen, Side::Left, 280, 1.0);
        assert_eq!(c + 280, -2546);
        // 屏幕正中不应吸附
        assert_eq!(compute_snap(left_screen, -1280, 280, 1.0), None);
    }

    #[test]
    fn clamps_y_into_screen_with_negative_origin() {
        // 副屏 Y=177，窗口不能被夹到 0（那是主屏的坐标系）
        assert_eq!(clamp_y(177, 1440, 0, 120), 177);
        assert_eq!(clamp_y(177, 1440, 5000, 120), 177 + 1440 - 120);
    }

    #[test]
    fn clamps_y_into_screen() {
        assert_eq!(clamp_y(0, 1080, -50, 120), 0);
        assert_eq!(clamp_y(0, 1080, 2000, 120), 960);
        assert_eq!(clamp_y(0, 1080, 500, 120), 500);
    }
}
