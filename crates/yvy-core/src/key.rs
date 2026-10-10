use shakmaty::{fen::Epd, Color, EnPassantMode, Position, Square};

/// Canonical key for a position: piece placement, side to move, castling rights and the
/// en-passant square. The en-passant square is written only when an en-passant capture is
/// actually legal, so two identical positions never get different keys because one of them
/// happened to follow a double pawn push. Move counters are excluded, so the same position
/// reached by a different move order (a transposition) produces the same key.
pub fn position_key<P: Position>(pos: &P) -> String {
    Epd::from_position(pos, EnPassantMode::Legal).to_string()
}

/// Placement of one side's pieces only, as a FEN-style board with the other side removed.
/// Two positions share a setup key when the player's own pieces stand on the same squares,
/// whatever the opponent has done. This is the looser notion of "a position you have been in"
/// used to steer gap-filling moves toward configurations the player actually reaches.
pub fn own_setup_key<P: Position>(pos: &P, color: Color) -> String {
    let board = pos.board();
    let mut out = String::with_capacity(72);
    for rank in (0..8u32).rev() {
        let mut empty = 0u8;
        for file in 0..8u32 {
            match board.piece_at(Square::new(rank * 8 + file)) {
                Some(piece) if piece.color == color => {
                    if empty > 0 {
                        out.push(char::from(b'0' + empty));
                        empty = 0;
                    }
                    out.push(piece.char());
                }
                _ => empty += 1,
            }
        }
        if empty > 0 {
            out.push(char::from(b'0' + empty));
        }
        if rank > 0 {
            out.push('/');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use shakmaty::{uci::UciMove, Chess};

    fn play(moves: &[&str]) -> Chess {
        let mut pos = Chess::default();
        for uci in moves {
            let m = uci.parse::<UciMove>().unwrap().to_move(&pos).unwrap();
            pos.play_unchecked(m);
        }
        pos
    }

    #[test]
    fn start_position_key() {
        assert_eq!(
            position_key(&Chess::default()),
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq -"
        );
    }

    #[test]
    fn en_passant_square_only_when_capture_is_legal() {
        // After 1. e4 no black pawn can capture en passant, so the key must not mention e3.
        assert_eq!(
            position_key(&play(&["e2e4"])),
            "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq -"
        );
        // After 1. e4 c5 2. e5 d5 the pawn on e5 can capture on d6, so d6 is part of the key.
        assert!(position_key(&play(&["e2e4", "c7c5", "e4e5", "d7d5"])).ends_with(" w KQkq d6"));
    }

    #[test]
    fn transpositions_share_a_key() {
        let a = play(&["e2e4", "e7e5", "g1f3", "b8c6"]);
        let b = play(&["g1f3", "b8c6", "e2e4", "e7e5"]);
        assert_eq!(position_key(&a), position_key(&b));
    }

    #[test]
    fn own_setup_ignores_the_opponent() {
        // Black's setup after 1. e4 e5 and after 1. d4 e5 is identical: only Black's pieces count.
        let a = play(&["e2e4", "e7e5"]);
        let b = play(&["d2d4", "e7e5"]);
        assert_eq!(own_setup_key(&a, Color::Black), own_setup_key(&b, Color::Black));
        assert_ne!(own_setup_key(&a, Color::White), own_setup_key(&b, Color::White));
        assert_eq!(own_setup_key(&Chess::default(), Color::White), "8/8/8/8/8/8/PPPPPPPP/RNBQKBNR");
        assert_eq!(own_setup_key(&a, Color::Black), "rnbqkbnr/pppp1ppp/8/4p3/8/8/8/8");
    }
}
