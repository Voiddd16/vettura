//! AC `sfx/GUIDs.txt` 事件表解析。
//!
//! AC 每个车模的 `sfx/GUIDs.txt` 把 FMOD Studio 的 GUID 映射到事件路径，例如：
//!
//! ```text
//! {79aba212-4247-42af-b172-d4388280680f} event:/cars/cim_2008_ferrari/engine_ext
//! ```
//!
//! FMOD runtime 可以直接用 GUID 查事件，因此这里**不解 bank**、不解音频，只把
//! “事件名 → GUID”这张表读出来，交给 `vettura-audio::AudioEngine` 播放。

use std::path::Path;

/// `sfx/GUIDs.txt` 中的一个 FMOD 事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundEvent {
    /// 事件名，例如 `engine_ext`。
    pub name: String,
    /// 完整 FMOD 路径，例如 `cars/cim_2008_ferrari/engine_ext`。
    pub path: String,
    /// GUID 字符串，带花括号，例如 `{79aba212-...-d4388280680f}`。
    pub guid: String,
    /// 路径里的车 ID（全局事件为 `None`）。
    pub car_id: Option<String>,
}

impl SoundEvent {
    /// 这个事件是否属于 `car_id`（全局事件始终算属于任何车）。
    pub fn matches_car(&self, car_id: &str) -> bool {
        self.car_id.as_deref() == Some(car_id)
    }
}

/// 解析整份 `GUIDs.txt` 里的 FMOD 事件。
///
/// 非 `event:` 行（`bank:` / `bus:` / `vca:`）对播放没有直接用途，直接跳过；这样
/// 车模即使带了 Designer 时代的额外映射也不会阻塞加载。
pub fn parse_events(text: &str) -> crate::Result<Vec<SoundEvent>> {
    let mut events = Vec::new();

    for (index, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        if !line.starts_with('{') {
            continue;
        }

        let Some((guid, rest)) = split_guid(line) else {
            return Err(crate::error::AudioError::invalid_guids_line(
                index + 1,
                line,
            ));
        };
        let rest = rest.trim();
        if !rest.starts_with("event:") {
            continue;
        }

        let path = rest["event:".len()..].trim().trim_start_matches('/');
        let (car_id, name) = split_event_path(path);
        let Some(name) = name else {
            return Err(crate::error::AudioError::invalid_guids_line(
                index + 1,
                line,
            ));
        };

        events.push(SoundEvent {
            name,
            path: path.to_owned(),
            guid: format!("{{{guid}}}"),
            car_id,
        });
    }

    Ok(events)
}

/// 解析并只保留属于 `car_id` 的事件。
pub fn parse_events_for(text: &str, car_id: &str) -> crate::Result<Vec<SoundEvent>> {
    let events = parse_events(text)?;
    Ok(events
        .into_iter()
        .filter(|event| event.matches_car(car_id))
        .collect())
}

/// 车目录名即 AC 车 ID（`sfx/<id>.bank` 由它决定）。
pub fn car_id_from_root(root: &Path) -> Option<String> {
    root.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

/// `{GUID} path` 切成 GUID 与剩余路径。
fn split_guid(line: &str) -> Option<(&str, &str)> {
    let line = line.trim_start();
    if !line.starts_with('{') {
        return None;
    }
    let close = line.find('}')?;
    let guid = &line[1..close];
    if guid.len() < 32 || guid.contains(',') || guid.contains(|c: char| c.is_whitespace()) {
        return None;
    }
    Some((guid, &line[close + 1..]))
}

/// `cars/<id>/<event>` 拆出可选车 ID 与事件名。
fn split_event_path(path: &str) -> (Option<String>, Option<String>) {
    let mut parts = path.split('/');
    if parts.next() == Some("cars") {
        let car_id = parts.next().map(str::to_owned);
        let event = parts.next().map(str::to_owned);
        (car_id, event)
    } else {
        (None, path.rsplit('/').next().map(str::to_owned))
    }
}

/// 把 `79aba212-4247-42af-b172-d4388280680f` 拆成 FMOD GUID 的四段数据。
///
/// 拆成纯 Rust 类型而不是 `fmod::Guid`，这样不启用 `fmod` feature 时也能单测。
pub fn guid_parts(text: &str) -> Option<(u32, u16, u16, [u8; 8])> {
    let mut sections = text.split('-');
    let data_1 = u32::from_str_radix(sections.next()?, 16).ok()?;
    let data_2 = u16::from_str_radix(sections.next()?, 16).ok()?;
    let data_3 = u16::from_str_radix(sections.next()?, 16).ok()?;
    // `{8-4-4-4-12}`: the last two sections together are FMOD's `data_4[8]`.
    let data_4_prefix = sections.next()?;
    let data_4_tail = sections.next()?;
    if sections.next().is_some() || data_4_prefix.len() != 4 || data_4_tail.len() != 12 {
        return None;
    }

    let data_4_text = format!("{data_4_prefix}{data_4_tail}");
    let mut data_4 = [0u8; 8];
    for (index, chunk) in data_4_text.as_bytes().chunks(2).enumerate() {
        let hex = std::str::from_utf8(chunk).ok()?;
        *data_4.get_mut(index)? = u8::from_str_radix(hex, 16).ok()?;
    }

    Some((data_1, data_2, data_3, data_4))
}

#[cfg(test)]
mod tests {
    use super::{SoundEvent, guid_parts, parse_events, parse_events_for};

    #[test]
    fn parses_car_and_global_events() {
        let events = parse_events(
            &[
                "{79aba212-4247-42af-b172-d4388280680f} event:/cars/cim_2008_ferrari/engine_ext",
                "{06f2ac60-cb9b-4127-8ca9-8e2b1845542f} event:/cars/cim_2008_ferrari/engine_int",
                "{e7a0a53c-47d9-43bc-b7b8-e52d665bb16f} event:/collisions/car/hit",
                "{a1d9afa6-0a0c-4ff0-b82b-43edeba0bf8f} event:/cars/other_car/wind",
                "{94f04703-a92c-49cf-a097-734f3e79ec08} vca:/vca_engine_ext",
            ]
            .join("\n"),
        )
        .expect("GUIDs.txt should parse");

        assert_eq!(events.len(), 4);
        let engine = &events[0];
        assert_eq!(engine.name, "engine_ext");
        assert_eq!(engine.car_id.as_deref(), Some("cim_2008_ferrari"));
        assert_eq!(engine.guid, "{79aba212-4247-42af-b172-d4388280680f}");
        assert!(engine.matches_car("cim_2008_ferrari"));
        assert!(!engine.matches_car("cim_2008_mclaren"));

        assert_eq!(events[2].car_id, None);
        assert_eq!(events[2].name, "hit");

        let filtered = parse_events_for(
            &events
                .iter()
                .map(event_records)
                .collect::<Vec<_>>()
                .join("\n"),
            "cim_2008_ferrari",
        )
        .expect("filtered parse");
        assert_eq!(filtered.len(), 2);
    }

    fn event_records(event: &SoundEvent) -> String {
        format!("{} event:/{}", event.guid, event.path)
    }

    #[test]
    fn parses_guid_parts_the_way_fmod_expects_them() {
        let (data_1, data_2, data_3, data_4) =
            guid_parts("79aba212-4247-42af-b172-d4388280680f").expect("GUID should parse");
        assert_eq!(data_1, 0x79ab_a212);
        assert_eq!(data_2, 0x4247);
        assert_eq!(data_3, 0x42af);
        assert_eq!(data_4, [0xb1, 0x72, 0xd4, 0x38, 0x82, 0x80, 0x68, 0x0f]);

        assert!(guid_parts("79aba212-4247-42af").is_none());
        assert!(guid_parts("not-a-guid").is_none());
    }
}
