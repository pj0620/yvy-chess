import { useEffect, useMemo, useState, type CSSProperties } from "react";
import { Link, useNavigate, useParams } from "react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Chessboard, type ChessboardOptions, type PositionDataType } from "react-chessboard";
import {
  ArrowLeftRight,
  ChevronFirst,
  ChevronLast,
  ChevronLeft,
  ChevronRight,
  Download,
  Eye,
  EyeOff,
  Flag,
  LoaderCircle,
  Plus,
  Radio,
  Repeat,
  RotateCcw,
  Swords,
} from "lucide-react";
import { api, download, timeClassLabel, type SessionState } from "../api";
import { CandidateBars } from "../components/CandidateBars";
import { MoveList } from "../components/MoveList";
import { ProvenanceBadge } from "../components/ProvenanceBadge";
import { applyUci, fenToPosition, positionAtPly } from "../lib/board";
import { dayLabel, isEngine, isHistory, statusText, steerLabel } from "../lib/format";

const PROMOTIONS: { code: "q" | "r" | "b" | "n"; label: string; white: string; black: string }[] = [
  { code: "q", label: "Queen", white: "♕", black: "♛" },
  { code: "r", label: "Rook", white: "♖", black: "♜" },
  { code: "b", label: "Bishop", white: "♗", black: "♝" },
  { code: "n", label: "Knight", white: "♘", black: "♞" },
];

/** How long a move takes to glide across the board. */
const ANIMATION_MS = 300;
/**
 * A reply from your history comes back in a few milliseconds, which lands before your own move
 * has finished animating and reads as a snap. Hold the reply until at least this long after
 * the move was sent so it arrives as a visible answer.
 */
const MIN_REPLY_MS = 600;

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

export function Game() {
  const { id = "" } = useParams();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const session = useQuery({ queryKey: ["session", id], queryFn: () => api.session(id) });
  const s = session.data;

  const [flipped, setFlipped] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const [promotion, setPromotion] = useState<{ from: string; to: string } | null>(null);
  const [optimistic, setOptimistic] = useState<PositionDataType | null>(null);
  const [showHint, setShowHint] = useState(false);
  /** Ply being looked at while stepping through the game; null means the live position. */
  const [viewPly, setViewPly] = useState<number | null>(null);

  const apply = (next: SessionState) => {
    qc.setQueryData(["session", id], next);
    setOptimistic(null);
    setViewPly(null);
  };
  const move = useMutation({
    mutationFn: async (uci: string) => {
      const started = Date.now();
      const next = await api.move(id, uci);
      const last = next.moves[next.moves.length - 1];
      const remaining = MIN_REPLY_MS - (Date.now() - started);
      if (last && last.source !== "player" && remaining > 0) await sleep(remaining);
      return next;
    },
    onSuccess: apply,
    onError: () => setOptimistic(null),
  });
  const undo = useMutation({ mutationFn: () => api.undo(id), onSuccess: apply });
  const resign = useMutation({ mutationFn: () => api.resign(id), onSuccess: apply });
  const rematch = useMutation({
    mutationFn: (next: SessionState) =>
      api.createSession({
        username: next.username,
        user_color: next.current_color === "white" ? "black" : "white",
        engine_elo: next.engine_elo,
        mode: next.mode,
        filters: next.filters,
        steer_cp: next.steer_cp,
        remember: next.remember,
        gap_mode: next.gap_mode,
      }),
    onSuccess: (created) => navigate(`/g/${created.id}`),
  });

  useEffect(() => {
    setSelected(null);
    setPromotion(null);
  }, [s?.ply]);

  const total = s?.moves.length ?? 0;
  const shownPly = viewPly == null ? total : Math.min(Math.max(viewPly, 0), total);
  const viewing = shownPly < total;

  // Arrow keys step through the game; Home/End jump to the start and back to live.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement | null)?.tagName;
      if (tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA") return;
      if (e.key === "ArrowLeft") setViewPly(Math.max(0, shownPly - 1));
      else if (e.key === "ArrowRight") setViewPly(shownPly + 1 >= total ? null : shownPly + 1);
      else if (e.key === "Home") setViewPly(0);
      else if (e.key === "End") setViewPly(null);
      else return;
      setSelected(null);
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [shownPly, total]);

  const livePosition = useMemo(() => (s ? fenToPosition(s.fen) : {}), [s?.fen]);
  const viewedPosition = useMemo(
    () => (s && viewing ? positionAtPly(s.moves, shownPly) : livePosition),
    [s?.moves, shownPly, viewing, livePosition],
  );

  if (session.isLoading) {
    return (
      <p className="muted">
        <LoaderCircle className="spin" size={16} aria-hidden /> Loading game…
      </p>
    );
  }
  if (session.error || !s) {
    return <p className="error">{(session.error as Error | null)?.message ?? "Game not found"}</p>;
  }

  const playing = s.status === "playing";
  const busy = move.isPending || undo.isPending || resign.isPending;
  const myColor = s.current_color;
  const myTurn = playing && s.turn === myColor && !busy && !viewing;
  const lastSwitch = s.switches[s.switches.length - 1];
  const justSwitched = playing && !!lastSwitch && lastSwitch.ply === s.moves.length;
  const shownMove = shownPly > 0 ? s.moves[shownPly - 1] : undefined;
  const lastReply = [...s.moves].reverse().find((m) => m.source !== "player");
  const focusMove = viewing ? shownMove : lastReply;
  const mine = (pieceType: string) => pieceType[0] === (myColor === "white" ? "w" : "b");
  const position = optimistic ?? viewedPosition;

  const submitMove = (uci: string) => {
    setOptimistic(applyUci(livePosition, uci));
    setSelected(null);
    setPromotion(null);
    move.mutate(uci);
  };

  const tryMove = (from: string, to: string): boolean => {
    const legal = s.legal_moves.filter((m) => m.startsWith(from + to));
    if (legal.length === 0) return false;
    if (legal.length === 1 && legal[0].length === 4) {
      submitMove(legal[0]);
      return true;
    }
    setPromotion({ from, to });
    setSelected(null);
    return true;
  };

  const squareStyles: Record<string, CSSProperties> = {};
  if (shownMove && !optimistic) {
    squareStyles[shownMove.uci.slice(0, 2)] = { backgroundColor: "var(--sq-last)" };
    squareStyles[shownMove.uci.slice(2, 4)] = { backgroundColor: "var(--sq-last)" };
  }
  if (selected && !viewing) {
    squareStyles[selected] = { backgroundColor: "var(--sq-selected)" };
    for (const m of s.legal_moves) {
      if (!m.startsWith(selected)) continue;
      const target = m.slice(2, 4);
      // A capture gets a ring around the piece so it stays visible; an empty square gets a dot.
      squareStyles[target] = position[target]
        ? {
            background: "radial-gradient(circle, transparent 0, transparent 60%, var(--sq-capture) 62%)",
            boxShadow: "inset 0 0 0 3px var(--sq-capture)",
          }
        : { background: "radial-gradient(circle, var(--sq-target) 22%, transparent 24%)" };
    }
  }

  const orientation = flipped ? (myColor === "white" ? "black" : "white") : myColor;
  const options: ChessboardOptions = {
    id: "yvy",
    position,
    boardOrientation: orientation,
    animationDurationInMs: ANIMATION_MS,
    allowDragging: myTurn,
    canDragPiece: ({ piece }) => myTurn && mine(piece.pieceType),
    onPieceDrop: ({ sourceSquare, targetSquare }) => (myTurn && targetSquare ? tryMove(sourceSquare, targetSquare) : false),
    onSquareClick: ({ square, piece }) => {
      if (!myTurn) return;
      if (selected) {
        if (square === selected) {
          setSelected(null);
          return;
        }
        if (tryMove(selected, square)) return;
      }
      setSelected(piece && mine(piece.pieceType) ? square : null);
    },
    squareStyles,
    darkSquareStyle: { backgroundColor: "#55708c" },
    lightSquareStyle: { backgroundColor: "#e3e9f0" },
    boardStyle: { borderRadius: "6px", overflow: "hidden", boxShadow: "0 18px 50px rgba(0,0,0,.45)" },
  };

  const filterSummary = [
    `Stockfish ${s.engine_elo}`,
    s.mode === "sampled" ? "sampled" : "most frequent",
    s.gap_mode === "switch" ? "switch sides at gaps" : `steer ${steerLabel(s.steer_cp).toLowerCase()}`,
    s.remember ? "remembered" : "not remembered",
    s.filters.time_classes?.map(timeClassLabel).join(", "),
    s.filters.date_from ? `from ${dayLabel(s.filters.date_from)}` : undefined,
    s.filters.date_to ? `to ${dayLabel(s.filters.date_to)}` : undefined,
  ]
    .filter(Boolean)
    .join(" · ");

  const over = statusText(s.status, s.result, s.user_color);
  const goTo = (ply: number) => {
    setSelected(null);
    setViewPly(ply >= total ? null : Math.max(0, ply));
  };

  return (
    <section className="game">
      <div className="board-column">
        <div className="board-wrap">
          <Chessboard options={options} />
          {promotion && (
            <div className="promo" role="dialog" aria-label="Choose a promotion piece">
              {PROMOTIONS.map((p) => (
                <button
                  key={p.code}
                  type="button"
                  className="piece"
                  title={p.label}
                  onClick={() => submitMove(`${promotion.from}${promotion.to}${p.code}`)}
                >
                  {myColor === "white" ? p.white : p.black}
                </button>
              ))}
              <button type="button" className="btn btn-ghost" onClick={() => setPromotion(null)}>
                cancel
              </button>
            </div>
          )}
        </div>

        <div className="replay" aria-label="Replay controls">
          <button type="button" className="btn btn-icon" onClick={() => goTo(0)} disabled={shownPly === 0} title="Start (Home)">
            <ChevronFirst size={16} aria-hidden />
          </button>
          <button type="button" className="btn btn-icon" onClick={() => goTo(shownPly - 1)} disabled={shownPly === 0} title="Previous move (←)">
            <ChevronLeft size={16} aria-hidden />
          </button>
          <button type="button" className="btn btn-icon" onClick={() => goTo(shownPly + 1)} disabled={!viewing} title="Next move (→)">
            <ChevronRight size={16} aria-hidden />
          </button>
          <button type="button" className="btn btn-icon" onClick={() => goTo(total)} disabled={!viewing} title="Latest position (End)">
            <ChevronLast size={16} aria-hidden />
          </button>
          <span className="replay-label">
            {viewing ? (
              <>
                move {shownPly} of {total}
                {shownMove && <strong> · {shownMove.san}</strong>}
              </>
            ) : over ? (
              <strong>{over}</strong>
            ) : (
              <span className="muted">use ← → to step through the game</span>
            )}
          </span>
          {viewing && (
            <button type="button" className="btn btn-ghost" onClick={() => goTo(total)}>
              <Radio size={14} aria-hidden /> Live
            </button>
          )}
        </div>
      </div>

      <aside className="panel">
        <div className="card">
          <div className="vs">
            <span>
              You play <strong>{myColor}</strong>
              {myColor !== s.user_color && <span className="muted"> (started as {s.user_color})</span>}
            </span>
            <span className="muted">vs</span>
            <span>
              <strong>{s.username}</strong> <span className="muted">(model)</span>
            </span>
          </div>
          <div className="muted small">{filterSummary}</div>
          <div className="status">
            {playing ? (
              busy ? (
                <>
                  <LoaderCircle className="spin" size={16} aria-hidden /> Your past self is thinking…
                </>
              ) : viewing ? (
                <span className="muted">Viewing an earlier position</span>
              ) : (
                <>Your move</>
              )
            ) : (
              <strong>{over}</strong>
            )}
          </div>
          {move.error && <p className="error">{(move.error as Error).message}</p>}
          {undo.error && <p className="error">{(undo.error as Error).message}</p>}
        </div>

        {justSwitched && (
          <div className="card switch-banner" role="status">
            <ArrowLeftRight size={18} aria-hidden />
            <div>
              <strong>No history as {lastSwitch.color} here.</strong> You take over {lastSwitch.color} and play the position yourself; the model
              answers as {lastSwitch.color === "white" ? "black" : "white"} from your games. {s.remember && "What you play here is remembered."}
            </div>
          </div>
        )}

        {focusMove && (
          <div className="card reply">
            <div className="card-title">
              {viewing ? `Move ${shownPly}` : "Last reply"} · <strong>{focusMove.san}</strong>
            </div>
            <ProvenanceBadge move={focusMove} size="lg" />
            {isHistory(focusMove.detail) && <CandidateBars candidates={focusMove.detail.candidates} chosen={focusMove.uci} />}
            {isEngine(focusMove.detail) && focusMove.detail.reason && <p className="muted small">{focusMove.detail.reason}</p>}
          </div>
        )}

        <div className="card">
          <div className="card-title">{viewing ? "Current position (live)" : "This position"}</div>
          {s.coverage.in_history ? (
            <>
              <p>
                You have faced this position <strong>{s.coverage.games}</strong> {s.coverage.games === 1 ? "time" : "times"} before.
              </p>
              <button type="button" className="btn btn-ghost" onClick={() => setShowHint((v) => !v)}>
                {showHint ? <EyeOff size={14} aria-hidden /> : <Eye size={14} aria-hidden />}
                {showHint ? "Hide" : "Show"} what you played
              </button>
              {showHint && <CandidateBars candidates={s.coverage.candidates} />}
            </>
          ) : (
            <p className="muted">New territory: this position is not in your games.</p>
          )}
          {s.coverage.returning_moves.length > 0 && (
            <>
              <p className="small" style={{ marginBottom: 0 }}>
                Moves after which the model (as {myColor === "white" ? "black" : "white"}) is back in {s.coverage.returning_moves.length === 1 ? "a game" : "games"} you played:
              </p>
              <div className="returning">
                {s.coverage.returning_moves.slice(0, 8).map((m) => (
                  <span key={m.uci} className="chip" title={`${m.games} ${m.games === 1 ? "game" : "games"} reached the position after ${m.san}`}>
                    <strong>{m.san}</strong>
                    <span className="muted">{m.games}</span>
                  </span>
                ))}
              </div>
            </>
          )}
        </div>

        <div className="card">
          <div className="card-title">Moves</div>
          <MoveList moves={s.moves} selected={shownMove?.ply ?? null} onSelect={goTo} switches={s.switches} />
        </div>

        <div className="controls">
          <button
            type="button"
            className="btn"
            onClick={() => undo.mutate()}
            disabled={busy || s.status === "resigned" || !s.moves.some((m) => m.source === "player")}
          >
            <RotateCcw size={14} aria-hidden /> Undo
          </button>
          <button type="button" className="btn" onClick={() => resign.mutate()} disabled={!playing || busy}>
            <Flag size={14} aria-hidden /> Resign
          </button>
          <button type="button" className="btn" onClick={() => setFlipped((f) => !f)}>
            <Repeat size={14} aria-hidden /> Flip
          </button>
          <button
            type="button"
            className="btn"
            onClick={async () => download(`yvy-${s.id.slice(0, 8)}.pgn`, await api.pgn(s.id), "application/x-chess-pgn")}
          >
            <Download size={14} aria-hidden /> PGN
          </button>
          {!playing && (
            <button type="button" className="btn btn-primary" onClick={() => rematch.mutate(s)} disabled={rematch.isPending}>
              <Swords size={14} aria-hidden /> Rematch as {myColor === "white" ? "black" : "white"}
            </button>
          )}
          <Link className={`btn${playing ? " btn-primary" : ""}`} to={`/p/${encodeURIComponent(s.username)}`}>
            <Plus size={14} aria-hidden /> New game
          </Link>
        </div>
      </aside>
    </section>
  );
}
