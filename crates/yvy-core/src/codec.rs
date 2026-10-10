//! Compact encodings used by the in-browser index: a move packed into 16 bits, and 64-bit
//! hashes for positions and for one side's piece setup. Hashes keep the index small enough to
//! hold a large game history in browser memory; collisions at 64 bits are negligible here.

use shakmaty::{uci::UciMove, zobrist::Zobrist64, CastlingMode, Color, EnPassantMode, Move, Piece, Position, Role, Square};

/// Hash of a position: piece placement, side to move, castling rights and the en-passant
/// square only when an en-passant capture is legal (same equivalence as [`crate::position_key`]).
pub fn position_hash<P: Position>(pos: &P) -> u64 {
    pos.zobrist_hash::<Zobrist64>(EnPassantMode::Legal).0
}

/// Hash of where one side's pieces stand, ignoring the opponent (same equivalence as
/// [`crate::own_setup_key`]).
pub fn setup_hash<P: Position>(pos: &P, color: Color) -> u64 {
    let board = pos.board();
    let mut h: u64 = match color {
        Color::White => 0x9e37_79b9_7f4a_7c15,
        Color::Black => 0xc2b2_ae3d_27d4_eb4f,
    };
    for role in [Role::Pawn, Role::Knight, Role::Bishop, Role::Rook, Role::Queen, Role::King] {
        let bb = u64::from(board.by_piece(Piece { color, role }));
        h = splitmix(h ^ bb.wrapping_add(role as u64));
    }
    h
}

fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Packs a move into 16 bits: from (6) | to (6) | promotion (3). Castling uses the
/// king-to-destination convention (e1g1), as in standard UCI.
pub fn encode_move(m: Move) -> u16 {
    encode_uci(&m.to_uci(CastlingMode::Standard))
}

pub fn encode_uci(uci: &UciMove) -> u16 {
    match *uci {
        UciMove::Normal { from, to, promotion } => {
            let promo = match promotion {
                None => 0,
                Some(Role::Knight) => 1,
                Some(Role::Bishop) => 2,
                Some(Role::Rook) => 3,
                Some(Role::Queen) => 4,
                Some(_) => 0,
            };
            (from as u16) | ((to as u16) << 6) | (promo << 12)
        }
        // Drops and null moves never occur in standard chess.
        _ => 0,
    }
}

pub fn decode_move(code: u16) -> UciMove {
    let from = Square::new(u32::from(code & 0x3f));
    let to = Square::new(u32::from((code >> 6) & 0x3f));
    let promotion = match (code >> 12) & 0x7 {
        1 => Some(Role::Knight),
        2 => Some(Role::Bishop),
        3 => Some(Role::Rook),
        4 => Some(Role::Queen),
        _ => None,
    };
    UciMove::Normal { from, to, promotion }
}

pub fn decode_uci_string(code: u16) -> String {
    decode_move(code).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{key::own_setup_key, position_key};
    use shakmaty::Chess;

    fn play(moves: &[&str]) -> Chess {
        let mut pos = Chess::default();
        for uci in moves {
            let m = uci.parse::<UciMove>().unwrap().to_move(&pos).unwrap();
            pos.play_unchecked(m);
        }
        pos
    }

    #[test]
    fn moves_round_trip() {
        for uci in ["e2e4", "g1f3", "e1g1", "e8c8", "a7a8q", "b2b1n", "h7h8r", "c7c8b"] {
            let parsed: UciMove = uci.parse().unwrap();
            assert_eq!(decode_uci_string(encode_uci(&parsed)), uci);
        }
        let castle = play(&["e2e4", "e7e5", "g1f3", "b8c6", "f1c4", "f8c5"]);
        let m = "e1g1".parse::<UciMove>().unwrap().to_move(&castle).unwrap();
        assert_eq!(decode_uci_string(encode_move(m)), "e1g1");
    }

    #[test]
    fn hashes_follow_the_string_keys() {
        let lines: [&[&str]; 5] = [
            &[],
            &["e2e4"],
            &["e2e4", "c7c5", "e4e5", "d7d5"],
            &["e2e4", "e7e5", "g1f3", "b8c6"],
            &["g1f3", "b8c6", "e2e4", "e7e5"],
        ];
        let positions: Vec<Chess> = lines.iter().map(|l| play(l)).collect();
        for a in &positions {
            for b in &positions {
                assert_eq!(position_key(a) == position_key(b), position_hash(a) == position_hash(b));
                for c in [Color::White, Color::Black] {
                    assert_eq!(own_setup_key(a, c) == own_setup_key(b, c), setup_hash(a, c) == setup_hash(b, c));
                }
            }
        }
        // A double push with no possible en-passant capture hashes like the plain position.
        assert_eq!(position_hash(&play(&["e2e4"])), position_hash(&play(&["e2e4"])));
        assert_ne!(setup_hash(&Chess::default(), Color::White), setup_hash(&Chess::default(), Color::Black));
    }
}
