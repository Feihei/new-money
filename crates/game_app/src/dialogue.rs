//! 台词正文：`.agents/CHARACTERS-dialogue.md` §四 的 12 人 × 15 句，按语音事件键索引。
//!
//! 语音只有一个「这一句是谁、说的哪件事」（[`crate::audio::VoiceEvent`]），正文不在音频里 ——
//! 字幕要读的就是这张表。键是**语音目录名**（`audio/voice/<键>/`，与 `characters.json` 的 id
//! 同一套拼写），事件键是 `VoiceEvent::file()` 的那串；两处错一个字都是「有声音、没字幕」。
//!
//! 一张表而不是按角色分文件：12 × 15 一共 180 行，打开一次就全在主存里；分文件要多一套
//! `AssetServer` 生命周期，换来的只是 diff 时少几行。盘上坏掉时整张表为空（一声不响地
//! 没字幕），不 panic —— 与 `art` 那条「缺素材退回程序化」同一条纪律。

use bevy::prelude::*;
use std::collections::HashMap;

const DIALOGUE_JSON: &str = include_str!("../../../assets/data/dialogue.json");

/// 角色键 → 事件键 → 台词。查不到就是「这个人这件事没有词」。
#[derive(Resource, Debug, Default)]
pub(crate) struct DialogueLines {
    by_key: HashMap<String, HashMap<String, String>>,
}

impl DialogueLines {
    pub(crate) fn load() -> Self {
        Self {
            by_key: serde_json::from_str(DIALOGUE_JSON).unwrap_or_default(),
        }
    }

    /// `key` 是语音目录名（[`crate::art::voice_key`] 的返回值），`event` 是
    /// [`crate::audio::VoiceEvent::file`]。两处都只认全等，拼错的后果是静默。
    pub(crate) fn line(&self, key: &str, event: &str) -> Option<&str> {
        self.by_key.get(key)?.get(event).map(String::as_str)
    }
}

pub(crate) struct DialoguePlugin;

impl Plugin for DialoguePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(DialogueLines::load());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 十五个事件一个不能少：少一个就是「这一件事只有声音」。键名与
    /// `audio::VoiceEvent::file()` 是同一套字面量，这条测试把它们钉在一起。
    const EVENTS: [&str; 15] = [
        "enter",
        "cash_big_gain",
        "cash_big_loss",
        "monopoly",
        "detained",
        "fate_gain",
        "fate_loss",
        "item_frame",
        "framed",
        "item_turn",
        "item_stay",
        "item_turtle",
        "mine",
        "bankrupt",
        "win",
    ];

    /// 十二位有声角色各十五句、句句非空，且不把舞台指示（`（埋）` 那类）
    /// 当作台词上屏 —— 括号里的动作录音里没有。
    #[test]
    fn every_voiced_character_has_fifteen_spoken_lines() {
        let lines = DialogueLines::load();
        assert_eq!(lines.by_key.len(), 12, "有声角色是十二位");
        for (key, said) in &lines.by_key {
            assert_eq!(said.len(), EVENTS.len(), "{key} 的事件句数不对");
            for event in EVENTS {
                let text = said.get(event).unwrap_or_else(|| {
                    panic!("{key} 少了 {event}");
                });
                assert!(!text.trim().is_empty(), "{key}/{event} 是空句");
                assert!(
                    !text.contains('（') && !text.contains('）'),
                    "{key}/{event} 的舞台指示不该上屏：{text}"
                );
            }
        }
    }

    /// 查表的两个键都得是「全等」：语音目录名与事件文件名各错一个字都是静默，
    /// 而静默正是这类缺陷唯一的症状。
    #[test]
    fn lookup_is_exact_on_both_keys() {
        let lines = DialogueLines::load();
        assert_eq!(
            lines.line("ji-bainian", "enter"),
            Some("诸位好。我的耐心，比这条街的历史还长——请随意。")
        );
        assert_eq!(
            lines.line("ji-bainian", "detained"),
            Some("养病嘛，正好把家族相册翻完。")
        );
        assert_eq!(lines.line("ji-bainian", "nope"), None);
        assert_eq!(lines.line("nope", "enter"), None);
        // 四恶人与无名座位没有录音，也就没有台词 —— 不在这里编一句。
        assert_eq!(lines.line("shen-wansan", "enter"), None);
    }
}
