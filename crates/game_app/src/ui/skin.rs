//! UI 底板皮肤：把 `.tmp/ui/` 那两批剪纸底板接进 bevy_ui。
//!
//! 口径沿自素材侧（`baofu-ui-assets`）：**底板只出形不出字**——中央素面留给引擎渲染
//! 文字，形状外透明，交互态（按下/悬停）由引擎叠色派生。素材按 alpha 外沿裁边后转
//! webp 放 `assets/ui/skin_*.webp`（`assets/` 整目录不入库，盘上没有是正常状态，
//! [`crate::art::ui_skin`] 查不到就退回纯色面板——与 logo/背景图同一条纪律）。
//!
//! 面板走**九宫格**（`NodeImageMode::Sliced`）：四角的花纹不变形、边条沿边拉伸、
//! 中央素面吃掉内容撑出来的高度。切片边距是从母版程序化量出来的（素面色块边界到
//! 内容边），每张不同，见 [`Skins::load`] 的表。
//!
//! 这一批只接**现有界面真需要的六款面板**。按钮三款与横幅两款刻意不接：
//! 按钮显示比例（约 3.2:1）与母版（2.4:1）失配，九宫格会把回纹压扁，正确做法是为
//! 按钮重新生成一条长条比例的底板；横幅（新闻卷轴）对应的界面还不存在，硬塞进
//! 顶栏会把轴杆拉细——等落点出现再接。

use bevy::prelude::*;
use bevy::sprite::SliceScaleMode;
use game_core::prelude::Pending;

use crate::art;
use crate::render::DevFont;

/// 一款底板：贴图句柄、九宫格边距（纹理像素）与四角缩放上限。
/// `image` 为 `None` 即盘上没有，调用方退回 [`theme::panel`] 的纯色面板。
#[derive(Clone, Debug)]
pub struct Skin {
    pub image: Option<Handle<Image>>,
    /// 左、上、右、下。单位是贴图像素，[`panel_image`] 直接按这个值切片。
    pub slice: [f32; 4],
    /// 四角在屏上的最大缩放。母版是 640 宽的剪纸边框，原尺寸贴到 400–1000 宽的
    /// 面板上会吃掉大半内容区；这个数把边框带压到能用的厚度（与
    /// `TextureSlicer::max_corner_scale` 同一个值，见 [`panel_image`]）。
    pub corner_scale: f32,
}

// 模态构造函数的公共前缀。`font` 与 `skins` 在每一处都成对出现，分开传就撞
// `too_many_arguments`——按房规收拢成参数包而不是挂 `#[allow]`（§六 坑 8）。
pub struct UiKit<'a> {
    pub font: &'a DevFont,
    pub skins: &'a Skins,
}

/// 全部底板句柄。开机在 `UiPlugin::build` 里同步 `load` 一次——`AssetServer::load`
/// 是异步解码、同步拿句柄，第一帧纯色、随后贴图浮现，与 DevFont 同一条路。
#[derive(Resource, Clone, Debug)]
pub struct Skins {
    /// 通用模态（买地/升级/收据/卡面/银行/百货/建设/魔法屋/被动门……）
    pub modal: Skin,
    /// 拍卖台（P9）
    pub auction: Skin,
    /// 彩票柜台（P8）
    pub lottery: Skin,
    /// 保释窗口（P4③）
    pub bail: Skin,
    /// 常驻浮层（ATM / 证券 / 手牌 / 道具）
    pub float: Skin,
}

impl Skins {
    fn load(server: &AssetServer, key: &str, slice: [f32; 4], corner_scale: f32) -> Skin {
        Skin {
            image: art::ui_skin(key).map(|path| server.load(path)),
            slice,
            corner_scale,
        }
    }

    pub fn load_all(server: &AssetServer) -> Self {
        // 边距是 `.tmp/ui/finalize.py` 按「深度法」量的（每边素面最大侵入深度，
        // 波浪边取保守值），母版已统一预缩放到 640px 宽——表与图不同步的症状是
        // 边框花纹被九宫格拉伸。v1 的「素面 bbox 最近点」量法被四角侵入的花纹
        // 骗过一轮，换表前先想清楚量的是什么。
        //
        // `corner_scale` 是显示端的另一刀：640 母版原样贴进 300–1000 宽的面板，
        // 边框带会从「装饰」变成「吃掉内容」。0.5–0.7 是让边框仍读得出来、又不
        // 把文字挤进窄条的值；改它要连同 [`padding`] 的估算一起看。
        Self {
            modal: Self::load(server, "panel_modal", [68.0, 72.0, 88.0, 82.0], 0.50),
            auction: Self::load(server, "panel_auction", [24.0, 104.0, 26.0, 28.0], 0.55),
            lottery: Self::load(server, "panel_lottery", [94.0, 88.0, 88.0, 108.0], 0.55),
            bail: Self::load(server, "panel_bail", [44.0, 34.0, 42.0, 60.0], 0.60),
            float: Self::load(server, "panel_float", [56.0, 42.0, 58.0, 40.0], 0.70),
        }
    }

    /// 这个待决事项该用哪块底板。拍卖/彩票/保释各有专属，其余全走通用模态——
    /// 那几块屏的语义色（`BORDER_*`）继续由 accent 承担，只是贴图共用一块。
    pub fn pick(&self, about: &Pending) -> &Skin {
        match about {
            Pending::Auction => &self.auction,
            Pending::Lottery => &self.lottery,
            Pending::Bail(_) => &self.bail,
            _ => &self.modal,
        }
    }
}

/// 内容起笔要离面板边多远（像素）。取「原设计的内边距」与「边框带屏上厚度」的较大者。
///
/// 边框带厚度 = 纹理边距 × 缩放：`compute_texture_slices` 取
/// `min(面板宽/图宽, 面板高/图高, corner_scale)`——这里只用宽度估，面板高未知；
/// 估算偏大时结果是内边距略保守（内容多缩一点），不会压到花纹上。
pub fn inset(skin: &Skin, base_x: f32, base_y: f32, panel_w: f32) -> (f32, f32) {
    match skin.image {
        Some(_) => {
            let scale = (panel_w / 640.0).min(skin.corner_scale);
            (
                base_x.max(skin.slice[0].max(skin.slice[2]) * scale),
                base_y.max(skin.slice[1].max(skin.slice[3]) * scale),
            )
        }
        None => (base_x, base_y),
    }
}

/// [`inset`] 的 `UiRect` 形状，直接给 `Node::padding`。
pub fn padding(skin: &Skin, base_x: f32, base_y: f32, panel_w: f32) -> UiRect {
    let (x, y) = inset(skin, base_x, base_y, panel_w);
    UiRect::axes(Val::Px(x), Val::Px(y))
}

/// 面板底图的九宫格节点。盘上没有素材就是 `None`，调用方保持纯色面板。
///
/// 0.20 移除了 `Option<C>` 的 `Bundle` 实现，所以皮肤不再作为返回元组的一格由调用方
/// 组装，而是「先 spawn 纯色面板、再按这一格的返回值 `insert`」——加载完成前第一帧
/// 是纯色底，贴图随后浮现，与 DevFont 同一条路。
///
/// 返回 `(Node, ImageNode)` 两件：贴图挂**子节点**而不直接插在面板节点上——面板高度
/// 由内容撑开，`Sliced` 对「尺寸未定」的节点的取形不可靠（实测按原比例缩在中央、
/// 四周露出纯色底），绝对定位 100%×100% 的显式矩形才给它一个确定的目标框。
pub fn panel_image(skin: &Skin) -> Option<(Node, ImageNode)> {
    let handle = skin.image.clone()?;
    // Sliced 的边距单位是纹理像素（四角按 1:1 原样贴）；母版已统一预缩放到
    // 640px 宽（见 `.tmp/ui/finalize.py` 的深度法量测），边距表就是那个尺度下的
    // 显示像素。0.20 的 `BorderRect` 是「矩形内缩」形状：min 角 = 左/上，max 角 = 右/下。
    let [left, top, right, bottom] = skin.slice;
    Some((
        Node {
            position_type: PositionType::Absolute,
            // 铺满面板节点；`left:0` 与 padding 互不干扰——绝对定位的参照是父节点的边框盒
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            right: Val::Px(0.0),
            bottom: Val::Px(0.0),
            ..default()
        },
        ImageNode {
            image: handle,
            image_mode: NodeImageMode::Sliced(TextureSlicer {
                border: BorderRect {
                    min_inset: Vec2::new(left, top),
                    max_inset: Vec2::new(right, bottom),
                },
                // 边条平铺而不是拉伸：`.tmp/ui/unitize.py` 已把每条边改成「花纹 + 镜像」
                // 的回文单元，首尾像素相同，所以 Tile 重复整条边时接缝不断。中间是纯色，
                // 用默认的 Stretch 即可。
                sides_scale_mode: SliceScaleMode::Tile { stretch_value: 1.0 },
                max_corner_scale: skin.corner_scale,
                ..default()
            }),
            ..default()
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bare(slice: [f32; 4]) -> Skin {
        Skin {
            image: None,
            slice,
            corner_scale: 0.5,
        }
    }

    fn all_bare() -> Skins {
        Skins {
            modal: bare([1.0, 2.0, 3.0, 4.0]),
            auction: bare([10.0, 20.0, 30.0, 40.0]),
            lottery: bare([50.0, 60.0, 70.0, 80.0]),
            bail: bare([90.0, 100.0, 110.0, 120.0]),
            float: bare([130.0, 140.0, 150.0, 160.0]),
        }
    }

    #[test]
    fn without_an_image_the_padding_is_the_original_design() {
        // 素材缺席时 `padding` 必须一字不改地退回设计值——那是「缺素材退回纯色」的边界
        let skin = bare([999.0; 4]);
        assert_eq!(
            padding(&skin, 16.0, 14.0, 1000.0),
            UiRect::axes(Val::Px(16.0), Val::Px(14.0))
        );
    }

    #[test]
    fn a_wide_panel_pads_by_the_corner_scale() {
        // 1000 宽面板：四角缩放被 corner_scale 封顶在 0.5，右 88 → 44、下 82 → 41
        let mut skin = bare([68.0, 72.0, 88.0, 82.0]);
        skin.image = Some(Handle::default());
        assert_eq!(
            padding(&skin, 26.0, 24.0, 1000.0),
            UiRect::axes(Val::Px(44.0), Val::Px(41.0))
        );
    }

    #[test]
    fn a_narrow_panel_scales_the_border_down_with_its_width() {
        // 240 宽：缩放 240/640 = 0.375，边框带跟着变薄，内容区才不至于被吃掉
        let mut skin = bare([68.0, 72.0, 88.0, 82.0]);
        skin.image = Some(Handle::default());
        let (x, y) = inset(&skin, 14.0, 12.0, 240.0);
        assert!((x - 33.0).abs() < 0.001, "x={x}");
        assert!((y - 30.75).abs() < 0.001, "y={y}");
    }

    #[test]
    fn a_narrow_panel_keeps_the_larger_design_inset() {
        // 设计内边距更大就取设计值，不能反被花纹顶小
        let mut skin = bare([10.0, 10.0, 10.0, 10.0]);
        skin.image = Some(Handle::default());
        assert_eq!(
            padding(&skin, 26.0, 24.0, 640.0),
            UiRect::axes(Val::Px(26.0), Val::Px(24.0))
        );
    }

    #[test]
    fn counters_with_a_dedicated_panel_do_not_fall_back_to_the_modal() {
        let skins = all_bare();
        assert_eq!(skins.pick(&Pending::Auction).slice[0], 10.0);
        assert_eq!(skins.pick(&Pending::Lottery).slice[0], 50.0);
        // 没有专属底板的那几屏继续共用通用模态
        assert_eq!(skins.pick(&Pending::Shop).slice[0], 1.0);
    }
}
