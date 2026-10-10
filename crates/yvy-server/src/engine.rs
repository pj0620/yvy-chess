//! A small UCI driver around a Stockfish process. One long-lived engine is shared behind a
//! mutex and respawned if it ever dies.

use std::{process::Stdio, time::Duration};

use anyhow::{anyhow, bail, Context};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
};

#[derive(Debug, Clone, serde::Serialize)]
pub struct EngineInfo {
    pub name: String,
    pub path: String,
    pub elo_min: u32,
    pub elo_max: u32,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct EngineReply {
    pub uci: String,
    /// The Elo the engine was limited to, after clamping to its supported range.
    pub elo: u32,
    /// Set when the requested Elo was below the engine's range and Skill Level was used instead.
    pub skill_level: Option<u32>,
    pub depth: Option<u32>,
    pub score_cp: Option<i32>,
    pub score_mate: Option<i32>,
    pub movetime_ms: u64,
}

/// One principal variation from a multi-line search, identified by its first move.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Line {
    pub uci: String,
    pub depth: Option<u32>,
    pub score_cp: Option<i32>,
    pub score_mate: Option<i32>,
}

struct SearchOutcome {
    bestmove: String,
    lines: Vec<Line>,
}

pub struct Engine {
    _child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    info: EngineInfo,
}

impl Engine {
    pub async fn spawn(path: &str) -> anyhow::Result<Self> {
        let resolved = which::which(path)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| path.to_string());
        let mut child = Command::new(&resolved)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("starting the engine at {resolved}"))?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("engine stdin unavailable"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("engine stdout unavailable"))?;
        let mut engine = Engine {
            _child: child,
            stdin,
            lines: BufReader::new(stdout).lines(),
            info: EngineInfo {
                name: "unknown engine".to_string(),
                path: resolved,
                elo_min: 1320,
                elo_max: 3190,
            },
        };
        engine.send("uci").await?;
        tokio::time::timeout(Duration::from_secs(15), engine.read_uci_banner())
            .await
            .context("engine did not complete the UCI handshake")??;
        engine.send("isready").await?;
        engine.wait_ready().await?;
        Ok(engine)
    }

    pub fn info(&self) -> &EngineInfo {
        &self.info
    }

    async fn read_uci_banner(&mut self) -> anyhow::Result<()> {
        loop {
            let line = self.read_line().await?;
            if let Some(name) = line.strip_prefix("id name ") {
                self.info.name = name.trim().to_string();
            } else if line.starts_with("option name UCI_Elo ") {
                let toks: Vec<&str> = line.split_whitespace().collect();
                for (i, tok) in toks.iter().enumerate() {
                    match (*tok, toks.get(i + 1).and_then(|v| v.parse::<u32>().ok())) {
                        ("min", Some(v)) => self.info.elo_min = v,
                        ("max", Some(v)) => self.info.elo_max = v,
                        _ => {}
                    }
                }
            } else if line.trim() == "uciok" {
                return Ok(());
            }
        }
    }

    async fn send(&mut self, cmd: &str) -> anyhow::Result<()> {
        self.stdin.write_all(cmd.as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn read_line(&mut self) -> anyhow::Result<String> {
        self.lines
            .next_line()
            .await?
            .ok_or_else(|| anyhow!("the engine closed its output"))
    }

    async fn wait_ready(&mut self) -> anyhow::Result<()> {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if self.read_line().await?.trim() == "readyok" {
                    return Ok::<(), anyhow::Error>(());
                }
            }
        })
        .await
        .context("engine did not become ready")?
    }

    async fn apply_strength(&mut self, elo: u32) -> anyhow::Result<(u32, Option<u32>)> {
        let (elo_used, skill) = if elo >= self.info.elo_min {
            (elo.min(self.info.elo_max), None)
        } else {
            // Below the engine's Elo range, approximate with Skill Level 0..19 across 600..elo_min.
            let span = (self.info.elo_min as i64 - 600).max(1);
            let skill = ((elo as i64 - 600).max(0) * 19 / span).clamp(0, 19) as u32;
            (elo, Some(skill))
        };
        match skill {
            None => {
                self.send("setoption name Skill Level value 20").await?;
                self.send("setoption name UCI_LimitStrength value true").await?;
                self.send(&format!("setoption name UCI_Elo value {elo_used}")).await?;
            }
            Some(skill) => {
                self.send("setoption name UCI_LimitStrength value false").await?;
                self.send(&format!("setoption name Skill Level value {skill}")).await?;
            }
        }
        Ok((elo_used, skill))
    }

    /// Runs one search. With `searchmoves`, only those root moves are considered and each gets
    /// its own line (MultiPV), which is how steering candidates are scored at equal depth.
    async fn search(&mut self, fen: &str, movetime_ms: u64, searchmoves: Option<&[String]>) -> anyhow::Result<SearchOutcome> {
        let multipv = searchmoves.map(|m| m.len().max(1)).unwrap_or(1);
        self.send(&format!("setoption name MultiPV value {multipv}")).await?;
        self.send(&format!("position fen {fen}")).await?;
        match searchmoves {
            Some(moves) if !moves.is_empty() => {
                self.send(&format!("go movetime {movetime_ms} searchmoves {}", moves.join(" "))).await?
            }
            _ => self.send(&format!("go movetime {movetime_ms}")).await?,
        }

        let mut lines: Vec<Line> = Vec::new();
        let mut slots: Vec<Option<Line>> = vec![None; multipv];
        let bestmove = tokio::time::timeout(Duration::from_millis(movetime_ms + 10_000), async {
            loop {
                let line = self.read_line().await?;
                let toks: Vec<&str> = line.split_whitespace().collect();
                match toks.first().copied() {
                    Some("info") => {
                        let mut index = 1usize;
                        let mut parsed = Line { uci: String::new(), depth: None, score_cp: None, score_mate: None };
                        for (i, tok) in toks.iter().enumerate() {
                            match *tok {
                                "multipv" => index = toks.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(1),
                                "depth" => parsed.depth = toks.get(i + 1).and_then(|v| v.parse().ok()),
                                "pv" => parsed.uci = toks.get(i + 1).map(|v| v.to_string()).unwrap_or_default(),
                                "score" => match (toks.get(i + 1).copied(), toks.get(i + 2)) {
                                    (Some("cp"), Some(v)) => {
                                        parsed.score_cp = v.parse().ok();
                                        parsed.score_mate = None;
                                    }
                                    (Some("mate"), Some(v)) => {
                                        parsed.score_mate = v.parse().ok();
                                        parsed.score_cp = None;
                                    }
                                    _ => {}
                                },
                                _ => {}
                            }
                        }
                        if !parsed.uci.is_empty() && index >= 1 && index <= slots.len() {
                            slots[index - 1] = Some(parsed);
                        }
                    }
                    Some("bestmove") => {
                        let mv = toks.get(1).copied().unwrap_or("(none)");
                        if mv == "(none)" || mv == "0000" {
                            bail!("the engine has no legal move");
                        }
                        return Ok::<String, anyhow::Error>(mv.to_string());
                    }
                    _ => {}
                }
            }
        })
        .await
        .context("the engine did not answer in time")??;
        lines.extend(slots.into_iter().flatten());
        Ok(SearchOutcome { bestmove, lines })
    }

    /// Asks for the best move from `fen` at roughly the given Elo.
    pub async fn best_move(&mut self, fen: &str, elo: u32, movetime_ms: u64) -> anyhow::Result<EngineReply> {
        let (elo_used, skill) = self.apply_strength(elo).await?;
        let out = self.search(fen, movetime_ms, None).await?;
        let top = out.lines.first();
        Ok(EngineReply {
            uci: out.bestmove,
            elo: elo_used,
            skill_level: skill,
            depth: top.and_then(|l| l.depth),
            score_cp: top.and_then(|l| l.score_cp),
            score_mate: top.and_then(|l| l.score_mate),
            movetime_ms,
        })
    }

    /// Scores each of `moves` from `fen` with the same search budget, at the configured strength.
    pub async fn analyse_moves(&mut self, fen: &str, elo: u32, movetime_ms: u64, moves: &[String]) -> anyhow::Result<Vec<Line>> {
        if moves.is_empty() {
            return Ok(Vec::new());
        }
        self.apply_strength(elo).await?;
        Ok(self.search(fen, movetime_ms, Some(moves)).await?.lines)
    }
}

/// Lazily spawned, shared engine. Any failure drops the process so the next call respawns it.
pub struct EngineHandle {
    path: String,
    movetime_ms: u64,
    inner: Mutex<Option<Engine>>,
}

impl EngineHandle {
    pub fn new(path: String, movetime_ms: u64) -> Self {
        EngineHandle {
            path,
            movetime_ms,
            inner: Mutex::new(None),
        }
    }

    pub fn movetime_ms(&self) -> u64 {
        self.movetime_ms
    }

    async fn ensure<'a>(&self, guard: &'a mut Option<Engine>) -> anyhow::Result<&'a mut Engine> {
        if guard.is_none() {
            *guard = Some(Engine::spawn(&self.path).await?);
        }
        Ok(guard.as_mut().expect("engine just spawned"))
    }

    pub async fn info(&self) -> anyhow::Result<EngineInfo> {
        let mut guard = self.inner.lock().await;
        Ok(self.ensure(&mut guard).await?.info().clone())
    }

    pub async fn best_move(&self, fen: &str, elo: u32) -> anyhow::Result<EngineReply> {
        let mut guard = self.inner.lock().await;
        let engine = self.ensure(&mut guard).await?;
        match engine.best_move(fen, elo, self.movetime_ms).await {
            Ok(reply) => Ok(reply),
            Err(e) => {
                *guard = None;
                Err(e)
            }
        }
    }

    pub async fn analyse_moves(&self, fen: &str, elo: u32, moves: &[String]) -> anyhow::Result<Vec<Line>> {
        let mut guard = self.inner.lock().await;
        let engine = self.ensure(&mut guard).await?;
        match engine.analyse_moves(fen, elo, self.movetime_ms, moves).await {
            Ok(lines) => Ok(lines),
            Err(e) => {
                *guard = None;
                Err(e)
            }
        }
    }
}
