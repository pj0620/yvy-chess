use shakmaty::{san::SanPlus, CastlingMode, Chess, Position};

use crate::key::position_key;

/// A legal move together with the position it leads to.
#[derive(Debug, Clone)]
pub struct Successor {
    pub uci: String,
    pub san: String,
    /// Key of the position after the move, see [`position_key`].
    pub key: String,
    pub pos: Chess,
}

/// Every legal move from `pos`, in move-generation order.
pub fn successors(pos: &Chess) -> Vec<Successor> {
    pos.legal_moves()
        .iter()
        .map(|m| {
            let mut next = pos.clone();
            let san = SanPlus::from_move_and_play_unchecked(&mut next, *m).to_string();
            Successor {
                uci: m.to_uci(CastlingMode::Standard).to_string(),
                san,
                key: position_key(&next),
                pos: next,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_position_has_twenty_successors_with_keys() {
        let s = successors(&Chess::default());
        assert_eq!(s.len(), 20);
        let e4 = s.iter().find(|x| x.uci == "e2e4").unwrap();
        assert_eq!(e4.san, "e4");
        assert_eq!(e4.key, "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq -");
        // Two plies deep: 20 replies to each of the 20 moves.
        let total: usize = s.iter().map(|x| successors(&x.pos).len()).sum();
        assert_eq!(total, 400);
    }

    #[test]
    fn transpositions_meet_two_plies_later() {
        // 1. Nf3 e5?! 2. e4 reaches the same position as 1. e4 e5 2. Nf3.
        let via_nf3 = successors(&Chess::default()).into_iter().find(|x| x.uci == "g1f3").unwrap();
        let after_e5 = successors(&via_nf3.pos).into_iter().find(|x| x.uci == "e7e5").unwrap();
        let after_e4 = successors(&after_e5.pos).into_iter().find(|x| x.uci == "e2e4").unwrap();
        let via_e4 = successors(&Chess::default()).into_iter().find(|x| x.uci == "e2e4").unwrap();
        let e5 = successors(&via_e4.pos).into_iter().find(|x| x.uci == "e7e5").unwrap();
        let nf3 = successors(&e5.pos).into_iter().find(|x| x.uci == "g1f3").unwrap();
        assert_eq!(after_e4.key, nf3.key);
    }
}
