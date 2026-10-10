import { ExternalLink } from "lucide-react";
import { gameLink, type Candidate } from "../api";
import { candidateShares, monthLabel } from "../lib/format";

/** Horizontal bars: how often each move was played from a position. */
export function CandidateBars({ candidates, chosen }: { candidates: Candidate[]; chosen?: string }) {
  const shares = candidateShares(candidates);
  if (candidates.length === 0) return <p className="muted">No games from this position.</p>;
  return (
    <ul className="bars">
      {candidates.map((c, i) => (
        <li key={c.uci} className={`bar-row${c.uci === chosen ? " bar-chosen" : ""}`}>
          <span className="bar-san">{c.san}</span>
          <span className="bar-track">
            <span className="bar-fill" style={{ width: `${Math.max(shares[i], 2)}%` }} />
          </span>
          <span className="bar-count">
            {c.count} <span className="muted">({Math.round(shares[i])}%)</span>
          </span>
          <span className="bar-when muted" title={c.last_played ? `last played ${new Date(c.last_played).toLocaleDateString()}` : undefined}>
            {monthLabel(c.last_played)}
          </span>
          {c.game_ids[0] &&
            (() => {
              const link = gameLink(c.game_ids[0]);
              return (
                <a
                  className="bar-link"
                  href={link.href}
                  target={link.external ? "_blank" : undefined}
                  rel={link.external ? "noreferrer" : undefined}
                  title={link.external ? "Open the most recent game on chess.com" : "Open the most recent game, played here"}
                >
                  <ExternalLink size={13} aria-hidden />
                </a>
              );
            })()}
        </li>
      ))}
    </ul>
  );
}
