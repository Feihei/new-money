//! 收费原型库（P10①）：商业用地与公司企业共用的那批「事件型收费」。
//!
//! PLAN §三 那句设计指令是本模块存在的理由：「商业地产收事件费、公司企业收服务费」
//! 听起来是三套系统，实际是**同一批原型的三种装配** —— 购物中心（骰 1–6）、
//! 旅馆（骰 1–4 强制住宿）、加油站（油费按步数）、航空（转盘定天数）、IBM（维修费按步数）
//! 全部只是下面四个原型各填一组参数。**同一原型换两个参数必须得出旅馆与航空、
//! 加油站与 IBM 两组不同行为** —— 这是 P10 的真验收（§四 P10 那行）；
//! 做不到的话，说明这里其实藏着五个一次性实现。
//!
//! 三条边界：
//! - **这里只算账，不收钱**。原型产出的是一张 [`ServiceBill`]（名义金额 + B 型的天数），
//!   过修正管线（大财神赖账、免费卡、物价指数）与扣款是 `Books::settle` 的事，
//!   挪棋子（B 型离场）是 `landing::detain` 的事 —— 本模块对世界一无所知，才能被
//!   商业用地（收款方＝地主）与公司企业（收款方＝董事长）两边各装配一次。
//! - **收的是 [`crate::charge::ChargeKind::Service`] 这一类钱**：它归「过路费」域
//!   （`charge.rs` 那行注释从 P2 就等着它），所以免费卡与大财神**天然**吃得到；
//!   「董事长免费」与「加油站对步行不收费」**不是**打折 —— 是根本不产生这笔账
//!   （见 [`assess`] 与它 `requires_mount` 那一格的注）。
//! - **C 型的步数＝本回合掷出的点数**（10-20 拍板，PLAN §三）：读 `DiceRolled.steps`
//!   的骰面原值（P3d④b 起它永不乘方向），不需要任何新状态。不选「从上次收费起累计
//!   走过的总步数」，因为那会把 C 型变成一个要存档的滑动窗口，玩家在踩到之前也算不出
//!   自己要付多少 —— 「可预判」退化成「事后惊讶」。

use crate::player::Mount;

/// 四原型（PLAN §三「事件型收费四原型」那张表的落地）。
///
/// 参数全是数据（P10b 起 `building_kind` 连着它一起进棋盘 JSON），
/// 原型本身不知道自己住在商业用地上还是公司企业上 —— 那是装配层的事。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeePrototype {
    /// **A 随机数决定金额**：踩到 → 摇一个数 → 按档位收钱。购物中心骰 1–6，
    /// 摇出几就走第几档（`tiers[roll-1]`）。
    RandomAmount { sides: u32, tiers: Vec<i64> },
    /// **B 随机数决定停留天数**：踩到 → 摇一个数 → **人离场 N 天** + 付费。
    /// 旅馆骰 1–4（强制住宿该天数），航空是转盘 —— 两者只有 `sides` 不同；
    /// 费用按天计（`per_day × 天数`），旅行费与住宿费在 4 代里都是「待几天掏几天的钱」。
    ///
    /// 「离场」在这个库里只是 [`ServiceBill::stay_days`] 一个数：出国旅游、住旅馆、
    /// 被绑架、坐牢在 [`crate::player::Detention`] 的模型里是同一件事的四种名字，
    /// 挪人那句 [`crate::landing::detain`] 三处共用，本模块不碰。
    RandomStay { sides: u32, per_day: i64 },
    /// **C 按步数计费**：单价 × 本回合骰面。`requires_mount` 是**载体参数**而不是
    /// 原型的不变式（10-20 拍板）：油费类（加油站、中石化）步行完全不收 —— 没车就
    /// 没有油耗；IBM／SEGA 的「电脑维修费」与车无关，步行照收。
    PerStep { rate: i64, requires_mount: bool },
    /// **D 功能型**：踩到不产生费用，解锁一个能力（研究所产道具、公园占地、
    /// 建设公司远程升级）。能力本身归装配层（P10c/P10d），这里只负责「不收费」，
    /// 免得每个 D 型载体各写一遍 `return None`。
    Functional,
}

impl FeePrototype {
    /// 这一原型摇几面的数。D 型返回 0 —— 它不摇。
    pub const fn sides(&self) -> u32 {
        match self {
            Self::RandomAmount { sides, .. } | Self::RandomStay { sides, .. } => *sides,
            Self::PerStep { .. } | Self::Functional => 0,
        }
    }

    /// 是不是 D 型。装配层用这句话决定「踩到了要不要另外给能力」，而**不该**用
    /// `matches!(proto, Functional)` 散着写 —— 将来 D 型长出参数（比如研究所的等级）
    /// 时，散写的那些地方会静默漏掉。
    pub const fn is_functional(&self) -> bool {
        matches!(self, Self::Functional)
    }
}

/// 一次踩格该收的那笔账。`base` 是名义金额（修正管线的输入，物价指数会缩它）；
/// `stay_days` 只有 B 型非零，调用方拿它去 [`crate::landing::detain`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceBill {
    pub base: i64,
    pub stay_days: u32,
}

/// 踩到服务载体上，这一次该收什么。返回 `None` ＝**根本不产生这笔账**：
/// D 型天然如此；C 型只在他「该付而没资格付」时如此（步行踩加油站 —— 没车就没油耗，
/// 这不是免单，是没这回事）。
///
/// 免单（金额 0）与不产生账是两件事：前者要过管线、留 `CashChanged(0)` 的痕迹与
/// 「被谁免掉」的流水（大财神在场时玩家得看得见为什么），后者连流水都不该有 ——
/// 玩家步行路过加油站十次，顶栏播十条「付了 0 元油费」是噪音。
///
/// `roll` 由调用方从服务子流抽（[`crate::Rng::pick_service`]，范围 `1..=sides`）；
/// D 型不摇数，传什么都行。`steps` 是本回合骰面（`DiceRolled.steps`，骰面原值）。
pub fn assess(proto: &FeePrototype, roll: u32, steps: u32, mount: Mount) -> Option<ServiceBill> {
    match proto {
        FeePrototype::RandomAmount { tiers, .. } => {
            // 摇出几就走第几档；表短于骰面（数据写歪）按没有这一档处理 ——
            // 缺档不该收第 0 档的钱，也不该 panic 一局对局。数据校验在 P10b 拦它。
            let base = tiers.get(usize::try_from(roll.checked_sub(1)?).ok()?).copied()?;
            Some(ServiceBill { base, stay_days: 0 })
        }
        FeePrototype::RandomStay { per_day, .. } => {
            // 摇出几就住几天，费用按天计。摇出 0 天（sides 配 0 的坏数据）等于白住，
            // 让它落到 `stay_days == 0` 由调用方看着办 —— 库不替装配层猜。
            Some(ServiceBill {
                base: per_day * roll as i64,
                stay_days: roll,
            })
        }
        FeePrototype::PerStep {
            rate,
            requires_mount,
        } => {
            if *requires_mount && !mount.is_vehicle() {
                return None;
            }
            Some(ServiceBill {
                base: rate * steps as i64,
                stay_days: 0,
            })
        }
        FeePrototype::Functional => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mall() -> FeePrototype {
        // 购物中心：骰 1–6，档位随摇出的点数走
        FeePrototype::RandomAmount {
            sides: 6,
            tiers: vec![100, 200, 300, 400, 500, 600],
        }
    }

    #[test]
    fn prototype_a_pays_the_tier_the_roll_picked() {
        let bill = assess(&mall(), 3, 0, Mount::Foot).expect("A 型必有账");
        assert_eq!(bill.base, 300);
        assert_eq!(bill.stay_days, 0, "A 型不住人");

        // 摇出几走第几档：1 与 6 各自对号
        assert_eq!(assess(&mall(), 1, 0, Mount::Foot).unwrap().base, 100);
        assert_eq!(assess(&mall(), 6, 0, Mount::Foot).unwrap().base, 600);
    }

    /// 表短于骰面是数据写歪：缺的那一档不收第 0 档的钱、也不 panic。
    /// （加载期的硬拦在 P10b 的 schema 校验，这里是运行时的退路。）
    #[test]
    fn a_short_tier_table_skips_the_missing_tier() {
        let crooked = FeePrototype::RandomAmount {
            sides: 6,
            tiers: vec![100, 200],
        };
        assert_eq!(assess(&crooked, 2, 0, Mount::Foot).unwrap().base, 200);
        assert_eq!(assess(&crooked, 3, 0, Mount::Foot), None);
    }

    /// B 型：旅馆（骰 1–4）。摇出几住几天、按天付费 ——
    /// 「踩别人旅馆既付钱又被扣住宿天数」那条验收的前一半在这里。
    #[test]
    fn prototype_b_stays_as_many_days_as_the_roll_says() {
        let hotel = FeePrototype::RandomStay {
            sides: 4,
            per_day: 500,
        };
        let bill = assess(&hotel, 2, 0, Mount::Foot).expect("B 型必有账");
        assert_eq!(bill.stay_days, 2);
        assert_eq!(bill.base, 1_000, "费用按天计");
        assert_eq!(assess(&hotel, 4, 0, Mount::Foot).unwrap().stay_days, 4);
    }

    /// C 型的载体参数：同一原型，加油站（油费，`requires_mount: true`）步行**不产生账**，
    /// IBM（维修费，`requires_mount: false`）步行照收 —— 这就是 10-20 那条拍板的回归。
    /// 骰面是 5 时两家都按 5 步算，`steps` 读的是 `DiceRolled.steps` 的骰面原值。
    #[test]
    fn prototype_c_charges_by_steps_but_only_for_vehicles_when_it_must() {
        let gas_station = FeePrototype::PerStep {
            rate: 100,
            requires_mount: true,
        };
        let ibm = FeePrototype::PerStep {
            rate: 80,
            requires_mount: false,
        };
        // 步行：加油站没这回事，IBM 照收
        assert_eq!(assess(&gas_station, 0, 5, Mount::Foot), None);
        assert_eq!(assess(&ibm, 0, 5, Mount::Foot).unwrap().base, 400);
        // 开车（机车也是车）：两家都按步数收
        assert_eq!(
            assess(&gas_station, 0, 5, Mount::Bike).unwrap().base,
            500
        );
        assert_eq!(assess(&ibm, 0, 5, Mount::Bike).unwrap().base, 400);
        // 骰面是几就按几收 —— 不乘方向、不夹乌龟（那是 walk 的事，`steps` 一直是原值）
        assert_eq!(
            assess(&gas_station, 0, 1, Mount::Car).unwrap().base,
            100
        );
    }

    /// D 型：功能型不产生账。「解锁能力」归装配层，库里只有这一句 `None`，
    /// 研究所／公园／建设公司不必各写一遍。
    #[test]
    fn prototype_d_never_bills() {
        assert_eq!(assess(&FeePrototype::Functional, 3, 5, Mount::Car), None);
        assert!(FeePrototype::Functional.is_functional());
        assert!(!mall().is_functional());
        assert_eq!(FeePrototype::Functional.sides(), 0, "D 型不摇数");
    }

    /// P10 的真验收（§四 那行原话）：**同一原型换两个参数就得出两组不同行为**。
    /// B 型换 `sides`：旅馆（骰 4 面）与航空（转盘 8 段）是同一句代码的两个参数组 ——
    /// 摇出的数直接变成停留天数，费用同一条 `per_day` 公式。
    #[test]
    fn the_same_prototype_with_two_parameter_sets_yields_two_carriers() {
        let hotel = FeePrototype::RandomStay {
            sides: 4,
            per_day: 500,
        };
        let airline = FeePrototype::RandomStay {
            sides: 8,
            per_day: 300,
        };
        // 同一个摇出的数：旅馆住 3 天付 1500，航空「出国旅游」3 天付 900 ——
        // 天数与费用都是各自参数的产物，实现只有一份
        assert_eq!(assess(&hotel, 3, 0, Mount::Foot).unwrap().stay_days, 3);
        assert_eq!(assess(&hotel, 3, 0, Mount::Foot).unwrap().base, 1_500);
        assert_eq!(assess(&airline, 3, 0, Mount::Foot).unwrap().stay_days, 3);
        assert_eq!(assess(&airline, 3, 0, Mount::Foot).unwrap().base, 900);
        // 转盘比骰子摇得远：8 段的航空摇得出 6，4 面的旅馆结构上摇不出
        assert_eq!(assess(&airline, 6, 0, Mount::Foot).unwrap().stay_days, 6);
    }
}
