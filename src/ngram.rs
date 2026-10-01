use crate::config::Config;
use crate::tokenize::{self, Token};

pub const MAX_ORDER: usize = 3;

#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    pub order_n: usize,
    pub context: String,
    pub token: String,
    pub display: String,
    pub count: u64,
    pub mass: f64,
    pub last_seen_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scored {
    pub display: String,
    pub token: String,
    pub count: u64,
    pub probability: f64,
    pub mass: f64,
    pub order_n: usize,
}

/// Context key for the `n` words preceding the candidate. Order 1 uses an empty key.
pub fn context_key(preceding: &[Token], order_n: usize) -> Option<String> {
    if !(1..=MAX_ORDER).contains(&order_n) {
        return None;
    }
    let need = order_n - 1;
    if preceding.len() < need {
        return None;
    }
    let start = preceding.len() - need;
    Some(
        preceding[start..]
            .iter()
            .map(|token| token.normalized.as_str())
            .collect::<Vec<_>>()
            .join("\u{1f}"),
    )
}

pub fn decayed_mass(mass: f64, last_seen_ms: i64, now_ms: i64, half_life_days: f64) -> f64 {
    if mass <= 0.0 || half_life_days <= 0.0 {
        return 0.0;
    }
    let age_ms = (now_ms - last_seen_ms).max(0) as f64;
    let age_days = age_ms / 86_400_000.0;
    let lambda = std::f64::consts::LN_2 / half_life_days;
    mass * (-lambda * age_days).exp()
}

/// Touch an observation: decay the stored mass, then add one new use.
pub fn touch(mass: f64, last_seen_ms: i64, now_ms: i64, half_life_days: f64) -> f64 {
    decayed_mass(mass, last_seen_ms, now_ms, half_life_days) + 1.0
}

/// Observations produced by one submitted prompt. Full prompt text is not retained.
pub fn observations_for(tokens: &[Token], now_ms: i64) -> Vec<Observation> {
    let mut out = Vec::new();
    for index in 0..tokens.len() {
        for order_n in 1..=MAX_ORDER.min(index + 1) {
            let need = order_n - 1;
            let start = index - need;
            let context = tokens[start..index]
                .iter()
                .map(|token| token.normalized.as_str())
                .collect::<Vec<_>>()
                .join("\u{1f}");
            out.push(Observation {
                order_n,
                context,
                token: tokens[index].normalized.clone(),
                display: tokens[index].display.clone(),
                count: 1,
                mass: 1.0,
                last_seen_ms: now_ms,
            });
        }
    }
    out
}

pub fn rank<'a>(
    rows: impl IntoIterator<Item = &'a Observation>,
    prefix: &str,
    config: &Config,
    now_ms: i64,
) -> Option<Scored> {
    let prefix_norm = tokenize::normalize(prefix);
    if prefix_norm.chars().count() < 2 {
        return None;
    }
    let rows: Vec<&Observation> = rows.into_iter().collect();
    for order_n in (1..=MAX_ORDER).rev() {
        let matching: Vec<&Observation> = rows
            .iter()
            .copied()
            .filter(|row| row.order_n == order_n && row.token.starts_with(&prefix_norm))
            .filter(|row| row.count >= config.min_support)
            .collect();
        if matching.is_empty() {
            continue;
        }
        let total: f64 = rows
            .iter()
            .copied()
            .filter(|row| row.order_n == order_n && row.context == matching[0].context)
            .map(|row| decayed_mass(row.mass, row.last_seen_ms, now_ms, config.half_life_days))
            .sum();
        if total <= 0.0 {
            continue;
        }
        let mut best: Option<Scored> = None;
        for row in matching {
            let mass = decayed_mass(row.mass, row.last_seen_ms, now_ms, config.half_life_days);
            let probability = mass / total;
            if probability < config.min_confidence {
                continue;
            }
            let replace = best.as_ref().is_none_or(|current| {
                mass > current.mass
                    || (mass == current.mass
                        && row.last_seen_ms > 0
                        && row.display < current.display)
            });
            if replace {
                best = Some(Scored {
                    display: row.display.clone(),
                    token: row.token.clone(),
                    count: row.count,
                    probability,
                    mass,
                    order_n,
                });
            }
        }
        if best.is_some() {
            return best;
        }
    }
    None
}

/// Suffix to draw after `prefix`, using the stored surface form and the user's case.
pub fn suffix_for(display: &str, prefix: &str) -> Option<String> {
    let display_chars: Vec<char> = display.chars().collect();
    let prefix_len = prefix.chars().count();
    if display_chars.len() <= prefix_len {
        return None;
    }
    let head: String = display_chars[..prefix_len].iter().collect();
    if tokenize::normalize(&head) != tokenize::normalize(prefix) {
        return None;
    }
    let mut suffix: String = display_chars[prefix_len..].iter().collect();
    if prefix.chars().all(|ch| ch.is_uppercase()) && prefix.chars().any(|ch| ch.is_alphabetic()) {
        suffix = suffix.to_uppercase();
    } else if prefix.chars().all(|ch| !ch.is_uppercase()) {
        suffix = suffix.to_lowercase();
    }
    if suffix.is_empty() {
        None
    } else {
        Some(suffix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(
        order_n: usize,
        context: &str,
        token: &str,
        display: &str,
        count: u64,
        mass: f64,
        last_seen_ms: i64,
    ) -> Observation {
        Observation {
            order_n,
            context: context.to_string(),
            token: token.to_string(),
            display: display.to_string(),
            count,
            mass,
            last_seen_ms,
        }
    }

    fn cfg() -> Config {
        Config {
            min_confidence: 0.15,
            min_support: 2,
            half_life_days: 30.0,
            ..Config::default()
        }
    }

    #[test]
    fn backoff_uses_a_shorter_context_when_the_long_one_is_sparse() {
        let rows = vec![
            obs(3, "can\u{1f}you", "help", "help", 1, 1.0, 0),
            obs(2, "you", "help", "help", 4, 4.0, 0),
            obs(2, "you", "hear", "hear", 1, 1.0, 0),
        ];
        let scored = rank(&rows, "he", &cfg(), 0).unwrap();
        assert_eq!(scored.token, "help");
        assert_eq!(scored.order_n, 2);
        assert!(scored.probability > 0.15);
    }

    #[test]
    fn weak_evidence_returns_nothing() {
        let rows = vec![obs(1, "", "help", "help", 1, 1.0, 0)];
        assert!(rank(&rows, "he", &cfg(), 0).is_none());
    }

    #[test]
    fn recency_decay_prefers_the_recent_word() {
        let mut config = cfg();
        config.half_life_days = 1.0;
        config.min_confidence = 0.05;
        let day = 86_400_000;
        let rows = vec![
            obs(1, "", "help", "help", 10, 10.0, 0),
            obs(1, "", "hello", "hello", 3, 3.0, 10 * day),
        ];
        let scored = rank(&rows, "he", &config, 10 * day).unwrap();
        assert_eq!(scored.token, "hello");
    }

    #[test]
    fn suffix_preserves_stored_case_and_matches_shouting() {
        assert_eq!(suffix_for("Help", "he").as_deref(), Some("lp"));
        assert_eq!(suffix_for("Help", "HE").as_deref(), Some("LP"));
        assert_eq!(suffix_for("Help", "He").as_deref(), Some("lp"));
        assert!(suffix_for("he", "he").is_none());
    }

    #[test]
    fn observations_do_not_contain_the_prompt_sentence() {
        let tokens = tokenize::tokenize("please help me");
        let rows = observations_for(&tokens, 5);
        assert!(rows.iter().all(|row| !row.context.contains(' ')));
        assert!(rows
            .iter()
            .any(|row| row.order_n == 2 && row.context == "please" && row.token == "help"));
        assert!(!rows.iter().any(|row| row.token.contains("please help")));
    }
}
