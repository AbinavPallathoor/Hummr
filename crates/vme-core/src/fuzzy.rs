//! Tiny fuzzy matcher for short lists (instrument names, commands).
//!
//! Query characters must appear in order in the candidate (case-insensitive,
//! spaces in the query ignored). Consecutive matches, matches at word starts
//! and an early first match score higher.

/// Returns a score (higher is better) or `None` if `query` does not match.
pub fn score(query: &str, candidate: &str) -> Option<i32> {
    let q: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    if q.is_empty() {
        return Some(0);
    }
    let c: Vec<char> = candidate.chars().flat_map(char::to_lowercase).collect();
    let mut qi = 0;
    let mut total = 0i32;
    let mut prev_match: Option<usize> = None;
    let mut first: Option<usize> = None;
    for (i, &ch) in c.iter().enumerate() {
        if qi < q.len() && ch == q[qi] {
            let mut s = 1;
            let word_start = i == 0 || !c[i - 1].is_alphanumeric();
            if word_start {
                s += 8;
            }
            if prev_match == Some(i.wrapping_sub(1)) {
                s += 5;
            }
            total += s;
            first.get_or_insert(i);
            prev_match = Some(i);
            qi += 1;
        }
    }
    if qi < q.len() {
        return None;
    }
    let first = first.unwrap_or(0) as i32;
    Some(total * 4 - first - c.len() as i32 / 4)
}

/// Ranks `candidates` by score, best first, dropping non-matches.
pub fn rank<'a, T>(
    query: &str,
    candidates: impl IntoIterator<Item = (T, &'a str)>,
) -> Vec<(T, i32)> {
    let mut out: Vec<(T, i32)> = candidates
        .into_iter()
        .filter_map(|(t, name)| score(query, name).map(|s| (t, s)))
        .collect();
    out.sort_by_key(|x| std::cmp::Reverse(x.1));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subsequence_required() {
        assert!(score("vln", "Violin").is_some());
        assert!(score("xyz", "Violin").is_none());
        assert_eq!(score("", "Violin"), Some(0));
    }

    #[test]
    fn prefers_word_starts_and_runs() {
        let a = score("vio", "Violin").unwrap();
        let b = score("vio", "Synth Voice").unwrap_or(i32::MIN);
        assert!(a > b);
        let names = [
            (0, "Viola"),
            (1, "Violin"),
            (2, "Electric Piano 1"),
            (3, "Vibraphone"),
        ];
        let ranked = rank("violin", names);
        assert_eq!(ranked[0].0, 1);
        let ranked = rank("ep", [(0, "Electric Piano 1"), (1, "Sleep Pad")]);
        assert_eq!(ranked[0].0, 0);
    }
}
