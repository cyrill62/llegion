//! Material icon codepoints (font registered by `egui_material_icons::initialize`).
//!
//! Macros rather than consts so they can be `concat!`ed into `&'static str` labels; `ic!`
//! joins one to a translated label.

macro_rules! icon {
    (HOME) => {
        "\u{e9b2}"
    };
    (MODELS) => {
        "\u{e1a1}"
    };
    (CHART) => {
        "\u{f190}"
    };
    (GPU) => {
        "\u{e322}"
    };
    (LOGS) => {
        "\u{e873}"
    };
    (SETTINGS) => {
        "\u{e8b8}"
    };
    (LOCK) => {
        "\u{e899}"
    };
    (LOCK_OPEN) => {
        "\u{e898}"
    };
    (KEY) => {
        "\u{e73c}"
    };
    (SEARCH) => {
        "\u{ef7a}"
    };
    (TIMER) => {
        "\u{e425}"
    };
    (DELETE) => {
        "\u{e92e}"
    };
    (REFRESH) => {
        "\u{e5d5}"
    };
    (LIGHT) => {
        "\u{e518}"
    };
    (DARK) => {
        "\u{e51c}"
    };
    (PLAY) => {
        "\u{e037}"
    };
    (STOP) => {
        "\u{e047}"
    };
    (RESTART) => {
        "\u{f053}"
    };
    (DOWNLOAD) => {
        "\u{f090}"
    };
    (INFO) => {
        "\u{e88e}"
    };
    (EJECT) => {
        "\u{e8fb}"
    };
    (COPY) => {
        "\u{e14d}"
    };
    (SAVE) => {
        "\u{e161}"
    };
    (PAUSE) => {
        "\u{e034}"
    };
    (TEST) => {
        "\u{ea4b}"
    };
    (ADD) => {
        "\u{e145}"
    };
    (UNDO) => {
        "\u{e166}"
    };
    (CLOSE) => {
        "\u{e5cd}"
    };
    (CHECK) => {
        "\u{f0be}"
    };
    (WARNING) => {
        "\u{f083}"
    };
    (LLAMA) => {
        "\u{e91d}"
    };
    (CABLE) => {
        "\u{efe6}"
    };
    (SERVER) => {
        "\u{e875}"
    };
    (PALETTE) => {
        "\u{e40a}"
    };
    (REGENERATE) => {
        "\u{e863}"
    };
    (FOLDER) => {
        "\u{e2c7}"
    };
    (TERMINAL) => {
        "\u{eb8e}"
    };
    (SPEED) => {
        "\u{e9e4}"
    };
    (TUNE) => {
        "\u{e429}"
    };
    (CHAT) => {
        "\u{e0c9}"
    };
    (SEND) => {
        "\u{e163}"
    };
    (EDIT) => {
        "\u{f097}"
    };
    (PERSON) => {
        "\u{f0d3}"
    };
    (BOT) => {
        "\u{f06c}"
    };
    (PREV) => {
        "\u{e5cb}"
    };
    (NEXT) => {
        "\u{e5cc}"
    };
    (THINK) => {
        "\u{ea4a}"
    };
    (NEW_CHAT) => {
        "\u{e266}"
    };
    (PANEL_CLOSE) => {
        "\u{f717}"
    };
    (PANEL_OPEN) => {
        "\u{f716}"
    };
}

/// `ic!(SAVE, "Save")` → icon followed by a label.
/// Icon + translated label.
macro_rules! ic {
    ($icon:ident, $text:literal) => {
        $crate::i18n::with_icon(icon!($icon), $text)
    };
}
