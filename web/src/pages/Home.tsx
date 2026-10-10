import { useState, type FormEvent } from "react";
import { useNavigate } from "react-router";
import { useMutation } from "@tanstack/react-query";
import { LoaderCircle, Play } from "lucide-react";
import { api, ApiError } from "../api";
import { Badge } from "../components/Badge";

function rememberedUsername(): string {
  try {
    return localStorage.getItem("yvy:last-username") ?? "";
  } catch {
    return "";
  }
}

export function Home() {
  const navigate = useNavigate();
  const [username, setUsername] = useState(rememberedUsername);

  const start = useMutation({
    mutationFn: async (name: string) => {
      const normalized = name.trim().toLowerCase();
      try {
        const summary = await api.player(normalized);
        if (summary.imported) return normalized;
      } catch (e) {
        if (!(e instanceof ApiError && e.status === 404)) throw e;
      }
      await api.startImport(normalized);
      return normalized;
    },
    onSuccess: (name) => {
      try {
        localStorage.setItem("yvy:last-username", name);
      } catch {
        // storage can be unavailable; nothing to do
      }
      navigate(`/p/${encodeURIComponent(name)}`);
    },
  });

  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (username.trim()) start.mutate(username);
  };

  return (
    <section className="hero">
      <h1>
        Play against <em>yourself</em>.
      </h1>
      <p className="lede">
        Enter your chess.com username. Every game you have played gets replayed and every decision you made gets indexed.
        Then you sit down across the board from that player. When the position is one you have faced before, your opponent
        plays what you played. When it is not, Stockfish fills in at a strength you choose, and the board always tells you
        which is which.
      </p>
      <form onSubmit={submit} className="username-form">
        <input
          className="input input-lg"
          placeholder="chess.com username"
          value={username}
          onChange={(e) => setUsername(e.target.value)}
          autoFocus
          spellCheck={false}
          autoCapitalize="none"
          autoCorrect="off"
        />
        <button type="submit" className="btn btn-primary btn-lg" disabled={start.isPending || !username.trim()}>
          {start.isPending ? <LoaderCircle className="spin" size={18} aria-hidden /> : <Play size={18} aria-hidden />}
          Load my games
        </button>
      </form>
      {start.error && <p className="error">{(start.error as Error).message}</p>}
      <ul className="how">
        <li>
          <Badge kind="history" />
          <span>a move you actually played from this exact position, picked in proportion to how often you played it</span>
        </li>
        <li>
          <Badge kind="stockfish" />
          <span>a position missing from your history, filled by Stockfish at the Elo you pick</span>
        </li>
      </ul>
    </section>
  );
}
