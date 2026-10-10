//! PGN export of a game played in the app, with a comment after every model move saying where
//! it came from and a comment wherever the human took over the other side.

use crate::session::Session;

/// (year, month, day) of a timestamp in milliseconds since the Unix epoch, in UTC.
pub fn civil_date(ms: i64) -> (i64, u32, u32) {
    let z = ms.div_euclid(86_400_000) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

pub fn render(s: &Session) -> String {
    let model_name = format!("{} (model)", s.username);
    let (white, black) = if s.user_color == "white" {
        (s.username.clone(), model_name)
    } else {
        (model_name, s.username.clone())
    };
    // The result is reported from the side the human plays at the end of the game.
    let human = s.current_color.as_str();
    let result_token = match (s.status.as_str(), s.result.as_deref()) {
        ("playing", _) => "*",
        (_, Some("win")) if human == "white" => "1-0",
        (_, Some("win")) => "0-1",
        (_, Some("loss")) if human == "white" => "0-1",
        (_, Some("loss")) => "1-0",
        _ => "1/2-1/2",
    };
    let (y, m, d) = civil_date(s.created_at);
    let mut out = format!(
        "[Event \"You v You\"]\n[Site \"yvy-chess\"]\n[Date \"{y:04}.{m:02}.{d:02}\"]\n[White \"{white}\"]\n[Black \"{black}\"]\n[Result \"{result_token}\"]\n\n"
    );
    for (i, mv) in s.moves.iter().enumerate() {
        if let Some(sw) = s.switches.iter().find(|sw| sw.ply as usize == i) {
            out.push_str(&format!("{{you take over as {}}} ", sw.color));
        }
        if i % 2 == 0 {
            out.push_str(&format!("{}. ", i / 2 + 1));
        }
        out.push_str(&mv.san);
        let note = match mv.source.as_str() {
            "history" => mv.detail.as_ref().and_then(|d| {
                let games = d.get("games")?.as_i64()?;
                let count = d.get("chosen_count")?.as_i64()?;
                Some(format!(" {{you: {count}/{games}}}"))
            }),
            "engine" => mv.detail.as_ref().map(|d| {
                let name = d.get("engine").and_then(|v| v.as_str()).unwrap_or("engine");
                match d.get("elo").and_then(|v| v.as_u64()) {
                    Some(elo) => format!(" {{{name} {elo}}}"),
                    None => format!(" {{{name}}}"),
                }
            }),
            _ => None,
        };
        if let Some(note) = note {
            out.push_str(&note);
        }
        out.push(' ');
    }
    out.push_str(result_token);
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::civil_date;

    #[test]
    fn dates() {
        assert_eq!(civil_date(0), (1970, 1, 1));
        assert_eq!(civil_date(951_782_400_000), (2000, 2, 29));
        assert_eq!(civil_date(1_791_590_400_000), (2026, 10, 10));
        assert_eq!(civil_date(-86_400_000), (1969, 12, 31));
    }
}
