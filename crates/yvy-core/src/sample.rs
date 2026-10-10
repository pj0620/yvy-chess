use rand::distr::{weighted::WeightedIndex, Distribution};
use rand::Rng;

/// How the model picks among the moves you played from a position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Pick a move with probability proportional to how often you played it.
    #[default]
    Sampled,
    /// Always pick the move you played most often (first in slice order on ties).
    MostFrequent,
}

/// Chooses an index into `counts`, or `None` when there is nothing to choose from.
pub fn choose_index<R: Rng + ?Sized>(counts: &[u32], mode: Mode, rng: &mut R) -> Option<usize> {
    if counts.iter().all(|&c| c == 0) {
        return None;
    }
    match mode {
        Mode::MostFrequent => {
            let mut best = 0;
            for (i, &c) in counts.iter().enumerate() {
                if c > counts[best] {
                    best = i;
                }
            }
            Some(best)
        }
        Mode::Sampled => WeightedIndex::new(counts.iter()).ok().map(|d| d.sample(rng)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{rngs::StdRng, SeedableRng};

    #[test]
    fn sampled_follows_the_frequencies() {
        let mut rng = StdRng::seed_from_u64(7);
        let counts = [3, 1];
        let mut hits = [0u32; 2];
        for _ in 0..20_000 {
            hits[choose_index(&counts, Mode::Sampled, &mut rng).unwrap()] += 1;
        }
        let share = hits[0] as f64 / 20_000.0;
        assert!((share - 0.75).abs() < 0.02, "share of first move was {share}");
    }

    #[test]
    fn most_frequent_is_deterministic_and_prefers_first_on_ties() {
        let mut rng = StdRng::seed_from_u64(1);
        assert_eq!(choose_index(&[2, 5, 5], Mode::MostFrequent, &mut rng), Some(1));
        assert_eq!(choose_index(&[1], Mode::MostFrequent, &mut rng), Some(0));
    }

    #[test]
    fn nothing_to_choose_from() {
        let mut rng = StdRng::seed_from_u64(1);
        assert_eq!(choose_index(&[], Mode::Sampled, &mut rng), None);
        assert_eq!(choose_index(&[0, 0], Mode::Sampled, &mut rng), None);
        assert_eq!(choose_index(&[0, 0], Mode::MostFrequent, &mut rng), None);
    }
}
