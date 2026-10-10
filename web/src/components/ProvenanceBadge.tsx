import type { MoveOut } from "../api";
import { describeMove } from "../lib/format";
import { Badge } from "./Badge";

export function ProvenanceBadge({ move, size = "sm" }: { move: MoveOut; size?: "sm" | "lg" }) {
  const d = describeMove(move);
  return (
    <span className="provenance">
      <Badge kind={d.kind} label={d.label} size={size} />
      {size === "lg" && d.sub && <span className="provenance-sub">{d.sub}</span>}
    </span>
  );
}
