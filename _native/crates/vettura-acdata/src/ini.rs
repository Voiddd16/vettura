//! A small, permissive INI reader.
//!
//! Assetto Corsa INI files are not fully standard: they use `;`, `#`, and
//! `//` comments, duplicate keys, and values that may contain `=` characters.
//! We keep the original order and leave unknown values alone.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ini {
    pub sections: Vec<IniSection>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IniSection {
    pub name: String,
    pub entries: Vec<IniEntry>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IniEntry {
    pub key: String,
    pub value: String,
}

impl Ini {
    pub fn parse(text: &str) -> Self {
        let mut ini = Ini::default();
        let mut current = IniSection::default();

        for raw_line in text.lines() {
            let line = raw_line.trim();
            if line.is_empty()
                || line.starts_with(';')
                || line.starts_with('#')
                || line.starts_with("//")
            {
                continue;
            }

            if let Some(name) = line
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
            {
                if !current.name.is_empty() || !current.entries.is_empty() {
                    ini.sections.push(std::mem::take(&mut current));
                }
                current.name = name.trim().to_owned();
                continue;
            }

            if let Some((key, value)) = line.split_once('=') {
                current.entries.push(IniEntry {
                    key: key.trim().to_owned(),
                    value: value.trim().to_owned(),
                });
            }
        }

        if !current.name.is_empty() || !current.entries.is_empty() {
            ini.sections.push(current);
        }

        ini
    }

    pub fn section(&self, name: &str) -> Option<&IniSection> {
        self.sections
            .iter()
            .find(|section| section.name.eq_ignore_ascii_case(name))
    }

    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.section(section)?.get(key)
    }

    pub fn get_f32(&self, section: &str, key: &str) -> Option<f32> {
        parse_f32(self.get(section, key)?)
    }

    pub fn get_i32(&self, section: &str, key: &str) -> Option<i32> {
        parse_number(self.get(section, key)?)?.parse().ok()
    }

    pub fn get_bool(&self, section: &str, key: &str) -> Option<bool> {
        match parse_number(self.get(section, key)?)?
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        }
    }
}

impl IniSection {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|entry| entry.key.eq_ignore_ascii_case(key))
            .map(|entry| entry.value.as_str())
    }

    pub fn get_f32(&self, key: &str) -> Option<f32> {
        parse_f32(self.get(key)?)
    }
}

fn parse_f32(value: &str) -> Option<f32> {
    parse_number(value)?.parse().ok()
}

/// Strip an AC inline comment from a value, returning the bare number.
///
/// Real car data is full of `SPRING_RATE=120000 ; hard springs` and
/// `LIMITER=19000 ; no limiter`; [`Ini::get`] keeps the raw text, but every
/// numeric accessor must ignore the comment or the field silently falls back
/// to its default.
pub fn strip_comment(value: &str) -> Option<&str> {
    let value = value.trim();
    let end = value
        .find(';')
        .into_iter()
        .chain(value.find('#'))
        .chain(value.find("//"))
        .min()
        .unwrap_or(value.len());
    let number = value[..end].trim();
    (!number.is_empty()).then_some(number)
}

fn parse_number(value: &str) -> Option<&str> {
    strip_comment(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sections_and_comments() {
        let ini = Ini::parse(
            r#"
; comment
[BASIC]
TOTALMASS = 1400
WHEELBASE=2.65

[ENGINE]
LIMITER = 18000 ; inline comment stays in the value
"#,
        );
        assert_eq!(ini.get_f32("basic", "totalmass"), Some(1400.0));
        assert_eq!(ini.get_f32("BASIC", "wheelbase"), Some(2.65));
        assert_eq!(
            ini.get("ENGINE", "LIMITER"),
            Some("18000 ; inline comment stays in the value")
        );
        assert!(ini.section("missing").is_none());
    }

    #[test]
    fn numeric_accessors_ignore_inline_comments() {
        let ini = Ini::parse(
            r#"
[ENGINE_DATA]
LIMITER=19000 ; no limiter
MINIMUM = 2350		# idle
[TURBO_0]
MAX_BOOST=0.5 // bar
ENABLED=1
"#,
        );
        assert_eq!(ini.get_f32("ENGINE_DATA", "LIMITER"), Some(19_000.0));
        assert_eq!(ini.get_i32("ENGINE_DATA", "MINIMUM"), Some(2_350));
        assert_eq!(ini.get_f32("TURBO_0", "MAX_BOOST"), Some(0.5));
        assert_eq!(ini.get_bool("TURBO_0", "ENABLED"), Some(true));
        // `get` itself stays raw so textual fields keep their comment.
        assert_eq!(
            ini.get("ENGINE_DATA", "LIMITER"),
            Some("19000 ; no limiter")
        );
    }
}
