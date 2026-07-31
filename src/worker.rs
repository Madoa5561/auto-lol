use crate::config::{Settings, champion_icon_path};
use crate::core::{Champion, ChampionCatalog, decide_ban, decide_pick};
use crate::lcu::LcuClient;
use std::fs;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub struct SharedState {
    pub settings: Mutex<Settings>,
    pub status: Mutex<String>,
    pub champions: Mutex<Vec<Champion>>,
    pub icons_ready: AtomicUsize,
    pub stop: AtomicBool,
}

impl SharedState {
    pub fn new(settings: Settings) -> Self {
        Self {
            settings: Mutex::new(settings),
            status: Mutex::new("Leagueクライアントを確認しています…".into()),
            champions: Mutex::new(Vec::new()),
            icons_ready: AtomicUsize::new(0),
            stop: AtomicBool::new(false),
        }
    }

    pub fn set_status(&self, status: impl Into<String>) {
        if let Ok(mut current) = self.status.lock() {
            *current = status.into();
        }
    }
}

pub fn spawn(shared: Arc<SharedState>) -> thread::JoinHandle<()> {
    thread::spawn(move || run(shared))
}

pub fn spawn_icon_cache(shared: Arc<SharedState>) -> thread::JoinHandle<()> {
    thread::spawn(move || cache_champion_icons(shared))
}

fn run(shared: Arc<SharedState>) {
    while !shared.stop.load(Ordering::Relaxed) {
        match LcuClient::discover() {
            Ok(client) => monitor_client(&shared, &client),
            Err(message) => {
                shared.set_status(message);
                sleep_interruptibly(&shared, Duration::from_secs(2));
            }
        }
    }
}

fn monitor_client(shared: &SharedState, client: &LcuClient) {
    let mut catalog: Option<ChampionCatalog> = None;
    let mut last_action: Option<(i64, i64, bool)> = None;

    loop {
        if shared.stop.load(Ordering::Relaxed) {
            return;
        }
        let settings = shared
            .settings
            .lock()
            .map(|settings| settings.clone())
            .unwrap_or_default();
        let phase = match client.phase() {
            Ok(phase) => phase,
            Err(_) => return,
        };

        if settings.auto_accept && phase == "ReadyCheck" {
            if client.accept_ready_check().unwrap_or(false) {
                shared.set_status("レディーチェックを承認しました");
            } else {
                shared.set_status("レディーチェックの承認を再試行中");
            }
        } else if (settings.auto_pick || settings.auto_ban) && phase == "ChampSelect" {
            if catalog.is_none() {
                let cached = shared
                    .champions
                    .lock()
                    .map(|champions| champions.clone())
                    .unwrap_or_default();
                if cached.is_empty() {
                    match client.champion_catalog() {
                        Ok(loaded) => catalog = Some(loaded),
                        Err(error) => {
                            shared.set_status(error);
                            sleep_interruptibly(shared, Duration::from_millis(700));
                            continue;
                        }
                    }
                } else {
                    catalog = Some(ChampionCatalog::new(cached));
                }
            }
            match client.champ_select_session() {
                Ok(Some(session)) => {
                    let ban_decision = settings
                        .auto_ban
                        .then(|| decide_ban(&settings, catalog.as_ref().unwrap(), &session))
                        .flatten();
                    let pick_decision = settings
                        .auto_pick
                        .then(|| decide_pick(&settings, catalog.as_ref().unwrap(), &session))
                        .flatten();
                    if let Some((decision, is_ban)) = ban_decision
                        .map(|decision| (decision, true))
                        .or_else(|| pick_decision.map(|decision| (decision, false)))
                    {
                        let signature = (decision.action_id, decision.champion_id, is_ban);
                        if last_action.as_ref() != Some(&signature) {
                            match client
                                .complete_champion_action(decision.action_id, decision.champion_id)
                            {
                                Ok(()) => {
                                    shared.set_status(format!(
                                        "{}: {} を{}しました",
                                        display_position(&decision.assigned_position),
                                        decision.champion_name,
                                        if is_ban { "BAN" } else { "ロックイン" }
                                    ));
                                    last_action = Some(signature);
                                }
                                Err(error) => {
                                    shared.set_status(error);
                                    sleep_interruptibly(shared, Duration::from_millis(350));
                                    continue;
                                }
                            }
                        }
                    } else {
                        shared.set_status("チャンピオン選択を待機中");
                    }
                }
                Ok(None) => shared.set_status("チャンピオン選択を待機中"),
                Err(error) => shared.set_status(error),
            }
        } else {
            last_action = None;
            shared.set_status(format!("League接続中 — {}", display_phase(&phase)));
        }

        sleep_interruptibly(shared, Duration::from_millis(650));
    }
}

fn cache_champion_icons(shared: Arc<SharedState>) {
    while !shared.stop.load(Ordering::Relaxed) {
        let client = match LcuClient::discover() {
            Ok(client) => client,
            Err(_) => {
                sleep_interruptibly(&shared, Duration::from_secs(2));
                continue;
            }
        };
        let champions = match client.champions() {
            Ok(champions) => champions
                .into_iter()
                .filter(|champion| champion.id > 0)
                .collect::<Vec<_>>(),
            Err(_) => {
                sleep_interruptibly(&shared, Duration::from_secs(2));
                continue;
            }
        };
        if let Ok(mut current) = shared.champions.lock() {
            *current = champions.clone();
        }
        shared.icons_ready.store(0, Ordering::Relaxed);
        for champion in champions {
            if shared.stop.load(Ordering::Relaxed) {
                return;
            }
            let path = champion_icon_path(champion.id);
            let cached = path
                .metadata()
                .map(|metadata| metadata.len() > 128)
                .unwrap_or(false);
            if !cached && let Ok(bytes) = client.asset_bytes(&champion.square_portrait_path) {
                if let Some(parent) = path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let _ = fs::write(&path, bytes);
            }
            if path.is_file() {
                shared.icons_ready.fetch_add(1, Ordering::Relaxed);
            }
        }
        return;
    }
}

fn sleep_interruptibly(shared: &SharedState, duration: Duration) {
    let slice = Duration::from_millis(100);
    let mut elapsed = Duration::ZERO;
    while elapsed < duration && !shared.stop.load(Ordering::Relaxed) {
        let remaining = duration.saturating_sub(elapsed);
        let current = remaining.min(slice);
        thread::sleep(current);
        elapsed += current;
    }
}

fn display_phase(phase: &str) -> &str {
    match phase {
        "None" => "ホーム",
        "Lobby" => "ロビー",
        "Matchmaking" => "マッチ検索中",
        "ReadyCheck" => "レディーチェック",
        "ChampSelect" => "チャンピオン選択",
        "InProgress" => "ゲーム中",
        "EndOfGame" => "ゲーム終了",
        "PreEndOfGame" => "ゲーム終了処理中",
        "WaitingForStats" => "戦績を待機中",
        _ => phase,
    }
}

fn display_position(position: &str) -> &str {
    match position.to_ascii_lowercase().as_str() {
        "top" => "TOP",
        "jungle" => "JUNGLE",
        "middle" | "mid" => "MID",
        "bottom" | "bot" | "adc" => "ADC",
        "utility" | "support" => "SUPPORT",
        _ => position,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "起動中のLeagueクライアントとローカルキャッシュ書き込みが必要"]
    fn caches_champion_icons_from_running_client() {
        let shared = Arc::new(SharedState::new(Settings::default()));
        cache_champion_icons(shared.clone());
        let champion_count = shared
            .champions
            .lock()
            .map(|champions| champions.len())
            .unwrap_or_default();
        assert!(champion_count > 200);
        assert!(shared.icons_ready.load(Ordering::Relaxed) > 200);
    }
}
