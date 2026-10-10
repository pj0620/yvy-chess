use std::collections::{BTreeMap, HashMap};

use serde::Serialize;
use yvy_core::{decode_uci_string, PlayerMove};

use crate::types::{Filters, GameMeta, StoredGame};

#[derive(Debug, Clone, Copy)]
struct Entry {
    key: u64,
    game: u32,
    mv: u16,
}

#[derive(Debug, Clone, Copy)]
struct SetupEntry {
    key: u64,
    game: u32,
}

/// One move the player made from a position: how often, in which games (most recent first,
/// at most ten) and when last.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawCandidate {
    pub mv: u16,
    pub count: i64,
    pub games: Vec<u32>,
    pub last_played: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub total: i64,
    pub white: i64,
    pub black: i64,
    /// Decisions indexed: positions in which it was the player's move.
    pub positions: i64,
    pub by_time_class: BTreeMap<String, i64>,
    pub first_game: Option<i64>,
    pub last_game: Option<i64>,
}

/// Every decision one player made, searchable by position hash and by own-piece setup.
#[derive(Debug, Default)]
pub struct PlayerIndex {
    games: Vec<GameMeta>,
    by_id: HashMap<String, u32>,
    base: Vec<Entry>,
    base_setups: Vec<SetupEntry>,
    sorted: bool,
    /// Games played in the app: replaced wholesale whenever the game changes.
    live: HashMap<u32, Vec<PlayerMove>>,
    live_keys: HashMap<u64, Vec<(u32, u16)>>,
    live_setups: HashMap<u64, Vec<u32>>,
}

impl PlayerIndex {
    pub fn contains(&self, id: &str) -> bool {
        self.by_id.contains_key(id)
    }

    pub fn game(&self, idx: u32) -> &GameMeta {
        &self.games[idx as usize]
    }

    pub fn game_by_id(&self, id: &str) -> Option<&GameMeta> {
        self.by_id.get(id).map(|&i| self.game(i))
    }

    pub fn live_decisions(&self, id: &str) -> Option<&[PlayerMove]> {
        self.by_id.get(id).and_then(|i| self.live.get(i)).map(|v| v.as_slice())
    }

    /// Adds an imported game. Returns false (and changes nothing) when the game is known.
    pub fn add_game(&mut self, game: &StoredGame) -> bool {
        if self.by_id.contains_key(&game.meta.id) {
            return false;
        }
        let idx = self.games.len() as u32;
        self.games.push(game.meta.clone());
        self.by_id.insert(game.meta.id.clone(), idx);
        for d in &game.decisions {
            self.base.push(Entry { key: d.key, game: idx, mv: d.mv });
            self.base_setups.push(SetupEntry { key: d.setup, game: idx });
        }
        self.sorted = false;
        true
    }

    /// Inserts or replaces a game played in the app, together with the player's decisions in it.
    pub fn set_live_game(&mut self, meta: GameMeta, decisions: Vec<PlayerMove>) {
        let idx = match self.by_id.get(&meta.id) {
            Some(&i) => {
                self.games[i as usize] = meta;
                i
            }
            None => {
                let i = self.games.len() as u32;
                self.by_id.insert(meta.id.clone(), i);
                self.games.push(meta);
                i
            }
        };
        self.live.insert(idx, decisions);
        self.live_keys.clear();
        self.live_setups.clear();
        for (&game, decisions) in &self.live {
            for d in decisions {
                self.live_keys.entry(d.key).or_default().push((game, d.mv));
                self.live_setups.entry(d.setup).or_default().push(game);
            }
        }
    }

    /// Sorts the imported decisions so lookups can binary-search. Cheap when already sorted.
    pub fn prepare(&mut self) {
        if !self.sorted {
            self.base.sort_unstable_by_key(|e| e.key);
            self.base_setups.sort_unstable_by_key(|e| e.key);
            self.sorted = true;
        }
    }

    fn entries(&self, key: u64) -> &[Entry] {
        if self.sorted {
            let lo = self.base.partition_point(|e| e.key < key);
            let hi = self.base.partition_point(|e| e.key <= key);
            &self.base[lo..hi]
        } else {
            &[]
        }
    }

    fn setup_entries(&self, key: u64) -> &[SetupEntry] {
        if self.sorted {
            let lo = self.base_setups.partition_point(|e| e.key < key);
            let hi = self.base_setups.partition_point(|e| e.key <= key);
            &self.base_setups[lo..hi]
        } else {
            &[]
        }
    }

    /// Visits (game, move) for every decision at `key`, including unsorted data via a scan.
    fn for_each_at(&self, key: u64, mut f: impl FnMut(u32, u16)) {
        if self.sorted {
            for e in self.entries(key) {
                f(e.game, e.mv);
            }
        } else {
            for e in self.base.iter().filter(|e| e.key == key) {
                f(e.game, e.mv);
            }
        }
        if let Some(live) = self.live_keys.get(&key) {
            for &(g, mv) in live {
                f(g, mv);
            }
        }
    }

    /// Number of times (under the filters) the player was to move in the position `key`.
    pub fn count(&self, key: u64, filters: &Filters) -> i64 {
        let mut n = 0;
        self.for_each_at(key, |g, _| {
            if filters.allows(self.game(g)) {
                n += 1;
            }
        });
        n
    }

    /// Number of decisions (under the filters) after which the player's own pieces stood as in
    /// the setup `key`.
    pub fn count_setup(&self, key: u64, filters: &Filters) -> i64 {
        let mut n = 0;
        let mut visit = |g: u32| {
            if filters.allows(self.game(g)) {
                n += 1;
            }
        };
        if self.sorted {
            self.setup_entries(key).iter().for_each(|e| visit(e.game));
        } else {
            self.base_setups.iter().filter(|e| e.key == key).for_each(|e| visit(e.game));
        }
        if let Some(live) = self.live_setups.get(&key) {
            live.iter().for_each(|&g| visit(g));
        }
        n
    }

    /// The moves the player made from `key`, most frequent first (ties by UCI).
    pub fn candidates(&self, key: u64, filters: &Filters) -> Vec<RawCandidate> {
        let mut groups: Vec<(u16, Vec<u32>)> = Vec::new();
        self.for_each_at(key, |g, mv| {
            if !filters.allows(self.game(g)) {
                return;
            }
            match groups.iter_mut().find(|(m, _)| *m == mv) {
                Some((_, games)) => games.push(g),
                None => groups.push((mv, vec![g])),
            }
        });
        let mut out: Vec<RawCandidate> = groups
            .into_iter()
            .map(|(mv, mut games)| {
                let count = games.len() as i64;
                games.sort_by_key(|&g| std::cmp::Reverse(self.game(g).end_time));
                games.dedup();
                let last_played = games.first().map(|&g| self.game(g).end_time).unwrap_or(0);
                games.truncate(10);
                RawCandidate { mv, count, games, last_played }
            })
            .collect();
        out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| decode_uci_string(a.mv).cmp(&decode_uci_string(b.mv))));
        out
    }

    pub fn summary(&self) -> Summary {
        let mut s = Summary::default();
        for g in &self.games {
            s.total += 1;
            match g.color.as_str() {
                "white" => s.white += 1,
                "black" => s.black += 1,
                _ => {}
            }
            *s.by_time_class.entry(g.time_class.clone()).or_default() += 1;
            s.first_game = Some(s.first_game.map_or(g.end_time, |t| t.min(g.end_time)));
            s.last_game = Some(s.last_game.map_or(g.end_time, |t| t.max(g.end_time)));
        }
        s.positions = (self.base.len() + self.live.values().map(Vec::len).sum::<usize>()) as i64;
        s
    }
}
