//! UI 尺寸与配色。
//!
//! 数值逐条对应 Godot 的 `scripts/ui/*.gd` 与 `scenes/ui/*.tscn`，出处注在行尾。
//! 集中一处是 §7.2 的要求：Bevy 没有可视化排版编辑器，每次调间距都要重编译，
//! 常量散在六个文件里会把「看着不对」变成考古。
//!
//! 字号有个陷阱：Godot 没写 `font_size` 的地方是它默认的 **16**，不是 24 也不是 20。

/// 颜色。`Color::srgba` 是 const fn，所以这些都能留在编译期。
pub mod col {
    use bevy::prelude::*;

    // ===== 遮罩与面板 =====
    /// 四个弹层同值（`property_dialog.gd:112` 等）
    pub const OVERLAY: Color = Color::srgba(0.0, 0.0, 0.0, 0.55);
    /// `pause_menu.tscn:22`
    pub const PAUSE_DIM: Color = Color::srgba(0.0, 0.0, 0.0, 0.60);
    /// 面板底（各 dialog 同值）
    pub const PANEL: Color = Color::srgba(0.10, 0.10, 0.14, 0.98);
    pub const BORDER_BUY: Color = Color::srgba(0.75, 0.65, 0.25, 0.80); // property_dialog.gd:130
    pub const BORDER_UPGRADE: Color = Color::srgba(0.25, 0.60, 0.35, 0.80); // upgrade_dialog.gd:117
    pub const BORDER_RENT: Color = Color::srgba(0.75, 0.55, 0.25, 0.80); // rent_notice.gd:78
    pub const BORDER_CARD: Color = Color::srgba(0.75, 0.60, 0.90, 0.85); // card_notice.gd:82
    /// B5b 新增：柜台与两块浮层的描边（源项目没有这两块，配色跟着语义走）
    pub const BORDER_BANK: Color = Color::srgba(0.45, 0.70, 0.90, 0.85);
    pub const BORDER_STOCK: Color = Color::srgba(0.55, 0.80, 0.65, 0.85);
    /// P3b 新手牌。与 `TITLE_GOLD` 同族但压暗：一块随时开合的浮层不该比标题还抢眼
    pub const BORDER_HAND: Color = Color::srgba(0.80, 0.68, 0.35, 0.85);

    // ===== 文字 =====
    pub const TITLE_GOLD: Color = Color::srgb(1.00, 0.85, 0.30);
    pub const HINT: Color = Color::srgb(0.90, 0.90, 0.95); // board.tscn:47
    pub const PHASE: Color = Color::srgb(1.00, 0.90, 0.30); // board.tscn:54
    pub const BODY: Color = Color::srgb(0.82, 0.82, 0.88);
    pub const MUTED: Color = Color::srgb(0.70, 0.70, 0.75);
    pub const KEY: Color = Color::srgb(0.60, 0.60, 0.65);
    pub const VALUE: Color = Color::srgb(0.75, 0.75, 0.80);
    /// 要付的钱（地价、升级费）
    pub const COST: Color = Color::srgb(1.00, 0.50, 0.40);
    /// 收益与增幅
    pub const GAIN: Color = Color::srgb(0.40, 0.85, 0.50);
    pub const CASH_OK: Color = Color::srgb(0.50, 0.85, 0.55);
    pub const CASH_BAD: Color = Color::srgb(1.00, 0.45, 0.45);
    pub const BANKRUPT: Color = Color::srgb(1.00, 0.35, 0.35);
    pub const JAILED: Color = Color::srgb(1.00, 0.60, 0.30);
    /// 卡面「效果：」那一行（card_notice.gd:138）
    pub const RESULT: Color = Color::srgb(0.55, 0.85, 0.60);
    pub const CHANCE: Color = Color::srgb(0.30, 0.70, 0.85); // card_notice.gd:13
    pub const FATE: Color = Color::srgb(0.80, 0.60, 0.95); // card_notice.gd:14

    // ===== 按钮（dialog 的 `_style_button`）=====
    pub const BTN_PRIMARY: Color = Color::srgb(0.25, 0.55, 0.30);
    pub const BTN_PRIMARY_HOVER: Color = Color::srgb(0.30, 0.65, 0.35);
    pub const BTN_SECONDARY: Color = Color::srgb(0.22, 0.22, 0.28);
    pub const BTN_SECONDARY_HOVER: Color = Color::srgb(0.30, 0.30, 0.38);
    pub const BTN_DISABLED: Color = Color::srgb(0.15, 0.15, 0.18);
    pub const BTN_BORDER: Color = Color::srgb(0.45, 0.45, 0.50);

    pub const BTN_TEXT_DISABLED: Color = Color::srgb(0.45, 0.45, 0.50);

    // ===== 玩家 HUD（player_hud.gd）=====
    pub const HUD_CARD: Color = Color::srgba(0.12, 0.12, 0.16, 0.90); // :72
    pub const HUD_CARD_ACTIVE: Color = Color::srgba(0.18, 0.20, 0.14, 0.95); // :208
    pub const HUD_BORDER_ACTIVE: Color = Color::srgba(0.70, 0.65, 0.25, 0.90); // :213
    pub const HUD_NAME: Color = Color::srgb(0.88, 0.88, 0.92); // :95
    pub const HUD_CASH: Color = Color::srgb(0.50, 0.85, 0.55); // :104
    pub const HUD_PROPS: Color = Color::srgb(0.65, 0.65, 0.72); // :112
    /// 点券。取 `palette::tile(TileKind::Coupon)` 那一档 mint：
    /// 数字与格子的颜色对上，玩家才认得出「HUD 上这笔账是路上捡来的那种钱」
    pub const HUD_POINTS: Color = Color::srgb(0.35, 0.9, 0.75);

    // ===== 主菜单（main_menu.tscn）=====
    pub const MENU_BG: Color = Color::srgb(0.12, 0.12, 0.16); // :21
    pub const MENU_TITLE: Color = Color::srgb(1.00, 0.85, 0.20); // :38
    pub const MENU_FOOTER: Color = Color::srgb(0.50, 0.50, 0.55); // :79
}

/// 字号。名字按用途而不是按数字，改数值不改调用点。
pub mod size {
    /// Godot 未覆盖处的默认字号
    pub const BODY: f32 = 16.0;
    /// HUD 的地产数与状态标签
    pub const SMALL: f32 = 13.0;
    /// 弹层信息行与副标题
    pub const ROW: f32 = 14.0;
    pub const NAME: f32 = 15.0;
    pub const CARD_TITLE: f32 = 26.0;
    pub const DIALOG_TITLE: f32 = 22.0;
    pub const BUY_TITLE: f32 = 24.0;
    pub const RENT_AMOUNT: f32 = 32.0;
    pub const PAUSE_TITLE: f32 = 28.0;
    pub const MENU_TITLE: f32 = 48.0;
    /// 卡面「确定」按钮（`card_notice.gd:150` 的 17，比其余按钮大一档）
    pub const CARD_OK: f32 = 17.0;
}

/// 布局尺寸（px）。
pub mod layout {
    /// 顶栏：`board.tscn:33-41` TOP_WIDE + 边距 16、gap 12
    pub const TOP_BAR_INSET: f32 = 16.0;
    pub const TOP_BAR_GAP: f32 = 12.0;
    pub const ROLL_BUTTON: [f32; 2] = [140.0, 44.0];
    pub const BACK_BUTTON: [f32; 2] = [120.0, 44.0];

    /// 弹层面板宽与「左上角相对屏幕中心」的偏移（`PRESET_CENTER` + `position`）
    pub const BUY_PANEL: [f32; 3] = [400.0, -200.0, -220.0];
    pub const UPGRADE_PANEL: [f32; 3] = [400.0, -200.0, -180.0];
    pub const RENT_PANEL: [f32; 3] = [380.0, -190.0, -150.0];
    pub const CARD_PANEL: [f32; 3] = [420.0, -210.0, -170.0];
    /// 弹层按钮：买地/升级成对（`property_dialog.gd:194,202`），收据与卡面各一颗（`:123`、`:148`）
    pub const MODAL_BUTTON: [f32; 2] = [140.0, 42.0];
    pub const OK_BUTTON: [f32; 2] = [160.0, 42.0];
    pub const CARD_OK_BUTTON: [f32; 2] = [180.0, 44.0];
    /// 按钮行间距（`btn_box` 的 separation 20）与信息行组间距（InfoPanel 的 4）
    pub const BTN_ROW_GAP: f32 = 20.0;
    pub const INFO_ROW_GAP: f32 = 4.0;
    /// 信息行键列宽：买地/升级面板是 100，升级面板 110（`_add_info_row`）
    pub const INFO_KEY_W: f32 = 100.0;
    pub const INFO_KEY_W_WIDE: f32 = 110.0;
    /// 面板圆角 / 边框宽 / 内边距（横 24~26、纵 20~22，取 Godot 的众数）
    pub const PANEL_RADIUS: f32 = 10.0;
    pub const PANEL_BORDER: f32 = 2.0;
    pub const PANEL_PAD_X: f32 = 24.0;
    pub const PANEL_PAD_Y: f32 = 20.0;
    pub const BTN_RADIUS: f32 = 6.0;
    pub const BTN_BORDER: f32 = 1.0;

    /// 玩家 HUD：底部通栏。Godot 的 `PRESET_BOTTOM_WIDE` + `offset_top=-120 / bottom=-8 /
    /// 左右 8`（`player_hud.gd:15-20`），翻成 Bevy 是「bottom 8 + 高 112」
    pub const HUD_BOTTOM: [f32; 4] = [112.0, 8.0, 8.0, 8.0];
    pub const HUD_CARD_GAP: f32 = 4.0;
    pub const HUD_ROW_GAP: f32 = 10.0;
    pub const HUD_DOT: f32 = 14.0;

    /// 主菜单：VBox 宽 320、gap 16（`main_menu.tscn:23-33`）
    pub const MENU_WIDTH: f32 = 320.0;
    pub const MENU_GAP: f32 = 16.0;
    pub const MENU_BUTTON_H: f32 = 48.0;
    pub const PAUSE_PANEL: [f32; 2] = [320.0, 280.0];

    // ===== B5b：银行与证券（源项目没有这两块，数值是本项目自己的）=====
    /// 顶栏三个入口按钮（ATM / 证券 / 手牌）。96 → 84 是让位给 P3b 那第三颗：
    /// 顶栏一行塞得下六颗，但提示行与阶段名会被挤到换行 —— 而换行的「等待掷\n骰」
    /// 看着像排版坏了。两个字符的中文标签在 16 号字下用不到 96 宽。
    pub const FINANCE_BUTTON: [f32; 2] = [84.0, 44.0];
    /// ATM 浮层：左上，压在顶栏下面。与右上角的详情卡、左下的证券面板互不透叠
    pub const ATM_PANEL: [f32; 3] = [300.0, 16.0, 72.0];
    /// 证券浮层：左下，留出底部 HUD 的 112 + 20
    pub const STOCK_PANEL: [f32; 3] = [460.0, 16.0, 132.0];
    /// 柜台模态：比买地面板高一截（四行按钮 + 办完）
    pub const BANK_PANEL: [f32; 3] = [440.0, -220.0, -230.0];
    /// 面板内的金额按钮：四行两列，比模态主按钮小一档
    pub const AMOUNT_BUTTON: [f32; 2] = [132.0, 36.0];
    /// 浮层里「关闭」那颗
    pub const PANEL_CLOSE_BUTTON: [f32; 2] = [92.0, 32.0];
    pub const OP_ROW_GAP: f32 = 8.0;

    // ===== P3b：手牌浮层 =====
    /// 手牌浮层：**右下**，`[宽, 右边距, 下边距]`。锚在下面而不是上面，
    /// 所以牌越攒越多时它是往上长，不会压住顶栏，也不会盖掉右上角的点格详情卡。
    /// 下边距留 132 与证券浮层同一水平线（底部 HUD 112 + 20）。
    pub const HAND_PANEL: [f32; 3] = [330.0, 16.0, 132.0];
    /// 一张卡一行：卡名一行、效果一行，右边竖着排「用」与每个对手一颗
    pub const CARD_BUTTON: [f32; 2] = [96.0, 32.0];
    /// 一张卡三行（卡名 / 效果 / 按钮），行间距比金额按钮那行再紧一档：
    /// 15 张牌时整块面板还要留在屏内
    pub const HAND_ROW_GAP: f32 = 6.0;
}
