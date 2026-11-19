#![allow(dead_code)]

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub const APP_BRAND_NAME: &str = "KeySwitch";
pub const APP_VERSION: &str = "2.0";
pub const APP_AUTHOR: &str = "KeySwitch";

pub const DEFAULT_QUOTA: f64 = 0.12;
pub const DEFAULT_MIN_MIDWORD_LEN: usize = 4;
pub const DEFAULT_MAX_THRESHOLD: usize = 8;
pub const ADD_DISTANCE: f64 = 1.0;
pub const DELETE_DISTANCE: f64 = 1.0;
pub const DEFAULT_ADJACENT_COLUMN_DISTANCE: f64 = 0.15;
pub const DEFAULT_ADJACENT_ROW_DISTANCE: f64 = 0.70;
pub const DEFAULT_SHIFT_DISTANCE: f64 = 0.40;
pub const DEFAULT_CHANGE_DISTANCE: f64 = 1.00;
pub const DEFAULT_MAX_DEPTH: usize = 3;
pub const DEFAULT_TOLERANCE: usize = 3;
pub const NOTIFIER_DEFAULT_OPACITY: f32 = 0.92;
pub const NOTIFIER_DISPLAY_INTERVAL_MS: f32 = 2600.0;
pub const ENG_TRIM_END: &[char] = &['.', ',', ';', ':', '!', '?', ' '];
pub const THAI_TRIM_END: &[char] = &['ๆ', 'ฯ', '.', ',', ';', ':', '!', '?', ' '];

pub const EMBEDDED_ADJACENT_COL: &str = include_str!("../data/adjacent-column.txt");
pub const EMBEDDED_SHIFT: &str = include_str!("../data/shift.txt");
pub const EMBEDDED_ENG_DICT: &str = include_str!("../dict/eng.txt");
pub const EMBEDDED_THAI_DICT: &str = include_str!("../dict/thai.txt");
pub const EMBEDDED_USER_EN: &str = include_str!("../dict/user.en.txt");
pub const EMBEDDED_USER_TH: &str = include_str!("../dict/user.th.txt");

const COMMON_SHORT_EN: &[&str] = &[
    "am", "an", "as", "at", "be", "by", "do", "go", "he", "hi", "if", "in", "is", "it", "me", "my",
    "no", "of", "ok", "on", "or", "so", "to", "up", "us", "we",
];

const COMMON_MODERN_EN: &[&str] = &[
    "discord", "youtube", "github", "chatgpt", "facebook", "instagram", "tiktok", "spotify",
    "netflix", "vscode", "rust", "docker", "typescript", "javascript", "valorant", "roblox",
    "minecraft", "reddit", "twitter", "gmail", "logout", "wifi", "bluetooth", "iphone", "android",
    "deploy", "npm", "git", "sql", "api", "json", "html", "css", "http", "https", "url", "app",
    "apps", "uninstall", "restart", "shutdown", "figma", "canva", "shopee", "lazada", "prompt",
    "claude", "openai", "username", "keyswitch", "sudo", "cursor", "antigravity", "gemini",
    "supabase", "vercel", "netlify", "firebase", "cloudflare", "tailwind", "nextjs", "nodejs",
    "golang", "kotlin", "flutter", "ubuntu", "kubernetes", "terraform", "graphql", "oauth",
    "regex", "async", "await", "mutex", "struct", "cargo", "localhost", "frontend", "backend",
    "fullstack", "webhook", "markdown", "refactor", "workflow", "sandbox", "tooltip", "checkbox",
    "dropdown", "autostart", "clipboard", "esports", "podcast", "emoji", "avatar", "unmute",
    "unban", "webcam", "headset", "mousepad", "router", "modem", "torrent", "bitrate", "ffmpeg",
    "anime", "manga", "cosplay", "meme", "memes", "vlog", "diff", "func", "repo", "stash",
    "rebase", "lint", "vpn", "dns", "ssl", "ssh", "cli", "gui", "ide", "llm", "cpu", "ram",
    "ssd", "usb", "fps", "afk", "obs", "pdf", "zip", "exe", "apk", "png", "jpg", "svg", "gif",
    "twitch", "steam", "nvidia", "slack", "notion", "zoom", "chrome",
];

pub fn is_sane_user_word_en(w: &str) -> bool {
    let len = w.len();
    if !(4..=24).contains(&len) || !w.bytes().all(|b| b.is_ascii_alphabetic()) {
        return false;
    }
    let mut prev = 0u8;
    let mut run = 0u8;
    let mut has_vowel = false;
    let mut counts = [0u8; 26];
    for b in w.bytes() {
        let lb = b.to_ascii_lowercase();
        if lb == prev {
            run += 1;
            if run >= 3 {
                return false;
            }
        } else {
            prev = lb;
            run = 1;
        }
        if matches!(lb, b'a' | b'e' | b'i' | b'o' | b'u' | b'y') {
            has_vowel = true;
        }
        counts[(lb - b'a') as usize] += 1;
    }
    if !has_vowel {
        return false;
    }
    if len >= 6 && counts.iter().any(|&c| (c as usize) > len / 2) {
        return false;
    }
    true
}

pub fn is_sane_user_word_th(w: &str) -> bool {
    let len = w.chars().count();
    if !(2..=24).contains(&len) || !has_valid_thai_sequence(w) || w.contains('ๅ') {
        return false;
    }
    let mut chars = w.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let fu = first as u32;
    if !((0x0E01..=0x0E2E).contains(&fu) || (0x0E40..=0x0E44).contains(&fu)) {
        return false;
    }
    let mut has_consonant = (0x0E01..=0x0E2E).contains(&fu);
    let mut prev = first;
    let mut run = 1u8;
    for c in chars {
        if (0x0E01..=0x0E2E).contains(&(c as u32)) {
            has_consonant = true;
        }
        if c == prev {
            run += 1;
            if run >= 4 {
                return false;
            }
        } else {
            prev = c;
            run = 1;
        }
    }
    has_consonant
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    English,
    Thai,
}

impl Language {
    pub fn opposite(self) -> Self {
        match self {
            Language::English => Language::Thai,
            Language::Thai => Language::English,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorrectionMode {
    Disabled = 0,
    Offline = 1,
    Online = 2,
}

impl CorrectionMode {
    pub fn label(self, thai_ui: bool) -> &'static str {
        match (self, thai_ui) {
            (CorrectionMode::Online, false) => "Auto Mode",
            (CorrectionMode::Offline, false) => "Manual Mode",
            (CorrectionMode::Disabled, false) => "Disabled",
            (CorrectionMode::Online, true) => "แก้ไขอัตโนมัติ",
            (CorrectionMode::Offline, true) => "แก้ไขโดยผู้ใช้",
            (CorrectionMode::Disabled, true) => "ไม่ใช้งาน",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceCheckingRule {
    None = 0,
    Basic = 1,
    Replace = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharClass {
    Consonant = 0,
    LeadingVowel = 1,
    AboveVowel = 2,
    BelowVowel = 3,
    FollowingVowel = 4,
    ToneMark = 5,
    Special = 6,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ThaiCell {
    pub consonant: Option<char>,
    pub vowel: Option<char>,
    pub tone_mark: Option<char>,
}

impl ThaiCell {
    pub fn clear(&mut self) {
        self.consonant = None;
        self.vowel = None;
        self.tone_mark = None;
    }
}

pub fn get_character_class(c: char, rule: SequenceCheckingRule) -> CharClass {
    let u = c as u32;
    if (0x0E40..=0x0E44).contains(&u) {
        CharClass::LeadingVowel
    } else if u == 0x0E31 || (0x0E34..=0x0E37).contains(&u) || u == 0x0E47 || u == 0x0E4D {
        CharClass::AboveVowel
    } else if (0x0E38..=0x0E3A).contains(&u) {
        CharClass::BelowVowel
    } else if u == 0x0E30
        || u == 0x0E32
        || u == 0x0E45
        || (rule == SequenceCheckingRule::Replace && u == 0x0E33)
    {
        CharClass::FollowingVowel
    } else if (0x0E48..=0x0E4B).contains(&u) {
        CharClass::ToneMark
    } else if (0x0E01..=0x0E2E).contains(&u) {
        CharClass::Consonant
    } else {
        CharClass::Special
    }
}

pub fn is_thai_vowel_char(c: char) -> bool {
    matches!(
        get_character_class(c, SequenceCheckingRule::None),
        CharClass::LeadingVowel
            | CharClass::AboveVowel
            | CharClass::BelowVowel
            | CharClass::FollowingVowel
    ) || c == '\u{0E33}'
}

pub fn is_thai_tone_char(c: char) -> bool {
    get_character_class(c, SequenceCheckingRule::None) == CharClass::ToneMark
}

pub fn merge_current_cell(cell: &mut ThaiCell, c: char, rule: SequenceCheckingRule) -> i32 {
    if rule == SequenceCheckingRule::None {
        return 1;
    }
    let class = get_character_class(c, rule);
    if class == CharClass::Consonant {
        cell.consonant = Some(c);
        cell.vowel = None;
        cell.tone_mark = None;
        return 1;
    }
    if class == CharClass::LeadingVowel || c == '\u{0E2F}' {
        cell.clear();
        return 1;
    }
    if c == '\u{0E4C}' {
        if cell.tone_mark.is_some()
            && cell.vowel.is_some()
            && rule == SequenceCheckingRule::Replace
        {
            cell.vowel = None;
            cell.tone_mark = Some(c);
            return -1;
        }
        if cell.consonant.is_some()
            && cell.tone_mark.is_none()
            && (cell.vowel.is_none()
                || get_character_class(cell.vowel.unwrap_or(' '), rule)
                    != CharClass::FollowingVowel)
        {
            cell.tone_mark = Some(c);
            return 1;
        }
        if (cell.tone_mark.is_some() || cell.vowel.is_some())
            && rule == SequenceCheckingRule::Replace
        {
            cell.vowel = None;
            cell.tone_mark = Some(c);
            return 0;
        }
        return 0;
    }
    if c == '\u{0E4D}' {
        if cell.tone_mark.is_some()
            && cell.vowel.is_some()
            && rule == SequenceCheckingRule::Replace
        {
            cell.vowel = Some(c);
            cell.tone_mark = None;
            return -1;
        }
        if cell.consonant.is_some() && cell.vowel.is_none() && cell.tone_mark.is_none() {
            cell.vowel = Some(c);
            return 1;
        }
        if (cell.tone_mark.is_some() || cell.vowel.is_some())
            && rule == SequenceCheckingRule::Replace
        {
            cell.vowel = Some(c);
            cell.tone_mark = None;
            return 0;
        }
        return 0;
    }
    if class == CharClass::Special {
        if cell.consonant.is_some()
            && (c != '\u{0E33}'
                || (cell.vowel.is_none() && (c != '\u{0E45}' || cell.tone_mark.is_none())))
        {
            cell.clear();
            return 1;
        }
        return 0;
    }
    if class == CharClass::FollowingVowel {
        if (c == '\u{0E30}' || c == '\u{0E45}')
            && cell.consonant.is_some()
            && (cell.vowel.is_none()
                || get_character_class(cell.vowel.unwrap_or(' '), rule)
                    != CharClass::FollowingVowel)
        {
            cell.clear();
            return 1;
        }
        if cell.consonant.is_some() && cell.vowel.is_none() {
            cell.vowel = Some(c);
            return 1;
        }
        if cell.vowel.is_some() && rule == SequenceCheckingRule::Replace {
            cell.vowel = Some(c);
            return 0;
        }
        return 0;
    }
    if class == CharClass::AboveVowel || class == CharClass::BelowVowel {
        if c == '\u{0E47}' {
            if cell.consonant.is_some() && cell.tone_mark.is_none() && cell.vowel.is_none() {
                cell.vowel = Some(c);
                return 1;
            }
            if cell.tone_mark.is_some()
                && cell.vowel.is_some()
                && rule == SequenceCheckingRule::Replace
            {
                cell.vowel = Some(c);
                cell.tone_mark = None;
                return -1;
            }
            if (cell.tone_mark.is_some() || cell.vowel.is_some())
                && rule == SequenceCheckingRule::Replace
            {
                cell.vowel = Some(c);
                cell.tone_mark = None;
                return 0;
            }
            return 0;
        }
        if cell.consonant.is_some() && cell.vowel.is_none() && cell.tone_mark.is_none() {
            cell.vowel = Some(c);
            return 1;
        }
        if cell.vowel.is_some() && rule == SequenceCheckingRule::Replace {
            cell.vowel = Some(c);
            return 0;
        }
        return 0;
    }
    if class == CharClass::ToneMark {
        if cell.consonant.is_some()
            && cell.tone_mark.is_none()
            && (cell.vowel.is_none()
                || get_character_class(cell.vowel.unwrap_or(' '), rule)
                    != CharClass::FollowingVowel)
        {
            cell.tone_mark = Some(c);
            return 1;
        }
        if cell.tone_mark.is_some() && rule == SequenceCheckingRule::Replace {
            cell.tone_mark = Some(c);
            return 0;
        }
        return 0;
    }
    1
}

const US_TO_THAI_PAIRS: &[(char, char)] = &[
    ('`', '_'),
    ('1', 'ๅ'),
    ('2', '/'),
    ('3', '-'),
    ('4', 'ภ'),
    ('5', 'ถ'),
    ('6', 'ุ'),
    ('7', 'ึ'),
    ('8', 'ค'),
    ('9', 'ต'),
    ('0', 'จ'),
    ('-', 'ข'),
    ('=', 'ช'),
    ('~', '%'),
    ('!', '+'),
    ('@', '๑'),
    ('#', '๒'),
    ('$', '๓'),
    ('%', '๔'),
    ('^', 'ู'),
    ('&', '฿'),
    ('*', '๕'),
    ('(', '๖'),
    (')', '๗'),
    ('_', '๘'),
    ('+', '๙'),
    ('q', 'ๆ'),
    ('w', 'ไ'),
    ('e', 'ำ'),
    ('r', 'พ'),
    ('t', 'ะ'),
    ('y', 'ั'),
    ('u', 'ี'),
    ('i', 'ร'),
    ('o', 'น'),
    ('p', 'ย'),
    ('[', 'บ'),
    (']', 'ล'),
    ('\\', 'ฃ'),
    ('Q', '๐'),
    ('W', '"'),
    ('E', 'ฎ'),
    ('R', 'ฑ'),
    ('T', 'ธ'),
    ('Y', 'ํ'),
    ('U', '๊'),
    ('I', 'ณ'),
    ('O', 'ฯ'),
    ('P', 'ญ'),
    ('{', 'ฐ'),
    ('}', ','),
    ('|', 'ฅ'),
    ('a', 'ฟ'),
    ('s', 'ห'),
    ('d', 'ก'),
    ('f', 'ด'),
    ('g', 'เ'),
    ('h', '้'),
    ('j', '่'),
    ('k', 'า'),
    ('l', 'ส'),
    (';', 'ว'),
    ('\'', 'ง'),
    ('A', 'ฤ'),
    ('S', 'ฆ'),
    ('D', 'ฏ'),
    ('F', 'โ'),
    ('G', 'ฌ'),
    ('H', '็'),
    ('J', '๋'),
    ('K', 'ษ'),
    ('L', 'ศ'),
    (':', 'ซ'),
    ('"', '.'),
    ('z', 'ผ'),
    ('x', 'ป'),
    ('c', 'แ'),
    ('v', 'อ'),
    ('b', 'ิ'),
    ('n', 'ื'),
    ('m', 'ท'),
    (',', 'ม'),
    ('.', 'ใ'),
    ('/', 'ฝ'),
    ('Z', '('),
    ('X', ')'),
    ('C', 'ฉ'),
    ('V', 'ฮ'),
    ('B', 'ฺ'),
    ('N', '์'),
    ('M', '?'),
    ('<', 'ฒ'),
    ('>', 'ฬ'),
    ('?', 'ฦ'),
];

const US_TO_THAI_LUT: [char; 128] = build_us_to_thai_lut();
const THAI_TO_US_LUT: [char; 128] = build_thai_to_us_lut();
const THAI_ASCII_TO_US_LUT: [char; 128] = build_thai_ascii_to_us_lut();

const fn build_us_to_thai_lut() -> [char; 128] {
    let mut lut = ['\0'; 128];
    let mut i = 0;
    while i < 128 {
        lut[i] = (i as u8) as char;
        i += 1;
    }
    let mut j = 0;
    while j < US_TO_THAI_PAIRS.len() {
        let (en, th) = US_TO_THAI_PAIRS[j];
        let code = en as usize;
        if code < 128 {
            lut[code] = th;
        }
        j += 1;
    }
    lut
}

const fn build_thai_to_us_lut() -> [char; 128] {
    let mut lut = ['\0'; 128];
    let mut i = 0;
    while i < 128 {
        if let Some(c) = char::from_u32(0x0E00 + (i as u32)) {
            lut[i] = c;
        }
        i += 1;
    }
    let mut j = US_TO_THAI_PAIRS.len();
    while j > 0 {
        j -= 1;
        let (en, th) = US_TO_THAI_PAIRS[j];
        let code = th as usize;
        if code >= 0x0E00 && code <= 0x0E7F {
            lut[code - 0x0E00] = en;
        }
    }
    lut
}

const fn build_thai_ascii_to_us_lut() -> [char; 128] {
    let mut lut = ['\0'; 128];
    let mut i = 0;
    while i < 128 {
        lut[i] = (i as u8) as char;
        i += 1;
    }
    let mut j = US_TO_THAI_PAIRS.len();
    while j > 0 {
        j -= 1;
        let (en, th) = US_TO_THAI_PAIRS[j];
        let code = th as usize;
        if code < 128 {
            lut[code] = en;
        }
    }
    lut
}

#[inline]
pub fn us_to_thai_char(c: char) -> char {
    let u = c as usize;
    if u < 128 {
        US_TO_THAI_LUT[u]
    } else {
        c
    }
}

#[inline]
pub fn thai_to_us_char(c: char) -> char {
    let u = c as usize;
    if u >= 0x0E00 && u <= 0x0E7F {
        THAI_TO_US_LUT[u - 0x0E00]
    } else if u < 128 {
        THAI_ASCII_TO_US_LUT[u]
    } else {
        c
    }
}

#[inline]
fn is_thai_combining_mark(u: u32) -> bool {
    u == 0x0E31 || (0x0E34..=0x0E3A).contains(&u) || (0x0E47..=0x0E4E).contains(&u)
}

#[inline]
fn is_thai_above_below_vowel(u: u32) -> bool {
    u == 0x0E31 || (0x0E34..=0x0E3A).contains(&u) || u == 0x0E47
}

pub fn has_valid_thai_sequence(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if is_thai_combining_mark(first as u32) {
        return false;
    }
    let mut prev_u = first as u32;
    for c in chars {
        let u = c as u32;
        if (0x0E48..=0x0E4B).contains(&prev_u) && (0x0E48..=0x0E4B).contains(&u) {
            return false;
        }
        if is_thai_above_below_vowel(prev_u) && is_thai_above_below_vowel(u) {
            return false;
        }
        prev_u = u;
    }
    true
}

#[inline]
pub fn strip_surrounding_punct(s: &str) -> &str {
    s.trim_start_matches(['(', '[', '{', '"', '\'', '“', '‘'])
        .trim_end_matches([
            '.', ',', ';', ':', '!', '?', ')', ']', '}', '"', '\'', '”', '’', 'ๆ', 'ฯ',
        ])
}

#[inline]
pub fn convert_char_language(c: char) -> char {
    if (c as u32) >= 0x0E00 && (c as u32) <= 0x0E7F {
        thai_to_us_char(c)
    } else {
        let mapped = us_to_thai_char(c);
        if mapped != c {
            mapped
        } else {
            thai_to_us_char(c)
        }
    }
}

pub fn convert_string_language(s: &str) -> String {
    s.chars().map(convert_char_language).collect()
}

pub fn map_keystroke_to_both(c: char, _active_lang: Language) -> (char, char) {
    if (c as u32) >= 0x0E00 && (c as u32) <= 0x0E7F {
        (thai_to_us_char(c), c)
    } else {
        (c, us_to_thai_char(c))
    }
}

pub fn vk_to_us_char(vk: u32, shift: bool, caps_lock: bool) -> Option<char> {
    let upper = shift ^ caps_lock;
    match vk {
        0x41..=0x5A => {
            let base = (b'a' + (vk as u8 - 0x41)) as char;
            Some(if upper {
                base.to_ascii_uppercase()
            } else {
                base
            })
        }
        0x30 => Some(if shift { ')' } else { '0' }),
        0x31 => Some(if shift { '!' } else { '1' }),
        0x32 => Some(if shift { '@' } else { '2' }),
        0x33 => Some(if shift { '#' } else { '3' }),
        0x34 => Some(if shift { '$' } else { '4' }),
        0x35 => Some(if shift { '%' } else { '5' }),
        0x36 => Some(if shift { '^' } else { '6' }),
        0x37 => Some(if shift { '&' } else { '7' }),
        0x38 => Some(if shift { '*' } else { '8' }),
        0x39 => Some(if shift { '(' } else { '9' }),
        0xBA => Some(if shift { ':' } else { ';' }),
        0xBB => Some(if shift { '+' } else { '=' }),
        0xBC => Some(if shift { '<' } else { ',' }),
        0xBD => Some(if shift { '_' } else { '-' }),
        0xBE => Some(if shift { '>' } else { '.' }),
        0xBF => Some(if shift { '?' } else { '/' }),
        0xC0 => Some(if shift { '~' } else { '`' }),
        0xDB => Some(if shift { '{' } else { '[' }),
        0xDC => Some(if shift { '|' } else { '\\' }),
        0xDD => Some(if shift { '}' } else { ']' }),
        0xDE => Some(if shift { '"' } else { '\'' }),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharDistanceKind {
    AdjacentColumn = 1,
    AdjacentRow = 2,
    Shift = 3,
}

#[derive(Debug, Clone)]
pub struct DistanceConfig {
    pub quota: f64,
    pub max_threshold: usize,
    pub adjacent_column_distance: f64,
    pub adjacent_row_distance: f64,
    pub shift_distance: f64,
    pub change_distance: f64,
}

impl Default for DistanceConfig {
    fn default() -> Self {
        Self {
            quota: DEFAULT_QUOTA,
            max_threshold: DEFAULT_MAX_THRESHOLD,
            adjacent_column_distance: DEFAULT_ADJACENT_COLUMN_DISTANCE,
            adjacent_row_distance: DEFAULT_ADJACENT_ROW_DISTANCE,
            shift_distance: DEFAULT_SHIFT_DISTANCE,
            change_distance: DEFAULT_CHANGE_DISTANCE,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct FilterEval {
    pub is_exact_match: bool,
    pub is_close_match: bool,
    pub is_close_prefix_match: bool,
}

#[derive(Debug, Clone, Copy)]
struct WordSlice {
    offset: u32,
    len: u16,
}

#[derive(Debug, Clone)]
struct FlatTrieNode {
    ch: char,
    first_child: u32,
    next_sibling: u32,
    word_ids: Vec<u32>,
}

const INVALID_NODE: u32 = u32::MAX;

pub struct WordDictFilter {
    pub language: Language,
    pub user_words: BTreeSet<String>,
    pub word_set: HashSet<String>,
    char_pool: Vec<char>,
    word_slices: Vec<WordSlice>,
    trie_nodes: Vec<FlatTrieNode>,
    pub buffer: String,
    pub last_eval: FilterEval,
}

impl WordDictFilter {
    pub fn new(language: Language, base_words: Vec<String>, user_words: Vec<String>) -> Self {
        let mut filter = Self {
            language,
            user_words: BTreeSet::new(),
            word_set: HashSet::new(),
            char_pool: Vec::new(),
            word_slices: Vec::new(),
            trie_nodes: Vec::new(),
            buffer: String::with_capacity(32),
            last_eval: FilterEval::default(),
        };
        filter.rebuild(base_words, user_words);
        filter
    }

    pub fn rebuild(&mut self, base_words: Vec<String>, user_words: Vec<String>) {
        self.user_words.clear();
        self.word_set.clear();
        self.char_pool.clear();
        self.word_slices.clear();
        self.trie_nodes.clear();

        self.trie_nodes.push(FlatTrieNode {
            ch: '\0',
            first_child: INVALID_NODE,
            next_sibling: INVALID_NODE,
            word_ids: Vec::new(),
        });

        let total_cap =
            base_words.len() + user_words.len() + COMMON_SHORT_EN.len() + COMMON_MODERN_EN.len();
        self.word_set.reserve(total_cap);
        self.word_slices.reserve(total_cap);
        self.char_pool.reserve(total_cap * 6);
        self.trie_nodes.reserve(4096);

        for w in base_words {
            let trimmed = w.trim().to_lowercase();
            if !trimmed.is_empty() && self.word_set.insert(trimmed.clone()) {
                self.insert_into_arena_and_trie(&trimmed);
            }
        }
        if self.language == Language::English {
            for &short_w in COMMON_SHORT_EN.iter().chain(COMMON_MODERN_EN.iter()) {
                if self.word_set.insert(short_w.to_string()) {
                    self.insert_into_arena_and_trie(short_w);
                }
            }
        }
        for w in user_words {
            let trimmed = w.trim().to_lowercase();
            let valid = match self.language {
                Language::English => is_sane_user_word_en(&trimmed),
                Language::Thai => is_sane_user_word_th(&trimmed),
            };
            if valid {
                if self.word_set.insert(trimmed.clone()) {
                    self.insert_into_arena_and_trie(&trimmed);
                }
                self.user_words.insert(trimmed);
            }
        }
        self.word_set.shrink_to_fit();
        self.char_pool.shrink_to_fit();
        self.word_slices.shrink_to_fit();
        for node in &mut self.trie_nodes {
            node.word_ids.shrink_to_fit();
        }
        self.trie_nodes.shrink_to_fit();
    }

    fn insert_into_arena_and_trie(&mut self, word: &str) {
        let offset = self.char_pool.len() as u32;
        let mut char_len = 0u16;
        let mut prefix_chars = ['\0'; DEFAULT_MAX_DEPTH];
        for c in word.chars() {
            let idx = char_len as usize;
            if idx < DEFAULT_MAX_DEPTH {
                prefix_chars[idx] = c;
            }
            self.char_pool.push(c);
            char_len = char_len.saturating_add(1);
        }
        if char_len == 0 {
            return;
        }
        let word_id = self.word_slices.len() as u32;
        self.word_slices.push(WordSlice {
            offset,
            len: char_len,
        });

        let depth = (char_len as usize).min(DEFAULT_MAX_DEPTH);
        let mut curr_node = 0u32;
        for &pch in &prefix_chars[..depth] {
            let mut child = self.trie_nodes[curr_node as usize].first_child;
            let mut found = INVALID_NODE;
            while child != INVALID_NODE {
                if self.trie_nodes[child as usize].ch == pch {
                    found = child;
                    break;
                }
                child = self.trie_nodes[child as usize].next_sibling;
            }
            if found != INVALID_NODE {
                curr_node = found;
            } else {
                let new_idx = self.trie_nodes.len() as u32;
                let old_first = self.trie_nodes[curr_node as usize].first_child;
                self.trie_nodes.push(FlatTrieNode {
                    ch: pch,
                    first_child: INVALID_NODE,
                    next_sibling: old_first,
                    word_ids: Vec::new(),
                });
                self.trie_nodes[curr_node as usize].first_child = new_idx;
                curr_node = new_idx;
            }
        }
        self.trie_nodes[curr_node as usize].word_ids.push(word_id);
    }

    #[inline]
    fn get_word_chars(&self, word_id: u32) -> &[char] {
        let slice = self.word_slices[word_id as usize];
        let start = slice.offset as usize;
        let end = start + (slice.len as usize);
        &self.char_pool[start..end]
    }

    pub fn add_user_word(&mut self, word: &str) -> bool {
        let trimmed = word.trim().to_lowercase();
        if trimmed.is_empty() || self.word_set.contains(&trimmed) {
            return false;
        }
        self.word_set.insert(trimmed.clone());
        self.insert_into_arena_and_trie(&trimmed);
        self.user_words.insert(trimmed);
        true
    }

    #[inline]
    pub fn contains_lower(&self, s: &str) -> bool {
        if !s.bytes().any(|b| b.is_ascii_uppercase()) {
            self.word_set.contains(s)
        } else {
            self.word_set.contains(&s.to_ascii_lowercase())
        }
    }

    pub fn clear_buffer(&mut self) {
        self.buffer.clear();
        self.last_eval = FilterEval::default();
    }

    pub fn has_prefix(&self, prefix: &str) -> bool {
        let mut pchars = ['\0'; 64];
        let mut plen = 0usize;
        for c in prefix.chars() {
            for lc in c.to_lowercase() {
                if plen >= 64 {
                    return false;
                }
                pchars[plen] = lc;
                plen += 1;
            }
        }
        if plen == 0 {
            return false;
        }
        let depth = plen.min(DEFAULT_MAX_DEPTH);
        let mut curr_node = 0u32;
        for &pch in &pchars[..depth] {
            let mut child = self.trie_nodes[curr_node as usize].first_child;
            let mut found = INVALID_NODE;
            while child != INVALID_NODE {
                if self.trie_nodes[child as usize].ch == pch {
                    found = child;
                    break;
                }
                child = self.trie_nodes[child as usize].next_sibling;
            }
            if found == INVALID_NODE {
                return false;
            }
            curr_node = found;
        }
        if plen <= DEFAULT_MAX_DEPTH {
            return true;
        }
        let word_ids = &self.trie_nodes[curr_node as usize].word_ids;
        for &wid in word_ids {
            let w_chars = self.get_word_chars(wid);
            if w_chars.len() >= plen && w_chars[..plen] == pchars[..plen] {
                return true;
            }
        }
        false
    }

    pub fn try_compose(&self, s: &str) -> bool {
        let mut offsets = [0usize; 129];
        let Some(n) = collect_char_offsets(s, &mut offsets) else {
            return false;
        };
        let mut memo = [0u8; 129];
        self.try_compose_from(s, &offsets[..=n], 0, &mut memo)
    }

    fn try_compose_from(
        &self,
        s: &str,
        offsets: &[usize],
        idx: usize,
        memo: &mut [u8; 129],
    ) -> bool {
        let n = offsets.len() - 1;
        if idx == n {
            return true;
        }
        if memo[idx] != 0 {
            return memo[idx] == 1;
        }
        let max_end = (idx + 24).min(n);
        for end in (idx + 1..=max_end).rev() {
            let sub = &s[offsets[idx]..offsets[end]];
            if self.word_set.contains(sub) && self.try_compose_from(s, offsets, end, memo) {
                memo[idx] = 1;
                return true;
            }
        }
        memo[idx] = 2;
        false
    }

    pub fn try_compose_prefix(&self, s: &str) -> bool {
        let mut offsets = [0usize; 129];
        let Some(n) = collect_char_offsets(s, &mut offsets) else {
            return false;
        };
        let mut memo = [0u8; 129];
        self.try_compose_prefix_from(s, &offsets[..=n], 0, &mut memo)
    }

    fn try_compose_prefix_from(
        &self,
        s: &str,
        offsets: &[usize],
        idx: usize,
        memo: &mut [u8; 129],
    ) -> bool {
        let n = offsets.len() - 1;
        if idx == n {
            return true;
        }
        if memo[idx] != 0 {
            return memo[idx] == 1;
        }
        let tail = &s[offsets[idx]..offsets[n]];
        if self.word_set.contains(tail) || self.has_prefix(tail) {
            memo[idx] = 1;
            return true;
        }
        let max_end = (idx + 24).min(n);
        for end in (idx + 1..max_end).rev() {
            let sub = &s[offsets[idx]..offsets[end]];
            if self.word_set.contains(sub)
                && self.try_compose_prefix_from(s, offsets, end, memo)
            {
                memo[idx] = 1;
                return true;
            }
        }
        memo[idx] = 2;
        false
    }

    pub fn evaluate(
        &mut self,
        dist_map: &HashMap<(char, char), CharDistanceKind>,
        cfg: &DistanceConfig,
    ) -> FilterEval {
        self.evaluate_mode(dist_map, cfg, false)
    }

    pub fn evaluate_mode(
        &mut self,
        dist_map: &HashMap<(char, char), CharDistanceKind>,
        cfg: &DistanceConfig,
        prefix_only: bool,
    ) -> FilterEval {
        let mut raw_chars = ['\0'; 128];
        let mut raw_len = 0usize;
        let mut first_raw = '\0';
        for c in self.buffer.chars() {
            if raw_len == 0 {
                first_raw = c;
            }
            if raw_len < 128 {
                raw_chars[raw_len] = c;
                raw_len += 1;
            }
        }

        let offset = if raw_len > 0 && matches!(first_raw, '(' | '[' | '{' | '"' | '\'' | '“' | '‘')
        {
            1
        } else {
            0
        };

        let mut full_norm = ['\0'; 128];
        let mut full_n = 0usize;
        for &c in &raw_chars[offset..raw_len] {
            for lc in c.to_lowercase() {
                if full_n < 128 {
                    full_norm[full_n] = lc;
                    full_n += 1;
                }
            }
        }

        if full_n == 0 {
            self.last_eval = FilterEval::default();
            return self.last_eval.clone();
        }

        let full_str: String = full_norm[..full_n].iter().collect();

        if self.word_set.contains(&full_str)
            || (self.language == Language::Thai && full_n >= 4 && self.try_compose(&full_str))
        {
            let eval = FilterEval {
                is_exact_match: true,
                is_close_match: true,
                is_close_prefix_match: true,
            };
            self.last_eval = eval.clone();
            return eval;
        }

        if prefix_only
            && (self.has_prefix(&full_str)
                || (self.language == Language::Thai && self.try_compose_prefix(&full_str)))
        {
            let eval = FilterEval {
                is_exact_match: false,
                is_close_match: false,
                is_close_prefix_match: true,
            };
            self.last_eval = eval.clone();
            return eval;
        }

        if full_n > 31 {
            let eval = FilterEval::default();
            self.last_eval = eval.clone();
            return eval;
        }

        let n = full_n;
        let input_slice = &full_norm[..n];

        let allowed_distance = (n as f64) * cfg.quota;
        let prune_limit = allowed_distance + 1e-9;
        let mut exact_distance = 100.0_f64;
        let mut best_distance = 100.0_f64;

        let mut dp0 = [0.0_f64; 32];
        for j in 0..=n {
            dp0[j] = (j as f64) * DELETE_DISTANCE;
        }

        let mut d1_idx = self.trie_nodes[0].first_child;
        while d1_idx != INVALID_NODE {
            let node1_ch = self.trie_nodes[d1_idx as usize].ch;
            let mut dp1 = [0.0_f64; 32];
            let min1 = compute_dp_step(input_slice, node1_ch, 1, &dp0, &mut dp1, dist_map, cfg);
            let pref1 = dp0[n].min(dp1[n]);

            if min1 <= prune_limit {
                self.eval_node_words(
                    d1_idx,
                    1,
                    input_slice,
                    &dp1,
                    pref1,
                    prune_limit,
                    prefix_only,
                    dist_map,
                    cfg,
                    &mut exact_distance,
                    &mut best_distance,
                );
                if prefix_only && best_distance < prune_limit {
                    break;
                }

                let mut d2_idx = self.trie_nodes[d1_idx as usize].first_child;
                while d2_idx != INVALID_NODE {
                    let node2_ch = self.trie_nodes[d2_idx as usize].ch;
                    let mut dp2 = [0.0_f64; 32];
                    let min2 =
                        compute_dp_step(input_slice, node2_ch, 2, &dp1, &mut dp2, dist_map, cfg);
                    let pref2 = pref1.min(dp2[n]);

                    if min2 <= prune_limit || pref1 <= prune_limit {
                        self.eval_node_words(
                            d2_idx,
                            2,
                            input_slice,
                            &dp2,
                            pref2,
                            prune_limit,
                            prefix_only,
                            dist_map,
                            cfg,
                            &mut exact_distance,
                            &mut best_distance,
                        );
                        if prefix_only && best_distance < prune_limit {
                            break;
                        }

                        if min2 <= prune_limit {
                            let mut d3_idx = self.trie_nodes[d2_idx as usize].first_child;
                            while d3_idx != INVALID_NODE {
                                let node3_ch = self.trie_nodes[d3_idx as usize].ch;
                                let mut dp3 = [0.0_f64; 32];
                                let min3 = compute_dp_step(
                                    input_slice,
                                    node3_ch,
                                    3,
                                    &dp2,
                                    &mut dp3,
                                    dist_map,
                                    cfg,
                                );
                                let pref3 = pref2.min(dp3[n]);

                                if min3 <= prune_limit || pref2 <= prune_limit {
                                    self.eval_node_words(
                                        d3_idx,
                                        3,
                                        input_slice,
                                        &dp3,
                                        pref3,
                                        prune_limit,
                                        prefix_only,
                                        dist_map,
                                        cfg,
                                        &mut exact_distance,
                                        &mut best_distance,
                                    );
                                    if prefix_only && best_distance < prune_limit {
                                        break;
                                    }
                                }
                                d3_idx = self.trie_nodes[d3_idx as usize].next_sibling;
                            }
                            if prefix_only && best_distance < prune_limit {
                                break;
                            }
                        }
                    }
                    d2_idx = self.trie_nodes[d2_idx as usize].next_sibling;
                }
                if prefix_only && best_distance < prune_limit {
                    break;
                }
            }
            d1_idx = self.trie_nodes[d1_idx as usize].next_sibling;
        }

        let eval = FilterEval {
            is_exact_match: exact_distance < 1e-6,
            is_close_match: exact_distance < allowed_distance + 1e-9,
            is_close_prefix_match: best_distance < allowed_distance + 1e-9,
        };
        self.last_eval = eval.clone();
        eval
    }

    #[inline]
    #[allow(clippy::too_many_arguments)]
    fn eval_node_words(
        &self,
        node_idx: u32,
        start_depth: usize,
        input: &[char],
        dp_start: &[f64; 32],
        best_pref_so_far: f64,
        prune_limit: f64,
        prefix_only: bool,
        dist_map: &HashMap<(char, char), CharDistanceKind>,
        cfg: &DistanceConfig,
        exact_distance: &mut f64,
        best_distance: &mut f64,
    ) {
        let n = input.len();
        let word_ids = &self.trie_nodes[node_idx as usize].word_ids;
        for &wid in word_ids {
            let w_chars = self.get_word_chars(wid);
            let m = w_chars.len();
            if m + DEFAULT_TOLERANCE < n {
                continue;
            }
            let max_m = m.min(n + DEFAULT_TOLERANCE);
            let mut prev = *dp_start;
            let mut curr = [0.0_f64; 32];
            let mut best_pref = best_pref_so_far;
            let mut pruned = false;

            for i in (start_depth + 1)..=max_m {
                let wc = w_chars[i - 1];
                let row_min = compute_dp_step(input, wc, i, &prev, &mut curr, dist_map, cfg);
                if curr[n] < best_pref {
                    best_pref = curr[n];
                }
                prev = curr;
                if row_min > prune_limit {
                    pruned = true;
                    break;
                }
            }

            if !pruned {
                let full_d = if m <= max_m {
                    prev[n]
                } else {
                    prev[n] + ((m - max_m) as f64) * ADD_DISTANCE
                };
                if full_d < *exact_distance {
                    *exact_distance = full_d;
                }
            }
            if best_pref < *best_distance {
                *best_distance = best_pref;
            }
            if prefix_only && *best_distance < prune_limit {
                return;
            }
        }
    }
}

fn collect_char_offsets(s: &str, offsets: &mut [usize; 129]) -> Option<usize> {
    let mut n = 0usize;
    for (byte_idx, _) in s.char_indices() {
        if n >= 128 {
            return None;
        }
        offsets[n] = byte_idx;
        n += 1;
    }
    if n == 0 {
        return None;
    }
    offsets[n] = s.len();
    Some(n)
}

#[inline]
fn compute_dp_step(
    input: &[char],
    wc: char,
    row_idx: usize,
    prev: &[f64; 32],
    curr: &mut [f64; 32],
    dist_map: &HashMap<(char, char), CharDistanceKind>,
    cfg: &DistanceConfig,
) -> f64 {
    let n = input.len();
    curr[0] = (row_idx as f64) * ADD_DISTANCE;
    let mut row_min = curr[0];
    for j in 1..=n {
        let ic = input[j - 1];
        let delete_dist = curr[j - 1] + DELETE_DISTANCE;
        let add_dist = prev[j] + ADD_DISTANCE;
        let change_cost = get_change_distance(ic, wc, dist_map, cfg);
        let change_dist = prev[j - 1] + change_cost;
        let v = delete_dist.min(add_dist).min(change_dist);
        curr[j] = v;
        if v < row_min {
            row_min = v;
        }
    }
    row_min
}

#[inline]
pub fn get_change_distance(
    a: char,
    b: char,
    dist_map: &HashMap<(char, char), CharDistanceKind>,
    cfg: &DistanceConfig,
) -> f64 {
    if a == b {
        return 0.0;
    }
    match dist_map.get(&(a, b)) {
        Some(CharDistanceKind::AdjacentColumn) => cfg.adjacent_column_distance,
        Some(CharDistanceKind::AdjacentRow) => cfg.adjacent_row_distance,
        Some(CharDistanceKind::Shift) => cfg.shift_distance,
        None => cfg.change_distance,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationKind {
    AutoCorrection,
    ManualCorrection,
    NewWordLearned,
}

impl NotificationKind {
    pub fn title(self, thai_ui: bool) -> &'static str {
        match (self, thai_ui) {
            (NotificationKind::AutoCorrection, false) => "KeySwitch — Auto Switched",
            (NotificationKind::ManualCorrection, false) => "KeySwitch — Manual Switch",
            (NotificationKind::NewWordLearned, false) => "KeySwitch — Learned Word",
            (NotificationKind::AutoCorrection, true) => "KeySwitch — เปลี่ยนภาษาอัตโนมัติ",
            (NotificationKind::ManualCorrection, true) => "KeySwitch — เปลี่ยนภาษาด้วยตนเอง",
            (NotificationKind::NewWordLearned, true) => "KeySwitch — เรียนรู้คำใหม่",
        }
    }
}

#[derive(Debug, Clone)]
pub struct NotificationBanner {
    pub kind: NotificationKind,
    pub message: String,
    pub opacity: f32,
    pub remaining_ms: f32,
}

#[derive(Debug, Clone)]
pub struct PendingOsReplacement {
    pub backspaces: usize,
    pub target_lang: Language,
    pub replacement_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeferredKey {
    Printable(char),
    Boundary(char),
    Backspace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciledReplacement {
    pub backspaces: usize,
    pub base_text: String,
    pub replay_keys: Vec<DeferredKey>,
    pub aborted: bool,
}

pub fn reconcile_replacement_queue(
    rep: &PendingOsReplacement,
    leaked_events: &[DeferredKey],
) -> ReconciledReplacement {
    let mut backspaces = rep.backspaces;
    let mut base_text = rep.replacement_text.clone();
    let mut replay_keys = Vec::with_capacity(leaked_events.len());
    let mut aborted = false;

    for &ev in leaked_events {
        match ev {
            DeferredKey::Printable(c) => {
                backspaces += 1;
                replay_keys.push(DeferredKey::Printable(c));
            }
            DeferredKey::Boundary(ch) if ch == ' ' || ch == '\t' => {
                backspaces += 1;
                replay_keys.push(DeferredKey::Boundary(ch));
            }
            DeferredKey::Boundary(_) => {
                aborted = true;
                break;
            }
            DeferredKey::Backspace => {
                if replay_keys.pop().is_some() {
                    backspaces = backspaces.saturating_sub(1);
                } else if backspaces > 0 {
                    backspaces -= 1;
                    let _ = base_text.pop();
                }
            }
        }
    }

    ReconciledReplacement {
        backspaces,
        base_text,
        replay_keys,
        aborted,
    }
}

fn strip_leading_open_bracket(s: &str) -> &str {
    if let Some(first) = s.chars().next() {
        if matches!(first, '(' | '[' | '{' | '"' | '\'' | '“' | '‘') {
            return &s[first.len_utf8()..];
        }
    }
    s
}

fn tail_is_ascii_alpha(s: &str, k: usize) -> bool {
    let mut count = 0usize;
    for c in s.chars().rev().take(k) {
        if !c.is_ascii_alphabetic() {
            return false;
        }
        count += 1;
    }
    count == k
}

pub struct RightLangEngine {
    pub mode: CorrectionMode,
    pub auto_start: bool,
    pub learn_new_word: bool,
    pub show_popup: bool,
    pub switch_buffer_shortcut: String,
    pub switch_select_shortcut: String,
    pub thai_ui: bool,
    pub dist_config: DistanceConfig,
    pub dist_map: HashMap<(char, char), CharDistanceKind>,
    pub eng_filter: WordDictFilter,
    pub thai_filter: WordDictFilter,
    pub active_process_name: String,
    pub active_language: Language,
    pub current_cell: ThaiCell,
    pub char_counts: Vec<i32>,
    pub thai_vowels: usize,
    pub thai_tone_marks: usize,
    pub learn_this: bool,
    pub can_undo: bool,
    pub just_auto_switched_midword: bool,
    pub last_uncorrected: String,
    pub last_corrected: String,
    pub last_undo_backspaces: usize,
    pub last_space_eng: String,
    pub last_space_thai: String,
    pub last_space_char_counts: Vec<i32>,
    pub last_space_lang: Language,
    pub notification: Option<NotificationBanner>,
    pub pending_os_replacement: Option<PendingOsReplacement>,
}

impl RightLangEngine {
    pub fn new() -> Self {
        let dist_map = build_distance_map();
        let (eng_base, thai_base, eng_user, thai_user) = load_dictionaries_with_fallback();
        let eng_filter = WordDictFilter::new(Language::English, eng_base, eng_user);
        let thai_filter = WordDictFilter::new(Language::Thai, thai_base, thai_user);

        let mut engine = Self {
            mode: CorrectionMode::Online,
            auto_start: true,
            learn_new_word: true,
            show_popup: true,
            switch_buffer_shortcut: "Shift + Backspace".to_string(),
            switch_select_shortcut: "Shift + Caps Lock".to_string(),
            thai_ui: true,
            dist_config: DistanceConfig::default(),
            dist_map,
            eng_filter,
            thai_filter,
            active_process_name: String::new(),
            active_language: Language::English,
            current_cell: ThaiCell::default(),
            char_counts: Vec::with_capacity(16),
            thai_vowels: 0,
            thai_tone_marks: 0,
            learn_this: false,
            can_undo: false,
            just_auto_switched_midword: false,
            last_uncorrected: String::new(),
            last_corrected: String::new(),
            last_undo_backspaces: 0,
            last_space_eng: String::new(),
            last_space_thai: String::new(),
            last_space_char_counts: Vec::new(),
            last_space_lang: Language::English,
            notification: None,
            pending_os_replacement: None,
        };

        engine.load_config_if_exists();
        engine
    }

    pub fn set_active_process(&mut self, proc_name: &str) {
        let lower = proc_name
            .trim()
            .trim_end_matches(".exe")
            .to_ascii_lowercase();
        if self.active_process_name != lower {
            self.active_process_name = lower;
        }
    }

    pub fn active_checking_rule(&self) -> SequenceCheckingRule {
        SequenceCheckingRule::None
    }

    pub fn effective_mode(&self) -> CorrectionMode {
        if matches!(
            self.active_process_name.as_str(),
            "cmd" | "powershell" | "pwsh" | "windowsterminal" | "wt"
        ) {
            return CorrectionMode::Disabled;
        }
        self.mode
    }

    pub fn clear_current_word(&mut self) {
        self.eng_filter.clear_buffer();
        self.thai_filter.clear_buffer();
        self.current_cell.clear();
        self.char_counts.clear();
        self.thai_vowels = 0;
        self.thai_tone_marks = 0;
        self.learn_this = false;
    }

    fn clear_last_space(&mut self) {
        self.last_space_eng.clear();
        self.last_space_thai.clear();
        self.last_space_char_counts.clear();
    }

    pub fn reset_all_buffers(&mut self) {
        self.clear_current_word();
        self.clear_last_space();
        self.can_undo = false;
        self.just_auto_switched_midword = false;
        self.last_uncorrected.clear();
        self.last_corrected.clear();
        self.last_undo_backspaces = 0;
    }

    pub fn clear_buffers(&mut self) {
        self.reset_all_buffers();
    }

    fn recompute_thai_metrics(&mut self) {
        let rule = self.active_checking_rule();
        self.thai_vowels = 0;
        self.thai_tone_marks = 0;
        self.current_cell.clear();
        for c in self.thai_filter.buffer.chars() {
            if is_thai_vowel_char(c) {
                self.thai_vowels += 1;
            } else if is_thai_tone_char(c) {
                self.thai_tone_marks += 1;
            }
            let _ = merge_current_cell(&mut self.current_cell, c, rule);
        }
    }

    pub fn handle_backspace(&mut self) {
        self.can_undo = false;
        if self.eng_filter.buffer.is_empty() && !self.last_space_eng.is_empty() {
            let _ = self.last_space_eng.pop();
            let _ = self.last_space_thai.pop();
            let _ = self.last_space_char_counts.pop();
            if self.last_space_eng.is_empty() {
                self.clear_last_space();
                return;
            }
            if !self.last_space_eng.ends_with(' ') {
                self.eng_filter.buffer = std::mem::take(&mut self.last_space_eng);
                self.thai_filter.buffer = std::mem::take(&mut self.last_space_thai);
                self.char_counts = std::mem::take(&mut self.last_space_char_counts);
                self.active_language = self.last_space_lang;
                self.recompute_thai_metrics();
                self.eng_filter.last_eval = FilterEval::default();
                self.thai_filter.last_eval = FilterEval::default();
            }
            return;
        }

        let _ = self.thai_filter.buffer.pop();
        let _ = self.eng_filter.buffer.pop();
        let _ = self.char_counts.pop();
        if self.eng_filter.buffer.is_empty() {
            self.clear_current_word();
        } else {
            self.recompute_thai_metrics();
            self.eng_filter.last_eval = FilterEval::default();
            self.thai_filter.last_eval = FilterEval::default();
        }
    }

    fn is_valid_thai_text(&self, s: &str) -> bool {
        if s.is_empty() || !has_valid_thai_sequence(s) {
            return false;
        }
        self.thai_filter.word_set.contains(s)
            || (s.chars().count() >= 4 && self.thai_filter.try_compose(s))
    }

    fn is_valid_english_token(&self, s: &str) -> bool {
        if s.is_empty() {
            return false;
        }
        let lower = s.to_ascii_lowercase();
        if lower.contains(":\x2F\x2F") || lower.contains('@') || lower.contains("www.") {
            return true;
        }
        let core = strip_surrounding_punct(&lower);
        !core.is_empty() && self.eng_filter.word_set.contains(core)
    }

    fn find_mixed_suffix_split(
        &self,
        min_suffix_len: usize,
        allow_alpha_fallback: bool,
    ) -> Option<usize> {
        let eng_str = &self.eng_filter.buffer;
        let thai_str = &self.thai_filter.buffer;
        let mut eng_off = [0usize; 129];
        let mut thai_off = [0usize; 129];
        let eng_len = collect_char_offsets(eng_str, &mut eng_off)?;
        let thai_len = collect_char_offsets(thai_str, &mut thai_off)?;
        if eng_len != thai_len {
            return None;
        }
        let mut core_len = eng_len;
        while core_len > 0 && &eng_str[eng_off[core_len - 1]..eng_off[core_len]] == " " {
            core_len -= 1;
        }
        if core_len < min_suffix_len + 2 {
            return None;
        }

        let full_eng = &eng_str[..eng_off[core_len]];
        let full_thai = &thai_str[..thai_off[core_len]];

        match self.active_language {
            Language::Thai => {
                if self.is_valid_thai_text(full_thai)
                    || self.is_valid_english_token(full_eng)
                    || (full_eng.chars().all(|c| c.is_ascii_alphabetic())
                        && self.eng_filter.has_prefix(full_eng))
                {
                    return None;
                }
            }
            Language::English => {
                if self.is_valid_english_token(full_eng)
                    || self.is_valid_thai_text(full_thai)
                {
                    return None;
                }
            }
        }

        let start_split = core_len.saturating_sub(min_suffix_len);
        for split in (2..=start_split).rev() {
            let suffix_len = core_len - split;
            if suffix_len < min_suffix_len {
                continue;
            }
            let prefix_eng = &eng_str[..eng_off[split]];
            let prefix_thai = &thai_str[..thai_off[split]];
            let suffix_eng = &eng_str[eng_off[split]..eng_off[core_len]];
            let suffix_thai = &thai_str[thai_off[split]..thai_off[core_len]];

            match self.active_language {
                Language::Thai => {
                    if split < 3 && self.eng_filter.has_prefix(prefix_eng) {
                        continue;
                    }
                    let prefix_ok = has_valid_thai_sequence(prefix_thai)
                        && (self.thai_filter.word_set.contains(prefix_thai)
                            || self.thai_filter.try_compose(prefix_thai));
                    if !prefix_ok {
                        continue;
                    }
                    if self.is_valid_thai_text(suffix_thai) {
                        continue;
                    }
                    if !suffix_eng.chars().all(|c| c.is_ascii_alphabetic()) {
                        continue;
                    }
                    let can_fallback = allow_alpha_fallback
                        && split >= 3
                        && (split >= 4 || !prefix_eng.chars().all(|c| c.is_ascii_alphabetic()))
                        && !self.thai_filter.try_compose(suffix_thai);
                    let suffix_en_ok = if suffix_len == 2 {
                        if !suffix_eng.bytes().any(|b| b.is_ascii_uppercase()) {
                            COMMON_SHORT_EN.contains(&suffix_eng) || can_fallback
                        } else {
                            let low = suffix_eng.to_ascii_lowercase();
                            COMMON_SHORT_EN.contains(&low.as_str()) || can_fallback
                        }
                    } else {
                        self.eng_filter.contains_lower(suffix_eng) || can_fallback
                    };
                    if suffix_en_ok {
                        return Some(split);
                    }
                }
                Language::English => {
                    let prefix_ok = prefix_eng.chars().all(|c| c.is_ascii_alphabetic())
                        && self.eng_filter.contains_lower(prefix_eng);
                    if !prefix_ok {
                        continue;
                    }
                    if self.eng_filter.contains_lower(suffix_eng) {
                        continue;
                    }
                    let suffix_th_ok = has_valid_thai_sequence(suffix_thai)
                        && (self.thai_filter.word_set.contains(suffix_thai)
                            || (suffix_len >= 4 && self.thai_filter.try_compose(suffix_thai)));
                    if suffix_th_ok {
                        return Some(split);
                    }
                }
            }
        }
        None
    }

    pub fn process_keystroke(&mut self, raw_char: char) -> Option<PendingOsReplacement> {
        self.pending_os_replacement = None;
        self.can_undo = false;
        let eff_mode = self.effective_mode();
        if eff_mode == CorrectionMode::Disabled {
            if !self.eng_filter.buffer.is_empty() || !self.last_space_eng.is_empty() {
                self.reset_all_buffers();
            }
            return None;
        }
        let rule = self.active_checking_rule();

        if raw_char == ' ' || raw_char == '\n' || raw_char == '\t' {
            if self.eng_filter.buffer.is_empty() {
                if raw_char == ' ' && !self.last_space_eng.is_empty() {
                    let trailing_spaces = self
                        .last_space_eng
                        .chars()
                        .rev()
                        .take_while(|&c| c == ' ')
                        .count();
                    if trailing_spaces < 8 {
                        self.last_space_eng.push(' ');
                        self.last_space_thai.push(' ');
                        self.last_space_char_counts.push(1);
                    }
                } else {
                    self.clear_last_space();
                }
                self.just_auto_switched_midword = false;
                return None;
            }

            let len = self.eng_filter.buffer.chars().count();

            if eff_mode == CorrectionMode::Online
                && raw_char == ' '
                && len >= 2
                && !self.just_auto_switched_midword
            {
                let eng_lower = self.eng_filter.buffer.to_lowercase();
                let thai_buf = self.thai_filter.buffer.clone();
                let eng_core = strip_surrounding_punct(&eng_lower);
                let thai_core = strip_surrounding_punct(&thai_buf);
                let is_url_or_code = eng_lower.contains(":\x2F\x2F")
                    || eng_lower.contains('@')
                    || eng_lower.contains("www.");

                if self.active_language == Language::Thai {
                    let thai_stripped = strip_leading_open_bracket(&thai_buf);
                    let thai_is_valid = if len == 2 {
                        (self.thai_filter.word_set.contains(&thai_buf)
                            || (!thai_core.is_empty()
                                && self.thai_filter.word_set.contains(thai_core)))
                            && thai_buf != "กน"
                    } else {
                        self.thai_filter.word_set.contains(&thai_buf)
                            || (!thai_stripped.is_empty()
                                && self.thai_filter.word_set.contains(thai_stripped))
                            || (!thai_core.is_empty()
                                && (self.thai_filter.word_set.contains(thai_core)
                                    || (thai_core.chars().count() >= 4
                                        && self.thai_filter.try_compose(thai_core))))
                            || (len >= 4 && self.thai_filter.try_compose(&thai_buf))
                            || self
                                .thai_filter
                                .evaluate_mode(&self.dist_map, &self.dist_config, false)
                                .is_close_match
                    };

                    if !thai_is_valid {
                        let eng_is_valid = if len == 2 {
                            let mut eb = self.eng_filter.buffer.bytes();
                            let weird_case = match (eb.next(), eb.next(), eb.next()) {
                                (Some(a), Some(b), None) => {
                                    a.is_ascii_lowercase() && b.is_ascii_uppercase()
                                }
                                _ => false,
                            };
                            !weird_case && COMMON_SHORT_EN.contains(&eng_lower.as_str())
                        } else {
                            let eng_stripped = strip_leading_open_bracket(&eng_lower);
                            self.eng_filter.word_set.contains(&eng_lower)
                                || (!eng_stripped.is_empty()
                                    && self.eng_filter.word_set.contains(eng_stripped))
                                || (len >= 4
                                    && self
                                        .eng_filter
                                        .evaluate_mode(&self.dist_map, &self.dist_config, false)
                                        .is_close_match)
                        };

                        if eng_is_valid {
                            self.eng_filter.buffer.push(' ');
                            self.thai_filter.buffer.push(' ');
                            self.char_counts.push(1);
                            self.switch_buffer_to(
                                Language::English,
                                NotificationKind::AutoCorrection,
                            );
                            let rep = self.pending_os_replacement.take();
                            self.clear_current_word();
                            self.clear_last_space();
                            self.just_auto_switched_midword = false;
                            return rep;
                        }
                    }
                } else if self.active_language == Language::English && !is_url_or_code {
                    let max_diacritics = (len * 2) / 3 + 1;
                    let thai_stripped = strip_leading_open_bracket(&thai_buf);
                    let thai_is_exact = (self.thai_vowels + self.thai_tone_marks) <= max_diacritics
                        && has_valid_thai_sequence(&thai_buf)
                        && (self.thai_filter.word_set.contains(&thai_buf)
                            || (len >= 4 && self.thai_filter.try_compose(&thai_buf))
                            || (!thai_stripped.is_empty()
                                && (self.thai_filter.word_set.contains(thai_stripped)
                                    || (thai_stripped.chars().count() >= 4
                                        && self.thai_filter.try_compose(thai_stripped)))));

                    if thai_is_exact {
                        let eng_is_valid = self.eng_filter.word_set.contains(&eng_lower)
                            || (!eng_core.is_empty() && self.eng_filter.word_set.contains(eng_core))
                            || (len >= 4
                                && self
                                    .eng_filter
                                    .evaluate_mode(&self.dist_map, &self.dist_config, false)
                                    .is_close_match);

                        if !eng_is_valid {
                            self.eng_filter.buffer.push(' ');
                            self.thai_filter.buffer.push(' ');
                            self.char_counts.push(1);
                            self.switch_buffer_to(Language::Thai, NotificationKind::AutoCorrection);
                            let rep = self.pending_os_replacement.take();
                            self.clear_current_word();
                            self.clear_last_space();
                            self.just_auto_switched_midword = false;
                            return rep;
                        }
                    }
                }

                if len >= 4 && !(self.active_language == Language::English && is_url_or_code) {
                    if let Some(split_idx) = self.find_mixed_suffix_split(2, false) {
                        let target = self.active_language.opposite();
                        self.eng_filter.buffer.push(' ');
                        self.thai_filter.buffer.push(' ');
                        self.char_counts.push(1);
                        self.switch_buffer_suffix_to(
                            split_idx,
                            target,
                            NotificationKind::AutoCorrection,
                        );
                        let rep = self.pending_os_replacement.take();
                        self.clear_current_word();
                        self.clear_last_space();
                        self.just_auto_switched_midword = false;
                        return rep;
                    }
                }
            }

            if eff_mode == CorrectionMode::Online && self.learn_new_word && self.learn_this {
                if !self.just_auto_switched_midword
                    && !self
                        .eng_filter
                        .evaluate_mode(&self.dist_map, &self.dist_config, false)
                        .is_close_match
                    && !self.thai_filter.try_compose(&self.thai_filter.buffer)
                {
                    self.learn_current_word();
                }
            }

            if raw_char == ' ' && !self.eng_filter.buffer.is_empty() {
                self.last_space_eng = format!("{} ", self.eng_filter.buffer);
                self.last_space_thai = format!("{} ", self.thai_filter.buffer);
                self.last_space_char_counts = self.char_counts.clone();
                self.last_space_char_counts.push(1);
                self.last_space_lang = self.active_language;
            } else {
                self.clear_last_space();
            }

            self.clear_current_word();
            self.just_auto_switched_midword = false;
            return None;
        }

        self.clear_last_space();

        if self.eng_filter.buffer.chars().count() >= 96 {
            let drop_chars = {
                let th_chars: Vec<char> = self.thai_filter.buffer.chars().collect();
                let mut d = 32usize;
                while d < 48 && d < th_chars.len() && is_thai_combining_mark(th_chars[d] as u32) {
                    d += 1;
                }
                d.min(th_chars.len())
            };
            self.eng_filter.buffer = self.eng_filter.buffer.chars().skip(drop_chars).collect();
            self.thai_filter.buffer = self.thai_filter.buffer.chars().skip(drop_chars).collect();
            if drop_chars <= self.char_counts.len() {
                self.char_counts.drain(0..drop_chars);
            } else {
                self.char_counts.clear();
            }
            self.recompute_thai_metrics();
        }

        let (eng_c, thai_c) = map_keystroke_to_both(raw_char, self.active_language);
        self.eng_filter.buffer.push(eng_c);
        self.thai_filter.buffer.push(thai_c);

        if is_thai_vowel_char(thai_c) {
            self.thai_vowels += 1;
        } else if is_thai_tone_char(thai_c) {
            self.thai_tone_marks += 1;
        }

        let cell_delta = merge_current_cell(&mut self.current_cell, thai_c, rule);
        self.char_counts.push(cell_delta);

        if eff_mode != CorrectionMode::Online {
            return None;
        }

        let len = self.eng_filter.buffer.chars().count();
        if len < DEFAULT_MIN_MIDWORD_LEN {
            return None;
        }

        let eng_lower = self.eng_filter.buffer.to_lowercase();
        if self.active_language == Language::English
            && (eng_lower.contains(":\x2F\x2F")
                || eng_lower.contains('@')
                || eng_lower.contains("www."))
        {
            return None;
        }

        let max_t = self.dist_config.max_threshold;
        if self.just_auto_switched_midword {
            if len > max_t && self.active_language == Language::English {
                self.learn_this = true;
            }
            return None;
        }

        if self.active_language == Language::Thai {
            let is_en_alpha = self.eng_filter.buffer.chars().all(|c| c.is_ascii_alphabetic());
            let en_match = if !is_en_alpha {
                false
            } else if len != max_t {
                self.eng_filter.contains_lower(&self.eng_filter.buffer)
            } else {
                self.eng_filter
                    .evaluate_mode(&self.dist_map, &self.dist_config, true)
                    .is_close_prefix_match
            };
            let check_th = en_match
                || (len >= 6
                    && tail_is_ascii_alpha(&self.eng_filter.buffer, DEFAULT_MIN_MIDWORD_LEN));
            if check_th {
                let th_close_pref = self
                    .thai_filter
                    .evaluate_mode(&self.dist_map, &self.dist_config, true)
                    .is_close_prefix_match;
                if !th_close_pref && en_match {
                    self.switch_buffer_to(Language::English, NotificationKind::AutoCorrection);
                    let rep = self.pending_os_replacement.take();
                    self.clear_current_word();
                    self.just_auto_switched_midword = true;
                    return rep;
                }
                if !th_close_pref && len >= 6 {
                    if let Some(split_idx) =
                        self.find_mixed_suffix_split(DEFAULT_MIN_MIDWORD_LEN, false)
                    {
                        self.switch_buffer_suffix_to(
                            split_idx,
                            Language::English,
                            NotificationKind::AutoCorrection,
                        );
                        let rep = self.pending_os_replacement.take();
                        self.clear_current_word();
                        self.just_auto_switched_midword = true;
                        return rep;
                    }
                }
            }
        } else {
            let th_valid_seq = has_valid_thai_sequence(&self.thai_filter.buffer);
            let th_match = if !th_valid_seq {
                false
            } else if len != max_t {
                self.thai_filter.word_set.contains(&self.thai_filter.buffer)
                    || self.thai_filter.try_compose(&self.thai_filter.buffer)
            } else {
                self.thai_filter
                    .evaluate_mode(&self.dist_map, &self.dist_config, true)
                    .is_close_prefix_match
            };
            let max_diacritics = (len * 2) / 3 + 1;
            let check_en =
                (th_match && (self.thai_vowels + self.thai_tone_marks) <= max_diacritics)
                    || len >= 6;
            if check_en {
                let en_core = strip_surrounding_punct(&eng_lower);
                let en_close_pref = (!en_core.is_empty()
                    && self.eng_filter.word_set.contains(en_core))
                    || self
                        .eng_filter
                        .evaluate_mode(&self.dist_map, &self.dist_config, true)
                        .is_close_prefix_match;
                if !en_close_pref
                    && th_match
                    && (self.thai_vowels + self.thai_tone_marks) <= max_diacritics
                {
                    self.switch_buffer_to(Language::Thai, NotificationKind::AutoCorrection);
                    let rep = self.pending_os_replacement.take();
                    self.clear_current_word();
                    self.just_auto_switched_midword = true;
                    return rep;
                }
                if !en_close_pref && len >= 6 {
                    if let Some(split_idx) =
                        self.find_mixed_suffix_split(DEFAULT_MIN_MIDWORD_LEN, false)
                    {
                        self.switch_buffer_suffix_to(
                            split_idx,
                            Language::Thai,
                            NotificationKind::AutoCorrection,
                        );
                        let rep = self.pending_os_replacement.take();
                        self.clear_current_word();
                        self.just_auto_switched_midword = true;
                        return rep;
                    }
                }
            }
        }
        if len > max_t && self.active_language == Language::English {
            self.learn_this = true;
        }

        None
    }

    pub fn switch_buffer_to(&mut self, target_lang: Language, kind: NotificationKind) {
        let from_str = match target_lang {
            Language::English => self.thai_filter.buffer.clone(),
            Language::Thai => self.eng_filter.buffer.clone(),
        };
        let to_str = match target_lang {
            Language::English => self.eng_filter.buffer.clone(),
            Language::Thai => self.thai_filter.buffer.clone(),
        };
        if from_str.is_empty() {
            return;
        }

        let backspace_count: usize = if target_lang == Language::English {
            let sum: i32 = self.char_counts.iter().sum();
            sum.max(1) as usize
        } else {
            self.eng_filter.buffer.chars().count()
        };

        self.last_uncorrected = from_str.clone();
        self.last_corrected = to_str.clone();
        self.last_undo_backspaces = to_str.chars().count().max(1);
        self.active_language = target_lang;
        self.can_undo = true;

        self.pending_os_replacement = Some(PendingOsReplacement {
            backspaces: backspace_count,
            target_lang,
            replacement_text: to_str.clone(),
        });

        if self.show_popup {
            let msg = format!("{} » {}", from_str.trim_end(), to_str.trim_end());
            self.notification = Some(NotificationBanner {
                kind,
                message: msg,
                opacity: NOTIFIER_DEFAULT_OPACITY,
                remaining_ms: NOTIFIER_DISPLAY_INTERVAL_MS,
            });
        }
    }

    pub fn switch_buffer_suffix_to(
        &mut self,
        split_idx: usize,
        target_lang: Language,
        kind: NotificationKind,
    ) {
        let eng_suffix: String = self.eng_filter.buffer.chars().skip(split_idx).collect();
        let thai_suffix: String = self.thai_filter.buffer.chars().skip(split_idx).collect();
        let (from_str, to_str) = match target_lang {
            Language::English => (thai_suffix, eng_suffix),
            Language::Thai => (eng_suffix, thai_suffix),
        };
        if from_str.is_empty() {
            return;
        }

        let backspace_count: usize = if target_lang == Language::English {
            let sum: i32 = self.char_counts.iter().skip(split_idx).sum();
            sum.max(1) as usize
        } else {
            from_str.chars().count().max(1)
        };

        self.last_uncorrected = from_str.clone();
        self.last_corrected = to_str.clone();
        self.last_undo_backspaces = to_str.chars().count().max(1);
        self.active_language = target_lang;
        self.can_undo = true;

        self.pending_os_replacement = Some(PendingOsReplacement {
            backspaces: backspace_count,
            target_lang,
            replacement_text: to_str.clone(),
        });

        if self.show_popup {
            let msg = format!("{} » {}", from_str.trim_end(), to_str.trim_end());
            self.notification = Some(NotificationBanner {
                kind,
                message: msg,
                opacity: NOTIFIER_DEFAULT_OPACITY,
                remaining_ms: NOTIFIER_DISPLAY_INTERVAL_MS,
            });
        }
    }

    pub fn trigger_shift_backspace(&mut self) -> Option<PendingOsReplacement> {
        self.pending_os_replacement = None;
        if self.effective_mode() == CorrectionMode::Disabled {
            return None;
        }
        if self.can_undo && !self.last_uncorrected.is_empty() {
            let prev = self.last_uncorrected.clone();
            let curr = self.last_corrected.clone();
            let target = self.active_language.opposite();
            let backspaces = self.last_undo_backspaces.max(1);
            self.active_language = target;
            self.can_undo = true;
            self.last_uncorrected = curr.clone();
            self.last_corrected = prev.clone();
            self.last_undo_backspaces = prev.chars().count().max(1);

            if self.show_popup {
                self.notification = Some(NotificationBanner {
                    kind: NotificationKind::ManualCorrection,
                    message: format!("{} » {}", curr.trim_end(), prev.trim_end()),
                    opacity: NOTIFIER_DEFAULT_OPACITY,
                    remaining_ms: NOTIFIER_DISPLAY_INTERVAL_MS,
                });
            }
            return Some(PendingOsReplacement {
                backspaces,
                target_lang: target,
                replacement_text: prev,
            });
        }

        if self.eng_filter.buffer.is_empty() && !self.last_space_eng.is_empty() {
            self.eng_filter.buffer = std::mem::take(&mut self.last_space_eng);
            self.thai_filter.buffer = std::mem::take(&mut self.last_space_thai);
            self.char_counts = std::mem::take(&mut self.last_space_char_counts);
            self.active_language = self.last_space_lang;
        }

        if !self.eng_filter.buffer.is_empty() {
            let target = self.active_language.opposite();
            if let Some(split_idx) = self.find_mixed_suffix_split(2, true) {
                self.switch_buffer_suffix_to(split_idx, target, NotificationKind::ManualCorrection);
            } else {
                self.switch_buffer_to(target, NotificationKind::ManualCorrection);
                if self.learn_new_word && !self.eng_filter.buffer.ends_with(' ') {
                    self.learn_current_word();
                }
            }
            let rep = self.pending_os_replacement.take();
            self.clear_current_word();
            self.clear_last_space();
            return rep;
        }
        None
    }

    pub fn toggle_online_offline_mode(&mut self) {
        self.mode = match self.mode {
            CorrectionMode::Online => CorrectionMode::Offline,
            _ => CorrectionMode::Online,
        };
        let mode_str = self.mode.label(self.thai_ui);
        if self.show_popup {
            self.notification = Some(NotificationBanner {
                kind: NotificationKind::ManualCorrection,
                message: format!("โหมด: {mode_str}"),
                opacity: NOTIFIER_DEFAULT_OPACITY,
                remaining_ms: 1800.0,
            });
        }
    }

    fn learn_current_word(&mut self) {
        match self.active_language {
            Language::English => {
                let w = self
                    .eng_filter
                    .buffer
                    .trim_end_matches(ENG_TRIM_END)
                    .to_lowercase();
                if is_sane_user_word_en(&w) && self.eng_filter.add_user_word(&w) {
                    let _ = self.save_all_user_settings();
                }
            }
            Language::Thai => {
                let trimmed = self.thai_filter.buffer.trim_end_matches(THAI_TRIM_END);
                let chars: Vec<char> = trimmed.chars().collect();
                let take_n = chars.len().min(self.dist_config.max_threshold);
                let w: String = chars[..take_n].iter().collect();
                if take_n >= DEFAULT_MIN_MIDWORD_LEN
                    && is_sane_user_word_th(&w)
                    && self.thai_filter.add_user_word(&w)
                {
                    let _ = self.save_all_user_settings();
                }
            }
        }
    }

    pub fn load_config_if_exists(&mut self) {
        let base = active_storage_dir();
        let cfg_path = base.join("data").join("config.txt");
        let Ok(raw) = fs::read_to_string(&cfg_path) else {
            return;
        };
        let mut section = "";
        let mut seen_online = false;
        for line in raw.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            if t.starts_with('[') && t.ends_with(']') {
                section = &t[1..t.len() - 1];
                if section == "online" && !seen_online {
                    seen_online = true;
                    self.learn_new_word = false;
                    self.show_popup = false;
                }
                continue;
            }
            match section {
                "online" => match t {
                    "learn" => self.learn_new_word = true,
                    "popup" => self.show_popup = true,
                    _ => {}
                },
                "offline" => {
                    if let Some(rest) = t.strip_prefix("switch-buffer ") {
                        let r = rest.trim();
                        self.switch_buffer_shortcut = if r.eq_ignore_ascii_case("Shift + Back") {
                            "Shift + Backspace".to_string()
                        } else {
                            r.to_string()
                        };
                    } else if let Some(rest) = t.strip_prefix("switch-select ") {
                        let r = rest.trim();
                        self.switch_select_shortcut = if r.eq_ignore_ascii_case("Shift + Capital") {
                            "Shift + Caps Lock".to_string()
                        } else {
                            r.to_string()
                        };
                    }
                }
                "misc" => {
                    if let Some(rest) = t.strip_prefix("ui-lang ") {
                        self.thai_ui = !rest.trim().eq_ignore_ascii_case("en");
                    }
                }
                _ => {}
            }
        }
    }

    pub fn save_all_user_settings(&self) -> Result<(), String> {
        let base = active_storage_dir();
        let data_dir = base.join("data");
        let dict_dir = base.join("dict");
        fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
        fs::create_dir_all(&dict_dir).map_err(|e| e.to_string())?;

        let mut cfg_lines = vec!["[online]".to_string()];
        if self.learn_new_word {
            cfg_lines.push("learn".to_string());
        }
        if self.show_popup {
            cfg_lines.push("popup".to_string());
        }
        cfg_lines.push("".to_string());
        cfg_lines.push("[offline]".to_string());
        cfg_lines.push(format!("switch-buffer {}", self.switch_buffer_shortcut));
        cfg_lines.push(format!("switch-select {}", self.switch_select_shortcut));
        cfg_lines.push("".to_string());
        cfg_lines.push("[misc]".to_string());
        cfg_lines.push(format!(
            "ui-lang {}",
            if self.thai_ui { "th" } else { "en" }
        ));
        fs::write(data_dir.join("config.txt"), cfg_lines.join("\r\n"))
            .map_err(|e| e.to_string())?;

        let user_en: Vec<String> = self.eng_filter.user_words.iter().cloned().collect();
        let user_th: Vec<String> = self.thai_filter.user_words.iter().cloned().collect();
        fs::write(dict_dir.join("user.en.txt"), user_en.join("\r\n"))
            .map_err(|e| e.to_string())?;
        fs::write(dict_dir.join("user.th.txt"), user_th.join("\r\n"))
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

pub fn active_storage_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            if parent.join("data").exists() || parent.join("dict").exists() {
                return parent.to_path_buf();
            }
        }
    }
    let local_app_data =
        std::env::var("LOCALAPPDATA").unwrap_or_else(|_| r"C:\Users\Public".to_string());
    PathBuf::from(local_app_data)
        .join("Programs")
        .join("KeySwitch")
}

pub fn extract_embedded_data_to_dir(install_dir: &Path) -> Result<(), String> {
    let data_dir = install_dir.join("data");
    let dict_dir = install_dir.join("dict");
    fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
    fs::create_dir_all(&dict_dir).map_err(|e| e.to_string())?;

    let files: &[(&Path, &str, &str)] = &[
        (&data_dir, "adjacent-column.txt", EMBEDDED_ADJACENT_COL),
        (&data_dir, "shift.txt", EMBEDDED_SHIFT),
        (&dict_dir, "eng.txt", EMBEDDED_ENG_DICT),
        (&dict_dir, "thai.txt", EMBEDDED_THAI_DICT),
        (&dict_dir, "user.en.txt", EMBEDDED_USER_EN),
        (&dict_dir, "user.th.txt", EMBEDDED_USER_TH),
    ];

    for &(dir, name, content) in files {
        let p = dir.join(name);
        if !p.exists() {
            fs::write(&p, content).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub fn build_distance_map() -> HashMap<(char, char), CharDistanceKind> {
    let mut map = HashMap::with_capacity(256);
    let base = active_storage_dir();
    let col_raw = fs::read_to_string(base.join("data").join("adjacent-column.txt"))
        .unwrap_or_else(|_| EMBEDDED_ADJACENT_COL.to_string());
    let shift_raw = fs::read_to_string(base.join("data").join("shift.txt"))
        .unwrap_or_else(|_| EMBEDDED_SHIFT.to_string());

    for line in col_raw.lines() {
        let chars: Vec<char> = line.trim().chars().collect();
        if chars.len() >= 2 {
            map.insert((chars[0], chars[1]), CharDistanceKind::AdjacentColumn);
            map.insert((chars[1], chars[0]), CharDistanceKind::AdjacentColumn);
        }
    }
    for line in shift_raw.lines() {
        let chars: Vec<char> = line.trim().chars().collect();
        if chars.len() >= 2 {
            map.insert((chars[0], chars[1]), CharDistanceKind::Shift);
            map.insert((chars[1], chars[0]), CharDistanceKind::Shift);
        }
    }
    map
}

fn parse_lines(raw: &str) -> Vec<String> {
    raw.lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn load_lines_or_embedded(rel_path: &str, embedded: &str) -> Vec<String> {
    let base = active_storage_dir();
    let full = base.join(rel_path);
    if let Ok(content) = fs::read_to_string(full) {
        let lines = parse_lines(&content);
        if !lines.is_empty() {
            return lines;
        }
    }
    parse_lines(embedded)
}

fn load_dictionaries_with_fallback() -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
    let eng_base = load_lines_or_embedded("dict/eng.txt", EMBEDDED_ENG_DICT);
    let thai_base = load_lines_or_embedded("dict/thai.txt", EMBEDDED_THAI_DICT);
    let eng_user = load_lines_or_embedded("dict/user.en.txt", EMBEDDED_USER_EN);
    let thai_user = load_lines_or_embedded("dict/user.th.txt", EMBEDDED_USER_TH);
    (eng_base, thai_base, eng_user, thai_user)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_engine() -> RightLangEngine {
        let mut eng = RightLangEngine::new();
        eng.learn_new_word = false;
        eng.mode = CorrectionMode::Online;
        eng
    }

    #[test]
    fn test_do_space_preserves_trailing_space() {
        let mut eng = make_test_engine();
        eng.active_language = Language::Thai;
        assert!(eng.process_keystroke('d').is_none());
        assert!(eng.process_keystroke('o').is_none());
        let rep = eng.process_keystroke(' ').expect("Expected switch on 'กน '");
        assert_eq!(rep.target_lang, Language::English);
        assert_eq!(rep.replacement_text, "do ");
        assert_eq!(rep.backspaces, 3);
    }

    #[test]
    fn test_sawasdee_midword_switch() {
        let mut eng = make_test_engine();
        eng.active_language = Language::English;
        let mut triggered = None;
        for c in "l;ylfu".chars() {
            if let Some(r) = eng.process_keystroke(c) {
                triggered = Some(r);
            }
        }
        let rep = triggered.expect("Expected mid-word switch on 'l;ylfu'");
        assert_eq!(rep.target_lang, Language::Thai);
        assert_eq!(rep.replacement_text, "สวัสดี");
    }

    #[test]
    fn test_no_false_switch_on_normal_thai_or_english() {
        let mut eng = make_test_engine();
        eng.active_language = Language::Thai;
        for c in "fo9iu ".chars() {
            assert!(eng.process_keystroke(c).is_none());
        }
        eng.active_language = Language::English;
        for c in "hello world ".chars() {
            assert!(eng.process_keystroke(c).is_none());
        }
    }

    #[test]
    fn test_long_compound_thai_word_auto_switch() {
        let mut eng = make_test_engine();
        eng.active_language = Language::English;
        let mut triggered = None;
        for c in "c]tg8]upiN".chars() {
            if let Some(r) = eng.process_keystroke(c) {
                triggered = Some(r);
                break;
            }
        }
        let rep = triggered.expect("Expected auto-switch on 'c]tg8]upiN'");
        assert_eq!(rep.target_lang, Language::Thai);
        assert_eq!(rep.replacement_text, "และเคลีย");
        assert_eq!(rep.backspaces, 8);
    }

    #[test]
    fn test_thai_word_with_equals_and_caret_not_truncated() {
        let mut eng = make_test_engine();
        eng.mode = CorrectionMode::Offline;
        eng.active_language = Language::English;
        for c in "=jv".chars() {
            assert!(eng.process_keystroke(c).is_none());
        }
        let rep = eng
            .trigger_shift_backspace()
            .expect("Expected Shift+Backspace to convert whole '=jv'");
        assert_eq!(rep.replacement_text, "ช่อ");
        assert_eq!(rep.backspaces, 3);

        eng.reset_all_buffers();
        eng.active_language = Language::English;
        for c in "r^f".chars() {
            assert!(eng.process_keystroke(c).is_none());
        }
        let rep2 = eng
            .trigger_shift_backspace()
            .expect("Expected Shift+Backspace to convert whole 'r^f'");
        assert_eq!(rep2.replacement_text, "พูด");
        assert_eq!(rep2.backspaces, 3);
    }

    #[test]
    fn test_shift_backspace_after_space_converts_whole_word() {
        let mut eng = make_test_engine();
        eng.mode = CorrectionMode::Offline;
        eng.active_language = Language::English;
        for c in "c]tg8]upiN ".chars() {
            assert!(eng.process_keystroke(c).is_none());
        }
        let rep = eng
            .trigger_shift_backspace()
            .expect("Expected Shift+Backspace after Space to convert whole word");
        assert_eq!(rep.target_lang, Language::Thai);
        assert_eq!(rep.replacement_text, "และเคลียร์ ");
        assert_eq!(rep.backspaces, 11);
    }

    #[test]
    fn test_shift_backspace_preserves_multiple_trailing_spaces() {
        let mut eng = make_test_engine();
        eng.mode = CorrectionMode::Offline;
        eng.active_language = Language::English;
        for c in "c]tg8]upiN   ".chars() {
            assert!(eng.process_keystroke(c).is_none());
        }
        let rep = eng
            .trigger_shift_backspace()
            .expect("Expected Shift+Backspace to keep all trailing spaces");
        assert_eq!(rep.target_lang, Language::Thai);
        assert_eq!(rep.replacement_text, "และเคลียร์   ");
        assert_eq!(rep.backspaces, 13);
    }

    #[test]
    fn test_mixed_thai_then_english_attached_auto_switch() {
        let mut eng = make_test_engine();
        eng.active_language = Language::Thai;
        let mut rep_world = None;
        for c in "l;ylfuworld".chars() {
            if let Some(r) = eng.process_keystroke(c) {
                rep_world = Some(r);
                break;
            }
        }
        let rw = rep_world.expect("Expected auto-switch on attached 'world'");
        assert_eq!(rw.target_lang, Language::English);
        assert_eq!(rw.replacement_text, "world");
        assert_eq!(rw.backspaces, 5);

        let mut eng2 = make_test_engine();
        eng2.active_language = Language::Thai;
        let mut rep_hello = None;
        for c in "สวัสดี้ำสสน".chars() {
            if let Some(r) = eng2.process_keystroke(c) {
                rep_hello = Some(r);
                break;
            }
        }
        let rh = rep_hello.expect("Expected auto-switch on 'สวัสดี' + '้ำสส'");
        assert_eq!(rh.target_lang, Language::English);
        assert_eq!(rh.replacement_text, "hell");
        assert_eq!(rh.backspaces, 4);
        assert!(eng2.process_keystroke('o').is_none());
    }

    #[test]
    fn test_mixed_english_then_thai_attached_auto_switch() {
        let mut eng = make_test_engine();
        eng.active_language = Language::English;
        let mut triggered = None;
        for c in "testl;ylfu".chars() {
            if let Some(r) = eng.process_keystroke(c) {
                triggered = Some(r);
                break;
            }
        }
        let rep = triggered.expect("Expected auto-switch on 'testl;ylfu'");
        assert_eq!(rep.target_lang, Language::Thai);
        assert_eq!(rep.replacement_text, "สวัสดี");
        assert_eq!(rep.backspaces, 6);
    }

    #[test]
    fn test_mixed_thai_then_english_shift_backspace_with_space() {
        let mut eng = make_test_engine();
        eng.mode = CorrectionMode::Offline;
        eng.active_language = Language::Thai;
        for c in "สวัสดี้ำสสน ".chars() {
            assert!(eng.process_keystroke(c).is_none());
        }
        let rep = eng
            .trigger_shift_backspace()
            .expect("Expected Shift+Backspace to replace only '้ำสสน '");
        assert_eq!(rep.target_lang, Language::English);
        assert_eq!(rep.replacement_text, "hello ");
        assert_eq!(rep.backspaces, 6);
    }

    #[test]
    fn test_mixed_english_then_thai_shift_backspace_with_space() {
        let mut eng = make_test_engine();
        eng.mode = CorrectionMode::Offline;
        eng.active_language = Language::English;
        for c in "testl;ylfu ".chars() {
            assert!(eng.process_keystroke(c).is_none());
        }
        let rep = eng
            .trigger_shift_backspace()
            .expect("Expected Shift+Backspace to replace only 'l;ylfu '");
        assert_eq!(rep.target_lang, Language::Thai);
        assert_eq!(rep.replacement_text, "สวัสดี ");
        assert_eq!(rep.backspaces, 7);
    }

    #[test]
    fn test_mixed_manual_os_switch_segment_isolation() {
        let mut eng = make_test_engine();
        eng.mode = CorrectionMode::Offline;
        eng.active_language = Language::Thai;
        for c in "สวัสดี".chars() {
            assert!(eng.process_keystroke(c).is_none());
        }
        eng.clear_current_word();
        eng.active_language = Language::English;
        for c in "hello".chars() {
            assert!(eng.process_keystroke(c).is_none());
        }
        let rep = eng
            .trigger_shift_backspace()
            .expect("Expected Shift+Backspace to convert only isolated EN segment");
        assert_eq!(rep.target_lang, Language::Thai);
        assert_eq!(rep.replacement_text, "้ำสสน");
        assert_eq!(rep.backspaces, 5);
    }

    #[test]
    fn test_long_thai_sentence_no_false_switch_to_english() {
        let mut eng = make_test_engine();
        eng.active_language = Language::Thai;
        for c in "ทำได้ให้เราไปดี".chars() {
            assert!(eng.process_keystroke(c).is_none());
        }
        eng.reset_all_buffers();
        eng.active_language = Language::Thai;
        let long_thai = "ฉันจะไปทำงานที่บ้านกับเพื่อนทุกคนเลยวันนี้ตอนเย็นและจะกลับมาทานข้าวด้วยกันนะ";
        assert!(long_thai.chars().count() >= 60);
        for c in long_thai.chars() {
            assert!(
                eng.process_keystroke(c).is_none(),
                "False switch during long Thai sentence"
            );
        }
        assert!(eng.process_keystroke(' ').is_none());
    }

    #[test]
    fn test_short_2_letter_thai_words_in_english_mode() {
        for (en_keys, expected_th) in [
            (",k ", "มา "),
            ("wx ", "ไป "),
            ("fu ", "ดี "),
            ("ot ", "นะ "),
        ] {
            let mut eng = make_test_engine();
            eng.active_language = Language::English;
            let mut rep = None;
            for c in en_keys.chars() {
                rep = eng.process_keystroke(c);
            }
            let r = rep.unwrap_or_else(|| panic!("Expected switch for '{en_keys}'"));
            assert_eq!(r.target_lang, Language::Thai);
            assert_eq!(r.replacement_text, expected_th);
            assert_eq!(r.backspaces, 3);
        }

        let mut eng_me = make_test_engine();
        eng_me.active_language = Language::English;
        for c in "me ".chars() {
            assert!(eng_me.process_keystroke(c).is_none());
        }
    }

    #[test]
    fn test_punctuation_and_urls_no_false_switch() {
        let mut eng = make_test_engine();
        for token in [
            "hello, ",
            "yes! ",
            "(test) ",
            "user@gmail.com ",
            "https:\x2F\x2Fgoogle.com ",
        ] {
            eng.reset_all_buffers();
            eng.active_language = Language::English;
            for c in token.chars() {
                assert!(
                    eng.process_keystroke(c).is_none(),
                    "False switch on '{token}'"
                );
            }
        }
    }

    #[test]
    fn test_backspace_into_word_after_space_and_shift_backspace() {
        let mut eng = make_test_engine();
        eng.mode = CorrectionMode::Offline;
        eng.active_language = Language::English;
        for c in "abc ".chars() {
            assert!(eng.process_keystroke(c).is_none());
        }
        eng.handle_backspace();
        let rep = eng
            .trigger_shift_backspace()
            .expect("Expected Shift+Backspace after backspacing space");
        assert_eq!(rep.target_lang, Language::Thai);
        assert_eq!(rep.replacement_text, "ฟิแ");
        assert_eq!(rep.backspaces, 3);
    }

    #[test]
    fn test_reset_all_buffers_prevents_stale_undo_or_last_space() {
        let mut eng = make_test_engine();
        eng.active_language = Language::English;
        for c in "l;ylfu".chars() {
            let _ = eng.process_keystroke(c);
        }
        assert!(eng.can_undo);
        eng.reset_all_buffers();
        assert!(eng.trigger_shift_backspace().is_none());

        eng.mode = CorrectionMode::Offline;
        eng.active_language = Language::English;
        for c in "c]tg8]upiN ".chars() {
            let _ = eng.process_keystroke(c);
        }
        assert!(!eng.last_space_eng.is_empty());
        eng.reset_all_buffers();
        assert!(eng.trigger_shift_backspace().is_none());
    }

    #[test]
    fn test_shift_backspace_undo_toggle_back_and_forth() {
        let mut eng = make_test_engine();
        eng.mode = CorrectionMode::Offline;
        eng.active_language = Language::English;
        for c in "test".chars() {
            assert!(eng.process_keystroke(c).is_none());
        }
        let r1 = eng.trigger_shift_backspace().expect("First switch");
        assert_eq!(r1.target_lang, Language::Thai);
        assert_eq!(r1.replacement_text, "ะำหะ");
        assert_eq!(r1.backspaces, 4);

        let r2 = eng.trigger_shift_backspace().expect("Undo switch");
        assert_eq!(r2.target_lang, Language::English);
        assert_eq!(r2.replacement_text, "test");
        assert_eq!(r2.backspaces, 4);

        let r3 = eng.trigger_shift_backspace().expect("Redo switch");
        assert_eq!(r3.target_lang, Language::Thai);
        assert_eq!(r3.replacement_text, "ะำหะ");
        assert_eq!(r3.backspaces, 4);
    }

    #[test]
    fn test_disabled_mode_does_nothing() {
        let mut eng = make_test_engine();
        eng.mode = CorrectionMode::Disabled;
        eng.active_language = Language::English;
        for c in "l;ylfu ".chars() {
            assert!(eng.process_keystroke(c).is_none());
        }
        assert!(eng.trigger_shift_backspace().is_none());

        eng.mode = CorrectionMode::Online;
        eng.set_active_process("WindowsTerminal.exe");
        for c in "l;ylfu ".chars() {
            assert!(eng.process_keystroke(c).is_none());
        }
        assert!(eng.trigger_shift_backspace().is_none());
    }

    #[test]
    fn test_lut_matches_pairs_100_percent() {
        for &(en, th) in US_TO_THAI_PAIRS {
            assert_eq!(us_to_thai_char(en), th);
            assert_eq!(thai_to_us_char(th), en);
        }
        for code in 0u32..128 {
            if let Some(c) = char::from_u32(code) {
                let expected_th = US_TO_THAI_PAIRS
                    .iter()
                    .find(|&&(e, _)| e == c)
                    .map(|&(_, t)| t)
                    .unwrap_or(c);
                assert_eq!(us_to_thai_char(c), expected_th);

                let expected_en = US_TO_THAI_PAIRS
                    .iter()
                    .find(|&&(_, t)| t == c)
                    .map(|&(e, _)| e)
                    .unwrap_or(c);
                assert_eq!(thai_to_us_char(c), expected_en);
            }
        }
        for code in 0x0E00u32..=0x0E7F {
            if let Some(c) = char::from_u32(code) {
                let expected_en = US_TO_THAI_PAIRS
                    .iter()
                    .find(|&&(_, t)| t == c)
                    .map(|&(e, _)| e)
                    .unwrap_or(c);
                assert_eq!(thai_to_us_char(c), expected_en);
            }
        }
    }

    fn simulate_app_typing(eng: &mut RightLangEngine, us_keys: &str) -> (String, usize) {
        let mut screen = String::new();
        let mut switch_count = 0usize;
        for c in us_keys.chars() {
            let shown = match eng.active_language {
                Language::English => c,
                Language::Thai => {
                    if c == ' ' || c == '\n' || c == '\t' {
                        c
                    } else {
                        us_to_thai_char(c)
                    }
                }
            };
            screen.push(shown);
            if let Some(rep) = eng.process_keystroke(c) {
                for _ in 0..rep.backspaces {
                    let _ = screen.pop();
                }
                screen.push_str(&rep.replacement_text);
                switch_count += 1;
            }
        }
        (screen, switch_count)
    }

    #[test]
    fn test_discord_online_offline_and_no_false_prefix_split() {
        let mut eng = make_test_engine();
        eng.active_language = Language::Thai;
        let (out, switches) = simulate_app_typing(&mut eng, "discord ");
        assert_eq!(out, "discord ");
        assert_eq!(switches, 1);

        let mut eng_dis = make_test_engine();
        eng_dis.active_language = Language::Thai;
        for c in "dis".chars() {
            assert!(
                eng_dis.process_keystroke(c).is_none(),
                "Should not mid-word switch on 'dis'"
            );
        }

        eng_dis.mode = CorrectionMode::Offline;
        let r_dis = eng_dis
            .trigger_shift_backspace()
            .expect("Expected Shift+Backspace on 'กรห'");
        assert_eq!(r_dis.target_lang, Language::English);
        assert_eq!(r_dis.replacement_text, "dis");
        assert_eq!(r_dis.backspaces, 3);

        let mut eng_off = make_test_engine();
        eng_off.mode = CorrectionMode::Offline;
        eng_off.active_language = Language::Thai;
        for c in "discord".chars() {
            assert!(eng_off.process_keystroke(c).is_none());
        }
        let r_discord = eng_off
            .trigger_shift_backspace()
            .expect("Expected Shift+Backspace on 'กรหแนพก'");
        assert_eq!(r_discord.target_lang, Language::English);
        assert_eq!(r_discord.replacement_text, "discord");
        assert_eq!(r_discord.backspaces, 7);

        let mut eng_func = make_test_engine();
        eng_func.mode = CorrectionMode::Offline;
        eng_func.active_language = Language::Thai;
        for c in "func".chars() {
            assert!(eng_func.process_keystroke(c).is_none());
        }
        let r_func = eng_func
            .trigger_shift_backspace()
            .expect("Expected Shift+Backspace on 'ดีืแ'");
        assert_eq!(r_func.replacement_text, "func");
        assert_eq!(r_func.backspaces, 4);

        let mut eng_diff = make_test_engine();
        eng_diff.mode = CorrectionMode::Offline;
        eng_diff.active_language = Language::Thai;
        for c in "diff".chars() {
            assert!(eng_diff.process_keystroke(c).is_none());
        }
        let r_diff = eng_diff
            .trigger_shift_backspace()
            .expect("Expected Shift+Backspace on 'กรดด'");
        assert_eq!(r_diff.replacement_text, "diff");
        assert_eq!(r_diff.backspaces, 4);

        let mut eng_sudo = make_test_engine();
        eng_sudo.active_language = Language::Thai;
        let (sudo_out, _) = simulate_app_typing(&mut eng_sudo, "sudo ");
        assert_eq!(sudo_out, "sudo ");
    }

    #[test]
    fn test_long_thai_midword_tail_in_not_reverted_at_space() {
        let mut eng = make_test_engine();
        eng.active_language = Language::English;
        let (out, switches) = simulate_app_typing(&mut eng, "c]tg8]upiN ");
        assert_eq!(out, "และเคลียร์ ");
        assert_eq!(switches, 1);
    }

    #[test]
    fn test_win11_notepad_and_word_leading_diacritic_backspaces() {
        for proc_name in ["notepad.exe", "winword.exe"] {
            let mut eng = make_test_engine();
            eng.set_active_process(proc_name);
            eng.active_language = Language::Thai;
            let mut rep_hell = None;
            for c in "hell".chars() {
                if let Some(r) = eng.process_keystroke(c) {
                    rep_hell = Some(r);
                    break;
                }
            }
            let r1 = rep_hell.expect("Expected switch on 'hell'");
            assert_eq!(r1.replacement_text, "hell");
            assert_eq!(r1.backspaces, 4);

            eng.reset_all_buffers();
            eng.active_language = Language::Thai;
            let mut rep_user = None;
            for c in "user".chars() {
                if let Some(r) = eng.process_keystroke(c) {
                    rep_user = Some(r);
                    break;
                }
            }
            let r2 = rep_user.expect("Expected switch on 'user'");
            assert_eq!(r2.replacement_text, "user");
            assert_eq!(r2.backspaces, 4);
        }
    }

    #[test]
    fn test_user_dict_garbage_filter_blocks_gaming_wasd_and_bad_thai() {
        assert!(!is_sane_user_word_en("aaaa"));
        assert!(!is_sane_user_word_en("wssssssssss"));
        assert!(!is_sane_user_word_en("wsdwdd"));
        assert!(!is_sane_user_word_en("daaaaaaaaaaaaaaaaaaawwraw"));
        assert!(is_sane_user_word_en("discord"));
        assert!(is_sane_user_word_en("chatgpt"));
        assert!(is_sane_user_word_en("keyswitch"));

        assert!(!is_sane_user_word_th("าำั"));
        assert!(!is_sane_user_word_th("ๅชุด"));
        assert!(!is_sane_user_word_th("ห๊าาาาาา"));
        assert!(is_sane_user_word_th("เคลีย"));
        assert!(is_sane_user_word_th("อัปเดต"));
        assert!(is_sane_user_word_th("ศุภกฤต"));
    }

    #[test]
    fn test_reconcile_replacement_queue_prevents_stranded_chars() {
        let rep = PendingOsReplacement {
            backspaces: 4,
            target_lang: Language::English,
            replacement_text: "disc".to_string(),
        };

        let p1 = reconcile_replacement_queue(&rep, &[DeferredKey::Printable('o')]);
        assert!(!p1.aborted);
        assert_eq!(p1.backspaces, 5);
        assert_eq!(p1.base_text, "disc");
        assert_eq!(p1.replay_keys, vec![DeferredKey::Printable('o')]);

        let p2 = reconcile_replacement_queue(&rep, &[DeferredKey::Backspace]);
        assert!(!p2.aborted);
        assert_eq!(p2.backspaces, 3);
        assert_eq!(p2.base_text, "dis");
        assert!(p2.replay_keys.is_empty());

        let p3 = reconcile_replacement_queue(
            &rep,
            &[DeferredKey::Printable('o'), DeferredKey::Backspace],
        );
        assert!(!p3.aborted);
        assert_eq!(p3.backspaces, 4);
        assert_eq!(p3.base_text, "disc");
        assert!(p3.replay_keys.is_empty());

        let p4 = reconcile_replacement_queue(&rep, &[DeferredKey::Boundary(' ')]);
        assert!(!p4.aborted);
        assert_eq!(p4.backspaces, 5);
        assert_eq!(p4.base_text, "disc");
        assert_eq!(p4.replay_keys, vec![DeferredKey::Boundary(' ')]);

        let p5 = reconcile_replacement_queue(&rep, &[DeferredKey::Boundary('\n')]);
        assert!(p5.aborted);
    }

    #[test]
    fn test_modern_english_words_and_fast_evaluation_latency() {
        for word in ["youtube ", "github ", "chatgpt ", "shopee ", "vscode "] {
            let mut eng = make_test_engine();
            eng.active_language = Language::Thai;
            let (out, switches) = simulate_app_typing(&mut eng, word);
            assert_eq!(out, word, "Failed auto-switch for '{word}'");
            assert!(switches >= 1, "Expected switch for '{word}'");
        }

        let mut eng = make_test_engine();
        let t0 = std::time::Instant::now();
        let mut total_keys = 0usize;

        eng.active_language = Language::Thai;
        for c in "discord ".chars() {
            let _ = eng.process_keystroke(c);
            total_keys += 1;
        }
        eng.reset_all_buffers();
        eng.active_language = Language::Thai;
        for c in "สวัสดีครับวันนี้สบายดีไหม ".chars() {
            let _ = eng.process_keystroke(c);
            total_keys += 1;
        }
        eng.reset_all_buffers();
        eng.active_language = Language::English;
        for c in "c]tg8]upiN ".chars() {
            let _ = eng.process_keystroke(c);
            total_keys += 1;
        }
        eng.reset_all_buffers();
        eng.active_language = Language::English;
        for c in "discord ".chars() {
            let _ = eng.process_keystroke(c);
            total_keys += 1;
        }

        let elapsed = t0.elapsed();
        let avg_ms = elapsed.as_secs_f64() * 1000.0 / (total_keys as f64);
        assert!(
            avg_ms < 5.0,
            "Evaluation too slow: {avg_ms:.3} ms/key ({elapsed:?} for {total_keys} keys)"
        );
    }
}
