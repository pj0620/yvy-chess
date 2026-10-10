import { Link, Route, Routes } from "react-router";
import { Home } from "./pages/Home";
import { Player } from "./pages/Player";
import { Game } from "./pages/Game";

export function App() {
  return (
    <div className="shell">
      <header className="topbar">
        <Link to="/" className="brand">
          <span className="brand-you">You</span>
          <span className="brand-v">v</span>
          <span className="brand-you">You</span>
          <span className="brand-chess">Chess</span>
        </Link>
        <span className="topbar-tagline">play the player your games say you are</span>
      </header>
      <main className="content">
        <Routes>
          <Route path="/" element={<Home />} />
          <Route path="/p/:username" element={<Player />} />
          <Route path="/g/:id" element={<Game />} />
          <Route path="*" element={<p className="muted">Nothing here. <Link to="/">Go home.</Link></p>} />
        </Routes>
      </main>
      <footer className="footer">
        Everything runs in your browser: the logic is Rust compiled to WebAssembly, and your games stay on this device. Engine:{" "}
        <a href="https://github.com/official-stockfish/Stockfish" target="_blank" rel="noreferrer">Stockfish</a> via{" "}
        <a href="https://github.com/nmrugg/stockfish.js" target="_blank" rel="noreferrer">stockfish.js</a> (GPLv3) ·{" "}
        <a href="https://github.com/pj0620/yvy-chess" target="_blank" rel="noreferrer">source</a>
      </footer>
    </div>
  );
}
