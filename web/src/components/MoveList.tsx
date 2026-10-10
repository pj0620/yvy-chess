import { ArrowLeftRight, Compass, Cpu, Dices, User } from "lucide-react";
import type { MoveOut, Switch } from "../api";
import { describeMove, movePairs } from "../lib/format";

function Cell({ move, selected, onSelect, switched }: { move?: MoveOut; selected: boolean; onSelect: (ply: number) => void; switched?: Switch }) {
  if (!move) return <span className="move-cell move-empty" />;
  const d = describeMove(move);
  const Icon =
    d.kind === "history" ? User : d.kind === "stockfish" ? Cpu : d.kind === "steer" ? Compass : d.kind === "random" ? Dices : null;
  return (
    <button
      type="button"
      className={`move-cell move-${d.kind}${selected ? " move-selected" : ""}`}
      onClick={() => onSelect(move.ply)}
      title={(switched ? `you took over ${switched.color} here. ` : "") + (d.sub ? `${d.label}: ${d.sub}` : d.label)}
    >
      {switched && <ArrowLeftRight size={11} strokeWidth={2.6} className="move-switch" aria-label={`you took over ${switched.color}`} />}
      <span className="move-san">{move.san}</span>
      {Icon && <Icon size={12} strokeWidth={2.4} aria-label={d.label} />}
    </button>
  );
}

export function MoveList({
  moves,
  selected,
  onSelect,
  switches = [],
}: {
  moves: MoveOut[];
  selected: number | null;
  onSelect: (ply: number) => void;
  switches?: Switch[];
}) {
  const pairs = movePairs(moves);
  const before = (m?: MoveOut) => (m ? switches.find((sw) => sw.ply === m.ply - 1) : undefined);
  if (pairs.length === 0) return <p className="muted">No moves yet.</p>;
  return (
    <ol className="movelist">
      {pairs.map((p) => (
        <li key={p.number} className="move-pair">
          <span className="move-number">{p.number}.</span>
          <Cell move={p.white} selected={selected === p.white?.ply} onSelect={onSelect} switched={before(p.white)} />
          <Cell move={p.black} selected={selected === p.black?.ply} onSelect={onSelect} switched={before(p.black)} />
        </li>
      ))}
    </ol>
  );
}
