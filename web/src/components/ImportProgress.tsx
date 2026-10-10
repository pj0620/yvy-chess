import { LoaderCircle } from "lucide-react";
import type { Progress } from "../api";

export function ImportProgress({ progress }: { progress: Progress }) {
  const pct = progress.archives_total > 0 ? Math.round((100 * progress.archives_done) / progress.archives_total) : 0;
  const running = progress.status === "running";
  return (
    <div className={`card import-card import-${progress.status}`}>
      <div className="import-head">
        {running && <LoaderCircle className="spin" size={18} aria-hidden />}
        <strong>
          {running && `Importing ${progress.username}: ${progress.phase}`}
          {progress.status === "done" && `Import of ${progress.username} finished`}
          {progress.status === "failed" && `Import of ${progress.username} failed`}
        </strong>
        <span className="muted">
          {progress.archives_done}/{progress.archives_total} months
        </span>
      </div>
      <div className="progress" role="progressbar" aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100}>
        <div className="progress-fill" style={{ width: `${pct}%` }} />
      </div>
      <dl className="stats-inline">
        <div>
          <dt>games seen</dt>
          <dd>{progress.games_seen.toLocaleString()}</dd>
        </div>
        <div>
          <dt>new</dt>
          <dd>{progress.games_new.toLocaleString()}</dd>
        </div>
        <div>
          <dt>skipped</dt>
          <dd>{progress.games_skipped.toLocaleString()}</dd>
        </div>
        <div>
          <dt>decisions indexed</dt>
          <dd>{progress.positions_new.toLocaleString()}</dd>
        </div>
      </dl>
      {progress.message && <p className="error">{progress.message}</p>}
      {progress.errors.length > 0 && (
        <details>
          <summary>{progress.errors.length} game(s) could not be indexed</summary>
          <ul className="error-list">
            {progress.errors.map((e, i) => (
              <li key={i}>{e}</li>
            ))}
          </ul>
        </details>
      )}
    </div>
  );
}
