import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import { Link, useNavigate, useParams } from "react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertTriangle, Download, ExternalLink, LoaderCircle, Play, RefreshCw, Upload } from "lucide-react";
import { dayLabel, outcomeLabel } from "../lib/format";
import { api, ApiError, download, subscribeImport, timeClassLabel, APP_TIME_CLASS, type Color, type GapMode, type Mode, type Progress } from "../api";
import { ImportProgress } from "../components/ImportProgress";
import { defaultElo, GAP_MODES, STEER_LEVELS } from "../lib/format";

const CLASS_ORDER = ["bullet", "blitz", "rapid", "daily", APP_TIME_CLASS];

export function Player() {
  const { username = "" } = useParams();
  const navigate = useNavigate();
  const qc = useQueryClient();

  const summary = useQuery({
    queryKey: ["player", username],
    queryFn: () => api.player(username),
    retry: false,
  });
  const engine = useQuery({ queryKey: ["engine"], queryFn: api.engine, staleTime: 60_000 });
  const sessions = useQuery({ queryKey: ["sessions", username], queryFn: () => api.sessions(username), enabled: !!summary.data?.imported });

  const [live, setLive] = useState<Progress | null>(null);
  const progress = live ?? summary.data?.import ?? null;
  const importStatus = progress?.status;

  const startImport = useMutation({
    mutationFn: () => api.startImport(username),
    onSuccess: (r) => setLive(r.progress),
  });

  // Nobody has imported this account yet: start it once, automatically.
  const autoStarted = useRef(false);
  useEffect(() => {
    const notFound = summary.error instanceof ApiError && summary.error.status === 404;
    if (notFound && !autoStarted.current && !startImport.isPending) {
      autoStarted.current = true;
      startImport.mutate();
    }
  }, [summary.error, startImport]);

  // Follow a running import and refresh the summary when it ends.
  useEffect(() => {
    if (importStatus !== "running") return;
    return subscribeImport(username, (p) => {
      setLive(p);
      if (p.status !== "running") {
        qc.invalidateQueries({ queryKey: ["player", username] });
        qc.invalidateQueries({ queryKey: ["sessions", username] });
      }
    });
  }, [username, importStatus, qc]);

  const [backupNote, setBackupNote] = useState<string | null>(null);
  const restore = useMutation({
    mutationFn: async (file: File) => api.importBackup(await file.text()),
    onSuccess: (r) => {
      setBackupNote(`Restored ${r.added} game${r.added === 1 ? "" : "s"}.`);
      qc.invalidateQueries({ queryKey: ["player", username] });
      qc.invalidateQueries({ queryKey: ["sessions", username] });
    },
  });
  const backup = async () => {
    const b = await api.exportBackup(username);
    download(`yvy-backup-${username}-${dayLabel(Date.now())}.json`, JSON.stringify(b, null, 1), "application/json");
    setBackupNote(`Saved ${b.sessions.length} game${b.sessions.length === 1 ? "" : "s"} played here.`);
  };

  const data = summary.data;
  const games = data?.games;
  const ratings = data?.ratings;
  const availableClasses = useMemo(
    () => CLASS_ORDER.filter((c) => (games?.by_time_class[c] ?? 0) > 0).concat(Object.keys(games?.by_time_class ?? {}).filter((c) => !CLASS_ORDER.includes(c))),
    [games],
  );

  // New game form
  const [color, setColor] = useState<Color | "random">("random");
  const [classes, setClasses] = useState<string[]>([]);
  const [mode, setMode] = useState<Mode>("sampled");
  const [eloTouched, setEloTouched] = useState(false);
  const [elo, setElo] = useState(1500);
  const [dateFrom, setDateFrom] = useState("");
  const [dateTo, setDateTo] = useState("");
  const [steer, setSteer] = useState(100);
  const [remember, setRemember] = useState(true);
  const [gapMode, setGapMode] = useState<GapMode>("stockfish");

  const suggestedElo = defaultElo(ratings, classes);
  useEffect(() => {
    if (!eloTouched) setElo(suggestedElo);
  }, [suggestedElo, eloTouched]);

  const create = useMutation({
    mutationFn: () =>
      api.createSession({
        username,
        user_color: color === "random" ? (Math.random() < 0.5 ? "white" : "black") : color,
        engine_elo: elo,
        mode,
        steer_cp: steer,
        remember,
        gap_mode: gapMode,
        filters: {
          time_classes: classes.length > 0 ? classes : undefined,
          date_from: dateFrom ? new Date(`${dateFrom}T00:00:00`).getTime() : undefined,
          date_to: dateTo ? new Date(`${dateTo}T23:59:59.999`).getTime() : undefined,
        },
      }),
    onSuccess: (s) => navigate(`/g/${s.id}`),
  });

  const toggleClass = (c: string) =>
    setClasses((prev) => (prev.includes(c) ? prev.filter((x) => x !== c) : [...prev, c]));

  const submit = (e: FormEvent) => {
    e.preventDefault();
    create.mutate();
  };

  if (summary.isLoading) {
    return (
      <p className="muted">
        <LoaderCircle className="spin" size={16} aria-hidden /> Looking up {username}…
      </p>
    );
  }
  if (summary.error && !(summary.error instanceof ApiError && summary.error.status === 404)) {
    return <p className="error">{(summary.error as Error).message}</p>;
  }

  const imported = !!data?.imported && (games?.total ?? 0) > 0;
  const running = importStatus === "running";

  return (
    <section className="player">
      <header className="player-head">
        <div>
          <h1>{username}</h1>
          <a className="muted link" href={`https://www.chess.com/member/${username}`} target="_blank" rel="noreferrer">
            chess.com profile <ExternalLink size={12} aria-hidden />
          </a>
        </div>
        <div className="ratings">
          {ratings &&
            CLASS_ORDER.filter((c) => ratings[c]).map((c) => (
              <span key={c} className="chip chip-static">
                {c} <strong>{ratings[c]}</strong>
              </span>
            ))}
        </div>
      </header>

      {startImport.error && <p className="error">{(startImport.error as Error).message}</p>}
      {progress && <ImportProgress progress={progress} />}

      {engine.data && !engine.data.available && (
        <div className="card warn">
          <AlertTriangle size={18} aria-hidden />
          <div>
            <strong>Stockfish could not start in this browser</strong> ({engine.data.error}). Positions outside your history will
            get a random legal move, or you can pick "Switch sides" below.
          </div>
        </div>
      )}

      {imported && games && (
        <>
          <div className="stat-grid">
            <div className="card stat">
              <dt>games indexed</dt>
              <dd>{games.total.toLocaleString()}</dd>
              <small>
                {games.white.toLocaleString()} as White · {games.black.toLocaleString()} as Black
              </small>
            </div>
            <div className="card stat">
              <dt>decisions you made</dt>
              <dd>{games.positions.toLocaleString()}</dd>
              <small>positions where it was your move</small>
            </div>
            <div className="card stat">
              <dt>time classes</dt>
              <dd className="chips">
                {Object.entries(games.by_time_class).map(([c, n]) => (
                  <span key={c} className="chip chip-static">
                    {timeClassLabel(c)} {n.toLocaleString()}
                  </span>
                ))}
              </dd>
              <small>
                {dayLabel(games.first_game)} → {dayLabel(games.last_game)}
              </small>
            </div>
            <div className="card stat">
              <dt>last import</dt>
              <dd className="small">{data.last_import_at ? new Date(data.last_import_at).toLocaleString() : "running…"}</dd>
              <button type="button" className="btn btn-ghost" onClick={() => startImport.mutate()} disabled={running || startImport.isPending}>
                <RefreshCw size={14} aria-hidden /> Re-sync new games
              </button>
            </div>
          </div>

          <form className="card newgame" onSubmit={submit}>
            <h2>New game against {username}</h2>
            <div className="field">
              <label>You play</label>
              <div className="segmented">
                {(["white", "black", "random"] as const).map((c) => (
                  <button type="button" key={c} className={color === c ? "active" : ""} onClick={() => setColor(c)}>
                    {c}
                  </button>
                ))}
              </div>
              {(() => {
                const missing = (["white", "black"] as const).filter((side) => (games[side] ?? 0) === 0);
                const opponentSide = color === "white" ? "black" : color === "black" ? "white" : null;
                const affected = opponentSide ? missing.filter((m) => m === opponentSide) : missing;
                if (affected.length === 0) return null;
                return (
                  <p className="note">
                    You have no games as {affected.join(" or ")}, so when you play the other side your opponent will be pure Stockfish.
                  </p>
                );
              })()}
            </div>
            <div className="field">
              <label>
                Use games from <span className="muted">(none selected = all)</span>
              </label>
              <div className="chips">
                {availableClasses.map((c) => (
                  <button type="button" key={c} className={`chip${classes.includes(c) ? " chip-on" : ""}`} onClick={() => toggleClass(c)}>
                    {timeClassLabel(c)} <span className="muted">{games.by_time_class[c]?.toLocaleString()}</span>
                  </button>
                ))}
              </div>
            </div>
            <div className="field-row">
              <div className="field">
                <label htmlFor="from">From</label>
                <input id="from" type="date" className="input" value={dateFrom} onChange={(e) => setDateFrom(e.target.value)} />
              </div>
              <div className="field">
                <label htmlFor="to">To</label>
                <input id="to" type="date" className="input" value={dateTo} onChange={(e) => setDateTo(e.target.value)} />
              </div>
            </div>
            <div className="field">
              <label>When the model reaches a position you have never faced</label>
              <div className="segmented">
                {GAP_MODES.map((g) => (
                  <button type="button" key={g.value} className={gapMode === g.value ? "active" : ""} onClick={() => setGapMode(g.value)} title={g.hint}>
                    {g.label}
                  </button>
                ))}
              </div>
              <small className="muted">{GAP_MODES.find((g) => g.value === gapMode)?.hint}</small>
            </div>
            <div className="field-row" style={gapMode === "switch" ? { display: "none" } : undefined}>
              <div className="field">
                <label htmlFor="elo">
                  Stockfish strength (Elo){" "}
                  {!eloTouched && <span className="muted">suggested from your rating</span>}
                </label>
                <input
                  id="elo"
                  type="number"
                  className="input"
                  min={600}
                  max={engine.data?.elo_max ?? 3190}
                  value={elo}
                  onChange={(e) => {
                    setEloTouched(true);
                    setElo(Number(e.target.value));
                  }}
                />
                {engine.data?.available && elo < (engine.data.elo_min ?? 1320) && (
                  <small className="muted">below Stockfish's Elo range ({engine.data.elo_min}); its skill level is used instead</small>
                )}
              </div>
              <div className="field">
                <label htmlFor="steer">
                  When you leave your games, steer back <span className="muted">(what Stockfish may give up)</span>
                </label>
                <select id="steer" className="input" value={steer} onChange={(e) => setSteer(Number(e.target.value))}>
                  {STEER_LEVELS.map((l) => (
                    <option key={l.value} value={l.value}>
                      {l.label}: {l.hint}
                    </option>
                  ))}
                </select>
              </div>
            </div>
            <div className="field-row">
              <div className="field">
                <label htmlFor="mode">When you played several moves here</label>
                <select id="mode" className="input" value={mode} onChange={(e) => setMode(e.target.value as Mode)}>
                  <option value="sampled">pick one at random, weighted by how often (sampled)</option>
                  <option value="most_frequent">always play the most frequent one</option>
                </select>
              </div>
            </div>
            <label className="check">
              <input type="checkbox" checked={remember} onChange={(e) => setRemember(e.target.checked)} />
              <span>
                Remember my moves from this game <span className="muted">(they join your history as "You v You" games, so the model learns how you react)</span>
              </span>
            </label>
            {create.error && <p className="error">{(create.error as Error).message}</p>}
            <button type="submit" className="btn btn-primary btn-lg" disabled={create.isPending}>
              {create.isPending ? <LoaderCircle className="spin" size={18} aria-hidden /> : <Play size={18} aria-hidden />}
              Play
            </button>
          </form>
        </>
      )}

      {imported && sessions.data && sessions.data.length > 0 && (
        <div className="card" style={{ marginTop: 12 }}>
          <h2 style={{ marginTop: 0 }}>Games played here</h2>
          <ul className="sessions">
            {sessions.data.map((g) => {
              const outcome = outcomeLabel(g.status, g.result);
              const cls = g.result === "win" ? "outcome-win" : g.result === "loss" ? "outcome-loss" : "";
              return (
                <li key={g.id} className="session-row">
                  <span className="muted">{new Date(g.updated_at).toLocaleDateString()}</span>
                  <span>as {g.user_color}</span>
                  <span className={cls}>{outcome}</span>
                  <span className="muted">{Math.ceil(g.plies / 2)} moves{g.remember ? "" : " · not remembered"}</span>
                  <Link className="btn btn-ghost" to={`/g/${g.id}`}>
                    {g.status === "playing" ? "Continue" : "Replay"}
                  </Link>
                </li>
              );
            })}
          </ul>
        </div>
      )}

      {imported && (
        <div className="card backup" style={{ marginTop: 12 }}>
          <div>
            <h2 style={{ margin: 0 }}>Your data lives in this browser</h2>
            <p className="muted small" style={{ margin: "4px 0 0" }}>
              Nothing is uploaded anywhere. Games imported from chess.com can always be fetched again, but games played here
              exist only in this browser, so save a backup to move them to another device or keep them safe.
            </p>
          </div>
          <div className="controls">
            <button type="button" className="btn" onClick={backup}>
              <Download size={14} aria-hidden /> Save backup
            </button>
            <label className="btn">
              <Upload size={14} aria-hidden /> Restore backup
              <input
                type="file"
                accept="application/json,.json"
                hidden
                onChange={(e) => {
                  const file = e.target.files?.[0];
                  if (file) restore.mutate(file);
                  e.target.value = "";
                }}
              />
            </label>
          </div>
          {backupNote && <p className="small">{backupNote}</p>}
          {restore.error && <p className="error">{(restore.error as Error).message}</p>}
        </div>
      )}

      {!imported && !running && importStatus === "done" && (
        <p className="muted">No standard chess games were found for this account.</p>
      )}
      <p className="muted">
        <Link to="/">Use another account</Link>
      </p>
    </section>
  );
}
