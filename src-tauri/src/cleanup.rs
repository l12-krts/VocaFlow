//! Deterministic text cleanup. Runs on every dictation (formatter on or
//! off) and again on the formatter's output as a safety net.

use crate::context::{Context, Kind};
use once_cell::sync::Lazy;
use regex::Regex;

static FILLERS: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b(?:u+m+|u+h+|erm+|a+h+|hmm+|mm+)\b[,.]?").unwrap());
static SPACE_BEFORE_PUNCT: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+([,.;:!?])").unwrap());
static MULTI_SPACE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[ \t]{2,}").unwrap());
static NUM_RANGE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(\d)\s*[\u{2014}\u{2013}]\s*(\d)").unwrap());
static DASHES: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s*[\u{2014}\u{2013}\u{2012}\u{2015}]\s*").unwrap());
static EMOJI: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"[\u{1F000}-\u{1FAFF}\u{2600}-\u{27BF}\u{2B00}-\u{2BFF}\u{FE0F}\u{200D}\u{1F1E6}-\u{1F1FF}]").unwrap()
});

/// Spoken punctuation, longest phrases first.
const SPOKEN: &[(&str, &str)] = &[
    ("new paragraph", "\n\n"),
    ("next paragraph", "\n\n"),
    ("new line", "\n"),
    ("newline", "\n"),
    ("next line", "\n"),
    ("question mark", "?"),
    ("exclamation mark", "!"),
    ("exclamation point", "!"),
    ("full stop", "."),
    ("period", "."),
    ("comma", ","),
    ("colon", ":"),
    ("semicolon", ";"),
    ("open paren", "("),
    ("close paren", ")"),
    ("open quote", "\""),
    ("close quote", "\""),
];

/// "period" only counts as punctuation at the end of the dictation or
/// right before a spoken line break, so "the trial period ends" survives.
static PERIOD: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)[,.]?\s+(?:period|full stop)[,.]?(\s+(?:new line|newline|next line|new paragraph|next paragraph)\b|\s*$)").unwrap()
});

pub fn spoken_punctuation(s: &str) -> String {
    let mut out = PERIOD.replace_all(s, ".$1").into_owned();
    for (phrase, sym) in SPOKEN {
        if matches!(*phrase, "period" | "full stop") {
            continue;
        }
        let sym = *sym;
        // Line breaks keep the sentence end before them; other symbols
        // replace any comma or period Whisper already guessed there.
        let lead = if sym.starts_with('\n') { "" } else { "[,.]?" };
        let re = Regex::new(&format!(r"(?i){lead}\s*\b{}\b[,.]?", regex::escape(phrase))).unwrap();
        out = re
            .replace_all(&out, |_: &regex::Captures| match sym {
                "\n" | "\n\n" | "(" | "\"" => format!(" {sym}"),
                _ => sym.to_string(),
            })
            .into_owned();
    }
    // Tidy whitespace around newlines and capitalise after them.
    let re = Regex::new(r"[ \t]*\n[ \t]*").unwrap();
    out = re.replace_all(&out, "\n").into_owned();
    // "word ,next" → "word, next"
    let re = Regex::new(r"([,;:?!])([A-Za-z])").unwrap();
    out = re.replace_all(&out, "$1 $2").into_owned();
    capitalize_sentences(out.trim())
}

/// Spoken self-corrections. "at 3, actually 4" → "at 4";
/// "before the timeline, actually no wait, after the timeline" →
/// "after the timeline". The replacement takes over as many preceding
/// words as it has itself (after dropping lead-ins like "make it"), which
/// matches how people restate a phrase. Done in code because small models
/// get this wrong in both directions.
static CORRECTION: Lazy<Regex> = Lazy::new(|| {
    // Leftmost-first: the long markers win over a bare ", actually".
    // A plain "I mean" is left alone ("I mean it") unless it follows "sorry".
    Regex::new(
        r"(?i)(?:,?\s+(?:actually,?\s+no,?\s+wait|actually,?\s+no|no,?\s+wait|wait,?\s+no|scratch that|sorry,?\s+I\s+mean|or rather)|,\s*actually,?)[,.]?\s+",
    )
    .unwrap()
});
static LEAD_IN: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^(?:make it|let's say|let's do|let's make it|it's|it should be|change (?:it|that) to)\s+").unwrap());

pub fn self_corrections(s: &str) -> String {
    let mut out = s.to_string();
    // Repeat: a sentence can hold more than one correction.
    for _ in 0..4 {
        let Some(m) = CORRECTION.find(&out) else { break };
        let before = &out[..m.start()];
        let after = &out[m.end()..];
        // The correction runs to the next clause boundary.
        let end = after.find(|c| matches!(c, ',' | '.' | '?' | '!' | ';' | '\n')).unwrap_or(after.len());
        let fix_raw = after[..end].trim();
        let fix = LEAD_IN.replace(fix_raw, "").to_string();
        let fix_words: Vec<&str> = fix.split_whitespace().collect();
        // Only short restatements; a long tail is real content, not a fix.
        if fix_words.is_empty() || fix_words.len() > 6 {
            // Not a correction we understand: leave the text, drop nothing.
            let marker = &out[m.start()..m.end()];
            if marker.trim().trim_matches(',').eq_ignore_ascii_case("actually") {
                break;
            }
            break;
        }
        // The clause being corrected: back to the previous boundary.
        let clause_start = before.rfind(|c| matches!(c, '.' | '?' | '!' | ';' | '\n')).map(|i| i + 1).unwrap_or(0);
        let clause: Vec<&str> = before[clause_start..].split_whitespace().collect();
        if clause.len() < fix_words.len() {
            break;
        }
        // Align on a shared tail when there is one ("before the timeline"
        // vs "after the timeline"), otherwise replace word-for-word.
        let n = fix_words.len();
        let kept = &clause[..clause.len() - n];
        let mut fixed = kept.join(" ");
        if !fixed.is_empty() {
            fixed.push(' ');
        }
        fixed.push_str(&fix);
        let prefix = &before[..clause_start];
        let sep = if prefix.is_empty() || prefix.ends_with(char::is_whitespace) { "" } else { " " };
        out = format!("{prefix}{sep}{fixed}{}", &after[end..]);
    }
    out
}

pub fn remove_fillers(s: &str) -> String {
    let out = FILLERS.replace_all(s, "");
    let out = MULTI_SPACE.replace_all(&out, " ");
    let out = out.replace(" ,", ",").replace(",,", ",").replace(" .", ".");
    Regex::new(r"^[,\s]+").unwrap().replace(out.trim(), "").into_owned()
}

/// House style: no em/en dashes, no emojis.
pub fn enforce_style(s: &str) -> String {
    let s = EMOJI.replace_all(s, "");
    let s = NUM_RANGE.replace_all(&s, "$1-$2");
    // "word — word" reads naturally as "word, word".
    let s = DASHES.replace_all(&s, ", ");
    let s = SPACE_BEFORE_PUNCT.replace_all(&s, "$1");
    let s = s.replace(",,", ",").replace(", .", ".").replace(",.", ".");
    let lines: Vec<String> = s.lines().map(|l| MULTI_SPACE.replace_all(l.trim_end(), " ").into_owned()).collect();
    lines.join("\n").trim().to_string()
}

pub fn capitalize_sentences(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut cap = true;
    for ch in s.chars() {
        if cap && ch.is_alphabetic() {
            out.extend(ch.to_uppercase());
            cap = false;
            continue;
        }
        if matches!(ch, '.' | '!' | '?' | '\n') {
            cap = true;
        } else if !ch.is_whitespace() && ch != '"' && ch != '(' {
            cap = false;
        }
        out.push(ch);
    }
    out
}

/// Rules-only pass used when the formatter is off, not ready, or the text
/// is too short to be worth a model call.
pub fn basic(raw: &str, kind: Kind) -> String {
    let mut s = self_corrections(&remove_fillers(raw));
    s = spoken_punctuation(&s);
    s = enforce_style(&s);
    if matches!(kind, Kind::Search | Kind::Terminal) {
        // Search boxes and shells: no sentence punctuation.
        return s.trim_end_matches(['.', '!']).to_string();
    }
    let words = s.split_whitespace().count();
    if words >= 4 && !s.ends_with(['.', '!', '?', ':', ')', '"']) && kind != Kind::Commit && kind != Kind::Chat {
        s.push('.');
    }
    if kind == Kind::Commit {
        s = s.trim_end_matches('.').to_string();
    }
    s
}

/// Fit the text to what's already in the field: spacing and casing at the
/// join, like a careful typist would.
pub fn join_with_field(text: &str, ctx: &Context) -> String {
    let mut t = text.to_string();
    let prev = ctx.before.chars().last();
    match prev {
        None => {}
        Some(c) if c.is_whitespace() => {
            let trimmed = ctx.before.trim_end();
            let mid_sentence = trimmed.chars().last().is_some_and(|c| c.is_alphanumeric() || c == ',');
            if mid_sentence && !ctx.before.ends_with('\n') {
                t = lower_first(&t);
            }
        }
        Some(c) if c.is_alphanumeric() || matches!(c, ',' | ';' | ':') => {
            t = format!(" {}", if c == ',' || c.is_alphanumeric() { lower_first(&t) } else { t });
        }
        Some(c) if matches!(c, '.' | '!' | '?') => t = format!(" {t}"),
        _ => {}
    }
    // Text continues right after the caret: don't leave a dangling period.
    if ctx.after.chars().next().is_some_and(|c| c.is_alphanumeric()) {
        t = t.trim_end_matches('.').to_string() + " ";
    }
    t
}

fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        // Keep "I", acronyms and proper-looking words ("NASA", "iPhone").
        Some(f) if f.is_uppercase() => {
            let rest: String = c.clone().collect();
            let word: String = s.split_whitespace().next().unwrap_or("").to_string();
            let is_acronym = word.chars().filter(|c| c.is_uppercase()).count() > 1;
            if word == "I" || word.starts_with("I'") || is_acronym {
                s.to_string()
            } else {
                f.to_lowercase().collect::<String>() + &rest
            }
        }
        _ => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fillers() {
        assert_eq!(remove_fillers("um so I think uh we should go"), "so I think we should go");
        assert_eq!(remove_fillers("the summary is fine"), "the summary is fine");
        assert_eq!(remove_fillers("Um, okay. Uh, let's go."), "okay. let's go.");
    }

    #[test]
    fn spoken() {
        assert_eq!(spoken_punctuation("hello comma how are you question mark"), "Hello, how are you?");
        assert_eq!(spoken_punctuation("first line new line second line"), "First line\nSecond line");
        assert_eq!(spoken_punctuation("the trial period ends soon"), "The trial period ends soon");
        assert_eq!(spoken_punctuation("see you tomorrow period"), "See you tomorrow.");
        assert_eq!(spoken_punctuation("hi team period new paragraph the build is green"), "Hi team.\n\nThe build is green");
    }

    #[test]
    fn corrections() {
        assert_eq!(self_corrections("Can we meet Tuesday at 3, actually make it 4?"), "Can we meet Tuesday at 4?");
        assert_eq!(
            self_corrections("We should move pricing before the timeline, actually no wait, after the timeline. Thanks."),
            "We should move pricing after the timeline. Thanks."
        );
        assert_eq!(self_corrections("Send it to John, sorry, I mean Jane."), "Send it to Jane.");
        assert_eq!(self_corrections("That is great, I mean it."), "That is great, I mean it.");
        assert_eq!(self_corrections("I actually like the new design."), "I actually like the new design.");
        assert_eq!(self_corrections("We're at 3 now."), "We're at 3 now.");
    }

    #[test]
    fn style() {
        assert_eq!(enforce_style("Great idea — let's ship it 🚀"), "Great idea, let's ship it");
        assert_eq!(enforce_style("2019–2021"), "2019-2021");
    }

    #[test]
    fn join() {
        let ctx = Context { before: "We met at".into(), ..Default::default() };
        assert_eq!(join_with_field("The cafe.", &ctx), " the cafe.");
        let ctx = Context { before: "Done. ".into(), ..Default::default() };
        assert_eq!(join_with_field("Next step.", &ctx), "Next step.");
        let ctx = Context { before: "Hi,".into(), ..Default::default() };
        assert_eq!(join_with_field("I think so.", &ctx), " I think so.");
    }
}
