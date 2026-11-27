//! Deterministic text cleanup. Runs on every dictation (formatter on or
//! off) and again on the formatter's output as a safety net.

use crate::context::{Context, Kind};
use once_cell::sync::Lazy;
use regex::Regex;

/// English and Greek hesitation sounds ("um", "uh", "εεε", "εμ", "χμ").
static FILLERS: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(?:u+m+|u+h+|erm+|a+h+|hmm+|mm+|ε{2,}|εμ+|χμ+|α{3,})\b[,.]?").unwrap());
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
    // Greek. ("κόμμα" is handled separately: it also means a political
    // party. "τελεία" is handled with "period".)
    ("νέα παράγραφος", "\n\n"),
    ("καινούρια παράγραφος", "\n\n"),
    ("καινούργια παράγραφος", "\n\n"),
    ("νέα γραμμή", "\n"),
    ("καινούρια γραμμή", "\n"),
    ("καινούργια γραμμή", "\n"),
    ("ερωτηματικό", ";"),
    ("θαυμαστικό", "!"),
];

/// "άνω κάτω τελεία" (colon) must go before "τελεία" (full stop).
static GREEK_COLON: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)[,.]?\s*\bάνω κάτω τελεία\b[,.]?").unwrap());
/// Spoken "κόμμα", unless it follows an article ("το κόμμα" = the party).
static GREEK_COMMA: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)(\S+)?(\s*)\bκόμμα\b[,.]?").unwrap());
const GREEK_ARTICLES: &[&str] = &["το", "του", "τα", "των", "στο", "στα", "ένα", "κάθε", "ενός", "με", "σε"];

/// "period" only counts as punctuation at the end of the dictation or
/// right before a spoken line break, so "the trial period ends" survives.
static PERIOD: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)[,.]?\s+(?:period|full stop|τελεία)[,.]?(\s+(?:new line|newline|next line|new paragraph|next paragraph|νέα γραμμή|νέα παράγραφος|καινούρια γραμμή|καινούρια παράγραφος)\b|\s*$)").unwrap()
});

pub fn spoken_punctuation(s: &str) -> String {
    let s = GREEK_COLON.replace_all(s, ":");
    let s = GREEK_COMMA.replace_all(&s, |c: &regex::Captures| {
        let prev = c.get(1).map(|m| m.as_str()).unwrap_or("");
        let bare = prev.trim_matches(|ch: char| !ch.is_alphanumeric()).to_lowercase();
        if GREEK_ARTICLES.contains(&bare.as_str()) {
            c[0].to_string()
        } else {
            format!("{prev},")
        }
    });
    let mut out = PERIOD.replace_all(&s, ".$1").into_owned();
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
    let re = Regex::new(r"([,;:?!])([A-Za-zΑ-Ωα-ωΆ-Ώά-ώ])").unwrap();
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
        r"(?i)(?:,?\s+(?:actually,?\s+no,?\s+wait|actually,?\s+no|no,?\s+wait|wait,?\s+no|scratch that|sorry,?\s+I\s+mean|or rather|όχι,?\s+περίμενε|συγγνώμη,?\s+εννοώ|όχι,?\s+συγγνώμη)|,\s*actually,?)[,.]?\s+",
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

/// Mostly Greek letters? Then ";" is a question mark and ends a sentence.
fn is_greek(s: &str) -> bool {
    let (mut greek, mut other) = (0, 0);
    for c in s.chars().filter(|c| c.is_alphabetic()) {
        if matches!(c as u32, 0x0370..=0x03FF | 0x1F00..=0x1FFF) {
            greek += 1;
        } else {
            other += 1;
        }
    }
    greek > other
}

pub fn capitalize_sentences(s: &str) -> String {
    let greek = is_greek(s);
    let mut out = String::with_capacity(s.len());
    let mut cap = true;
    for ch in s.chars() {
        if cap && ch.is_alphabetic() {
            out.extend(ch.to_uppercase());
            cap = false;
            continue;
        }
        if matches!(ch, '.' | '!' | '?' | '\n') || (greek && matches!(ch, ';' | '\u{37e}')) {
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
    if words >= 4 && !s.ends_with(['.', '!', '?', ':', ')', '"', ';', '\u{37e}']) && kind != Kind::Commit && kind != Kind::Chat {
        s.push('.');
    }
    if kind == Kind::Commit {
        s = s.trim_end_matches('.').to_string();
    }
    if kind == Kind::Email {
        let (g, body, so) = split_email(&s);
        s = join_email(g.as_deref(), &paragraphs(&body), so.as_deref());
    } else if kind == Kind::Document {
        s = paragraphs(&s);
    }
    s
}

/// Long unbroken dictation reads as a wall of text. Group sentences into
/// paragraphs of roughly 60-90 words. Leaves short text and text that
/// already has line breaks alone (the speaker said "new paragraph").
pub fn paragraphs(s: &str) -> String {
    let words = s.split_whitespace().count();
    if words < 120 || s.contains('\n') {
        return s.to_string();
    }
    let mut out = String::new();
    let mut para = String::new();
    let mut para_words = 0;
    let mut sentence = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        sentence.push(c);
        let end = matches!(c, '.' | '!' | '?' | '\u{37e}') || (c == ';' && is_greek(s));
        if end && chars.peek().map_or(true, |n| n.is_whitespace()) {
            para_words += sentence.split_whitespace().count();
            para.push_str(&sentence);
            sentence.clear();
            if para_words >= 60 {
                if !out.is_empty() {
                    out.push_str("\n\n");
                }
                out.push_str(para.trim());
                para.clear();
                para_words = 0;
            }
        }
    }
    para.push_str(&sentence);
    if !para.trim().is_empty() {
        // A short tail joins the previous paragraph.
        if para_words < 25 && !out.is_empty() {
            out.push(' ');
        } else if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(para.trim());
    }
    out
}

/// Spoken greeting at the start ("hi Maria", "γεια σου Μαρία").
static GREETING: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^(hi|hello|hey|dear|good morning|good afternoon|γεια σου|γεια σας|καλημέρα|καλησπέρα|αγαπητέ|αγαπητή)\s+(\p{Lu}[\p{L}'-]*(?:\s+\p{Lu}[\p{L}'-]*)?)\s*[,!.]?\s+").unwrap()
});
/// Spoken sign-off and name at the end ("best, Alex", "φιλικά, Νίκος").
static SIGNOFF: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)[,.]?\s*\b(best regards|kind regards|warm regards|many thanks|thanks again|thank you|best wishes|best|regards|thanks|cheers|sincerely|φιλικά|με εκτίμηση|με φιλικούς χαιρετισμούς|χαιρετίσματα|ευχαριστώ πολύ|φιλιά)[,.]?\s+(\p{Lu}[\p{L}'-]*(?:\s+\p{Lu}[\p{L}'-]*){0,2})\.?\s*$").unwrap()
});

/// Split an email into (greeting, body, sign-off), as spoken. The
/// greeting and sign-off are kept verbatim and never sent to the model.
pub fn split_email(s: &str) -> (Option<String>, String, Option<String>) {
    let mut body = s.trim().to_string();
    let mut greeting = None;
    let mut signoff = None;
    if let Some(c) = GREETING.captures(&body) {
        let end = c.get(0).unwrap().end();
        greeting = Some(format!("{},", capitalize_sentences(&format!("{} {}", &c[1], &c[2]))));
        body = capitalize_sentences(body[end..].trim_start());
    }
    if let Some(c) = SIGNOFF.captures(&body) {
        let start = c.get(0).unwrap().start();
        let rest = body[..start].trim_end().to_string();
        // Only when it really closes the email, not a sentence ending in "thanks".
        if !rest.is_empty() {
            signoff = Some(format!("{},\n{}", capitalize_sentences(&c[1].to_lowercase()), &c[2]));
            body = if rest.ends_with(['.', '!', '?', ';', ':', '\u{37e}']) { rest } else { format!("{rest}.") };
        }
    }
    (greeting, body, signoff)
}

/// The bones of an email, deterministically: greeting on its own line,
/// sign-off and name on their own lines. Language-agnostic structure, so
/// it never risks the wording; the model (if used) handles paragraphs.
#[cfg_attr(not(test), allow(dead_code))]
pub fn email_shape(s: &str) -> String {
    let (g, body, so) = split_email(s);
    join_email(g.as_deref(), &body, so.as_deref())
}

pub fn join_email(greeting: Option<&str>, body: &str, signoff: Option<&str>) -> String {
    let mut parts: Vec<&str> = vec![];
    if let Some(g) = greeting {
        parts.push(g);
    }
    if !body.trim().is_empty() {
        parts.push(body.trim());
    }
    if let Some(so) = signoff {
        parts.push(so);
    }
    parts.join("\n\n")
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
    fn greek() {
        assert_eq!(remove_fillers("εεε λοιπόν χμμ θα έρθω"), "λοιπόν θα έρθω");
        assert_eq!(spoken_punctuation("γεια σου κόμμα τι κάνεις ερωτηματικό"), "Γεια σου, τι κάνεις;");
        assert_eq!(spoken_punctuation("ψήφισα το κόμμα της αρεσκείας μου"), "Ψήφισα το κόμμα της αρεσκείας μου");
        assert_eq!(spoken_punctuation("τα λέμε αύριο τελεία"), "Τα λέμε αύριο.");
        assert_eq!(spoken_punctuation("τρία πράγματα άνω κάτω τελεία ψωμί γάλα"), "Τρία πράγματα: ψωμί γάλα");
        assert_eq!(spoken_punctuation("πρώτη γραμμή νέα γραμμή δεύτερη γραμμή"), "Πρώτη γραμμή\nΔεύτερη γραμμή");
        assert_eq!(capitalize_sentences("ήρθες; ναι ήρθα."), "Ήρθες; Ναι ήρθα.");
        assert_eq!(
            self_corrections("Τα λέμε την Τρίτη στις τρεις, όχι περίμενε, στις τέσσερις."),
            "Τα λέμε την Τρίτη στις τέσσερις."
        );
        // English semicolons don't start sentences.
        assert_eq!(capitalize_sentences("we tried; it worked."), "We tried; it worked.");
    }

    #[test]
    fn paragraphs_long_text() {
        let s = "This is a sentence with exactly ten words in it ok. ".repeat(20);
        let p = paragraphs(s.trim());
        let paras: Vec<&str> = p.split("\n\n").collect();
        assert!(paras.len() >= 2 && paras.len() <= 4, "{}", paras.len());
        assert_eq!(p.split_whitespace().count(), 220);
        assert_eq!(paragraphs("Short text. Stays."), "Short text. Stays.");
    }

    #[test]
    fn email_bones() {
        assert_eq!(
            email_shape("Hi Maria, thanks for the draft. See you Friday. Best, Alex."),
            "Hi Maria,\n\nThanks for the draft. See you Friday.\n\nBest,\nAlex"
        );
        assert_eq!(
            email_shape("Γεια σου Μαρία, ευχαριστώ για το προσχέδιο. Τα λέμε την Παρασκευή. Φιλικά, Αλέξανδρος."),
            "Γεια σου Μαρία,\n\nΕυχαριστώ για το προσχέδιο. Τα λέμε την Παρασκευή.\n\nΦιλικά,\nΑλέξανδρος"
        );
        // "thanks" inside the body isn't a sign-off without a name after it.
        assert_eq!(email_shape("It works now, thanks."), "It works now, thanks.");
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
