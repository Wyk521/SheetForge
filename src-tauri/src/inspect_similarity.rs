//! Exact similarity suggestions with a lossless bigram candidate filter.
use crate::file_cache::Cache;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};
type Pairs = Vec<(String, String)>;
static RESULTS: OnceLock<Mutex<Cache<Vec<String>, Arc<Pairs>>>> = OnceLock::new();

pub(crate) fn similar_headers(mut names: Vec<String>) -> Arc<Pairs> {
    names.sort();
    names.dedup();
    let cache = RESULTS.get_or_init(|| Mutex::new(Cache::new(8, 8 * 1024 * 1024)));
    if let Some(result) = cache.lock().ok().and_then(|mut cache| cache.get(&names)) {
        return result;
    }
    let result = Arc::new(find_pairs(&names, 8));
    let bytes = names.iter().map(|s| s.capacity() + 24).sum::<usize>()
        + result
            .iter()
            .map(|(a, b)| a.len() + b.len() + 48)
            .sum::<usize>();
    if let Ok(mut cache) = cache.lock() {
        cache.insert(names, result.clone(), bytes);
    }
    result
}
fn find_pairs(names: &[String], limit: usize) -> Pairs {
    let mut postings = HashMap::<u64, Vec<(usize, usize)>>::new();
    let mut grams = Vec::with_capacity(names.len());
    let mut lengths = Vec::with_capacity(names.len());
    for (index, name) in names.iter().enumerate() {
        let chars = name.chars().collect::<Vec<_>>();
        lengths.push(chars.len());
        let mut counts = HashMap::<u64, usize>::new();
        for pair in chars.windows(2) {
            *counts
                .entry(((pair[0] as u64) << 32) | pair[1] as u64)
                .or_default() += 1;
        }
        for (&gram, &count) in &counts {
            postings.entry(gram).or_default().push((index, count));
        }
        grams.push(counts);
    }
    let mut result = Vec::new();
    for left in 0..names.len() {
        let mut intersections = HashMap::<usize, usize>::new();
        for (gram, count) in &grams[left] {
            for &(right, other_count) in &postings[gram] {
                if right > left {
                    *intersections.entry(right).or_default() += count.min(&other_count);
                }
            }
        }
        let mut candidates = intersections.into_iter().collect::<Vec<_>>();
        candidates.sort_unstable_by_key(|(right, _)| *right);
        for (right, common) in candidates {
            let longest = lengths[left].max(lengths[right]);
            // One edit destroys at most two bigrams. Ceil is deliberately loose
            // at the floating point threshold; the final score remains exact.
            let edits = (0.28 * longest as f64).ceil() as usize;
            if longest < 4
                || lengths[left].abs_diff(lengths[right]) > edits
                || common < longest.saturating_sub(1 + 2 * edits).max(1)
            {
                continue;
            }
            let score = strsim::normalized_levenshtein(&names[left], &names[right]);
            if (0.72..1.0).contains(&score) {
                result.push((names[left].clone(), names[right].clone()));
                if result.len() == limit {
                    return result;
                }
            }
        }
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn candidate_filter_preserves_naive_unicode_edit_distance_matches() {
        let mut names = vec![
            "abc".into(),
            "abcd".into(),
            "abce".into(),
            "aab".into(),
            "aaab".into(),
            "aaaa".into(),
            "baaa".into(),
            "中文客户编号".into(),
            "中文客户编码".into(),
            "🙂🙂🙂🙂".into(),
            "🙂🙂🙂a".into(),
        ];
        let alphabet = ['a', 'b', 'c', '1', '中', '文', '🙂'];
        let mut seed = 7u64;
        for n in 0..120 {
            let len = n % 35 + 1;
            let value = (0..len)
                .map(|_| {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                    alphabet[(seed >> 32) as usize % alphabet.len()]
                })
                .collect::<String>();
            names.push(value.clone());
            names.push(format!("{value}a"));
        }
        names.sort();
        names.dedup();
        let mut expected = vec![];
        for a in 0..names.len() {
            for b in a + 1..names.len() {
                if (0.72..1.0).contains(&strsim::normalized_levenshtein(&names[a], &names[b])) {
                    expected.push((names[a].clone(), names[b].clone()));
                }
            }
        }
        assert_eq!(find_pairs(&names, usize::MAX), expected);
    }
}
