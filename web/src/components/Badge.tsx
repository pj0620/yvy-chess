import { Compass, Cpu, Dices, User } from "lucide-react";

export type BadgeKind = "you" | "history" | "stockfish" | "steer" | "random";

const ICONS = {
  you: User,
  history: User,
  stockfish: Cpu,
  steer: Compass,
  random: Dices,
} as const;

const DEFAULT_LABELS: Record<BadgeKind, string> = {
  you: "You",
  history: "From your games",
  stockfish: "Stockfish",
  steer: "Stockfish, steering back toward your games",
  random: "Random move",
};

/** The human / engine marker that says where a move came from. */
export function Badge({ kind, label, size = "sm" }: { kind: BadgeKind; label?: string; size?: "sm" | "lg" }) {
  const Icon = ICONS[kind];
  return (
    <span className={`badge badge-${kind} badge-${size}`} title={DEFAULT_LABELS[kind]}>
      <Icon size={size === "lg" ? 18 : 13} strokeWidth={2.4} aria-hidden />
      <span>{label ?? DEFAULT_LABELS[kind]}</span>
    </span>
  );
}
