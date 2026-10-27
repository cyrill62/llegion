//! Interface translations. The English text is the key: `tr("Save")` looks it up in the
//! embedded `locales/<code>.json` of the current language and falls back to English.
//! `trf!` fills `{name}` placeholders after translating. Both front-ends share it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

/// Code and native name of each interface language; English first.
pub const LANGUAGES: [(&str, &str); 6] = [
    ("en", "English"),
    ("fr", "Français"),
    ("de", "Deutsch"),
    ("es", "Español"),
    ("it", "Italiano"),
    ("zh", "中文"),
];

const FILES: [&str; 5] = [
    include_str!("../locales/fr.json"),
    include_str!("../locales/de.json"),
    include_str!("../locales/es.json"),
    include_str!("../locales/it.json"),
    include_str!("../locales/zh.json"),
];

/// Index in `LANGUAGES` of the language in use.
static CURRENT: AtomicUsize = AtomicUsize::new(0);

fn table(lang: usize) -> &'static HashMap<String, String> {
    static TABLES: [OnceLock<HashMap<String, String>>; 5] = [const { OnceLock::new() }; 5];
    TABLES[lang - 1].get_or_init(|| serde_json::from_str(FILES[lang - 1]).unwrap_or_default())
}

/// The language the OS asks for, when it is one of ours; English otherwise.
pub fn system_language() -> &'static str {
    let locale = sys_locale::get_locale()
        .unwrap_or_default()
        .to_ascii_lowercase();
    LANGUAGES
        .iter()
        .map(|(code, _)| *code)
        .find(|code| locale.starts_with(code))
        .unwrap_or("en")
}

/// Switches the interface language; an empty or unknown code follows the system.
pub fn set_language(code: &str) {
    let code = if code.is_empty() {
        system_language()
    } else {
        code
    };
    let index = LANGUAGES.iter().position(|(c, _)| *c == code).unwrap_or(0);
    CURRENT.store(index, Ordering::Relaxed);
}

/// Code of the language in use.
pub fn current() -> &'static str {
    LANGUAGES[CURRENT.load(Ordering::Relaxed)].0
}

/// `text` in the current language.
pub fn tr(text: &str) -> &str {
    match CURRENT.load(Ordering::Relaxed) {
        0 => text,
        lang => table(lang).get(text).map_or(text, String::as_str),
    }
}

/// `icon` and the translated `text`, as one label (behind `ic!`). Interned: there is one
/// string per label and language, so they can be handed out as `&'static str`.
pub fn with_icon(icon: &'static str, text: &'static str) -> &'static str {
    type Labels = HashMap<(usize, &'static str, &'static str), &'static str>;
    static LABELS: OnceLock<Mutex<Labels>> = OnceLock::new();
    let lang = CURRENT.load(Ordering::Relaxed);
    let mut labels = LABELS.get_or_init(Default::default).lock().unwrap();
    labels
        .entry((lang, icon, text))
        .or_insert_with(|| format!("{icon}  {}", tr(text)).leak())
}

/// Marks a literal for translation where `tr` can't run (consts); translate it with `tr`
/// where it is shown.
macro_rules! tr_key {
    ($text:literal) => {
        $text
    };
}

/// `trf!("Pull {model}", model = name)`: translates, then fills the placeholders.
macro_rules! trf {
    ($text:literal $(, $name:ident = $value:expr)* $(,)?) => {{
        #[allow(unused_mut)]
        let mut s = $crate::i18n::tr($text).to_string();
        $(s = s.replace(concat!("{", stringify!($name), "}"), &$value.to_string());)*
        s
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every literal passed to `tr`, `trf!` or `ic!` in the sources.
    fn keys() -> Vec<String> {
        let mut keys = Vec::new();
        let mut files = vec![std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")];
        while let Some(path) = files.pop() {
            if path.is_dir() {
                files.extend(std::fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") || path.ends_with("i18n.rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            for marker in ["tr(", "trf!(", "tr_key!("] {
                for (at, _) in source.match_indices(marker) {
                    // Not `push_str(…)` and the like.
                    if source[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_') {
                        continue;
                    }
                    // rustfmt may put the literal on the next line.
                    let rest = source[at + marker.len()..].trim_start();
                    if let Some(rest) = rest.strip_prefix('"') {
                        keys.push(literal(rest));
                    }
                }
            }
            // ic!(ICON, "text")
            for (at, _) in source.match_indices("ic!(") {
                if source[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                let rest = &source[at + 4..];
                if let Some(quote) = rest.find('"')
                    && !rest[..quote].contains(')')
                {
                    keys.push(literal(&rest[quote + 1..]));
                }
            }
        }
        keys.sort();
        keys.dedup();
        keys
    }

    /// The Rust string literal starting at `rest` (after its opening quote), unescaped.
    fn literal(rest: &str) -> String {
        let mut out = String::new();
        let mut chars = rest.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('u') => {
                        let hex: String =
                            chars.by_ref().skip(1).take_while(|c| *c != '}').collect();
                        out.extend(char::from_u32(u32::from_str_radix(&hex, 16).unwrap()));
                    }
                    Some(c) => out.push(c),
                    None => {}
                },
                c => out.push(c),
            }
        }
        out
    }

    fn placeholders(text: &str) -> Vec<&str> {
        let mut found: Vec<&str> = text
            .split('{')
            .skip(1)
            .filter_map(|s| s.split_once('}').map(|(name, _)| name))
            .collect();
        found.sort();
        found
    }

    #[test]
    fn every_string_is_translated() {
        let keys = keys();
        assert!(keys.len() > 100, "found only {} strings", keys.len());
        for (lang, (code, _)) in LANGUAGES.iter().enumerate().skip(1) {
            let table = table(lang);
            assert!(!table.is_empty(), "locales/{}.json does not parse", code);
            let missing: Vec<&String> = keys.iter().filter(|k| !table.contains_key(*k)).collect();
            assert!(missing.is_empty(), "{}: missing {missing:#?}", code);
            for (key, value) in table {
                assert_eq!(
                    placeholders(key),
                    placeholders(value),
                    "{}: placeholders differ in {key:?}",
                    code
                );
            }
        }
    }
}
