//! Terminal column display-width calculation, ANSI stripping, and wide-character-safe truncation/padding.

/// Returns the monospace terminal column width (`0`, `1`, or `2`) of `ch`.
#[must_use]
pub const fn char_display_width(ch: char) -> usize {
    let code = ch as u32;
    if code <= 0x1F || (code >= 0x7F && code <= 0x9F) {
        return 0;
    }
    if matches!(
        code,
        0x0300..=0x036F
            | 0x0483..=0x0489
            | 0x0591..=0x05BD
            | 0x05BF
            | 0x05C1..=0x05C2
            | 0x05C4..=0x05C5
            | 0x05C7
            | 0x0610..=0x061A
            | 0x064B..=0x065F
            | 0x0670
            | 0x06D6..=0x06DC
            | 0x06DF..=0x06E4
            | 0x06E7..=0x06E8
            | 0x06EA..=0x06ED
            | 0x1AB0..=0x1AFF
            | 0x1DC0..=0x1DFF
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x206F
            | 0x20D0..=0x20FF
            | 0xFE00..=0xFE0F
            | 0xFE20..=0xFE2F
            | 0xFEFF
    ) {
        return 0;
    }
    if matches!(
        code,
        0x1100..=0x115F
            | 0x231A..=0x231B
            | 0x2329..=0x232A
            | 0x23E9..=0x23F3
            | 0x23F8..=0x23FA
            | 0x2600..=0x27BF
            | 0x2E80..=0x303E
            | 0x3040..=0x33BF
            | 0x3400..=0x4DBF
            | 0x4E00..=0xA4CF
            | 0xA960..=0xA97C
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE10..=0xFE19
            | 0xFE30..=0xFE6B
            | 0xFF01..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F000..=0x1FAFF
            | 0x20000..=0x3FFFD
    ) {
        return 2;
    }
    1
}

/// Computes the display width in terminal columns of an ANSI-free string `s`.
#[must_use]
pub fn display_width(s: &str) -> usize {
    s.chars().map(char_display_width).sum()
}

/// Strips ANSI CSI escape sequences (`\x1b[...<final>`) from `s`.
#[must_use]
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            let _ = chars.next();
            for csi_ch in chars.by_ref() {
                let b = csi_ch as u32;
                if (0x40..=0x7E).contains(&b) {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Computes the visible display width in terminal columns of `s`, ignoring ANSI CSI sequences.
#[must_use]
pub fn visible_width(s: &str) -> usize {
    let mut total = 0;
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            let _ = chars.next();
            for csi_ch in chars.by_ref() {
                let b = csi_ch as u32;
                if (0x40..=0x7E).contains(&b) {
                    break;
                }
            }
        } else {
            total += char_display_width(ch);
        }
    }
    total
}

/// Truncates `s` on a UTF-8 codepoint boundary so its display width never exceeds `max_width`.
#[must_use]
pub fn truncate_to_width(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in s.chars() {
        let w = char_display_width(ch);
        if w == 0 {
            if !out.is_empty() {
                out.push(ch);
            }
            continue;
        }
        if used + w > max_width {
            break;
        }
        out.push(ch);
        used += w;
    }
    out
}

/// Truncates `s` to `target_width` and right-pads with ASCII spaces to reach `target_width` columns.
#[must_use]
pub fn pad_right_to_width(s: &str, target_width: usize) -> String {
    let mut out = truncate_to_width(s, target_width);
    let used = display_width(&out);
    if used < target_width {
        out.extend(std::iter::repeat_n(' ', target_width - used));
    }
    out
}

/// Truncates `s` to `target_width` and centers it within `target_width` columns using ASCII spaces.
#[must_use]
pub fn pad_center_to_width(s: &str, target_width: usize) -> String {
    let clipped = truncate_to_width(s, target_width);
    let used = display_width(&clipped);
    if used >= target_width {
        return clipped;
    }
    let rem = target_width - used;
    let left = rem / 2;
    let right = rem - left;
    let mut out = String::with_capacity(clipped.len() + rem);
    out.extend(std::iter::repeat_n(' ', left));
    out.push_str(&clipped);
    out.extend(std::iter::repeat_n(' ', right));
    out
}
