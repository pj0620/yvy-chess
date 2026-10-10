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
    </div>
  );
}
