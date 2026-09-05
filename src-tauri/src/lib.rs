mod config;
mod lcu;
mod models;
mod opgg;
mod tts;

use models::{AppState, ConnectionStatus};
use std::sync::Arc;
use tauri::{Emitter, Manager};
use tokio::sync::Mutex;

type SharedState = Arc<Mutex<AppState>>;

fn save_config(app: &tauri::AppHandle, s: &models::AppState) {
    // Merge into the on-disk config so other accounts' buckets (and the
    // active one) stay intact even if we don't yet know our puuid.
    let mut cfg = config::load(app);
    cfg.region = s.region.clone();
    cfg.auto_apply = s.auto_apply;
    cfg.auto_lock = s.auto_lock;
    cfg.auto_accept = s.auto_accept;
    cfg.tts_enabled = s.tts_enabled;
    cfg.overlay_position = s.overlay_position.clone();
    if let Some(puuid) = s.summoner_puuid.as_deref() {
        cfg.set_lp_history(puuid, s.lp_history.clone());
    }
    // If puuid isn't known yet (rare: pre-connect save), don't touch lp buckets.
    config::save(app, &cfg);
}

fn notify(title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    {
        log::info!("{}: {}", title, body);
    }
    #[cfg(target_os = "windows")]
    {
        let _ = (title, body); // suppress unused warnings; Windows notifications can be added later
    }
}

fn map_position(pos: &str) -> &'static str {
    match pos.trim().to_ascii_lowercase().as_str() {
        "top" => "top",
        "jungle" => "jungle",
        "middle" | "mid" => "mid",
        "bottom" | "adc" => "adc",
        "utility" | "support" => "support",
        _ => "",
    }
}

const SWIFTPLAY_QUEUE_ID: i64 = 490;

fn is_supported_summoners_rift_queue(queue_id: i64) -> bool {
    matches!(queue_id, 400 | 420 | 440 | SWIFTPLAY_QUEUE_ID)
}

fn same_riot_name(a: &str, b: &str) -> bool {
    let clean = |name: &str| name.split('#').next().unwrap_or(name).trim().to_ascii_lowercase();
    !a.is_empty() && !b.is_empty() && clean(a) == clean(b)
}

fn local_live_build_key(
    live: &models::LiveGameState,
    summoner_name: &str,
) -> Option<(i64, String)> {
    live.allies.iter()
        .find(|p| same_riot_name(&p.summoner_name, summoner_name))
        .or_else(|| live.allies.first())
        .and_then(|p| {
            let position = map_position(&p.position);
            (p.champion_id > 0 && !position.is_empty())
                .then(|| (p.champion_id, position.to_string()))
        })
}

#[tauri::command]
async fn get_state(state: tauri::State<'_, SharedState>) -> Result<AppState, String> {
    Ok(state.lock().await.clone())
}

#[tauri::command]
async fn set_auto_apply(
    enabled: bool,
    state: tauri::State<'_, SharedState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let mut s = state.lock().await;
    s.auto_apply = enabled;
    let _ = app_handle.emit("app-state-changed", s.clone());
    save_config(&app_handle, &s);
    Ok(())
}

#[tauri::command]
async fn set_region(
    region: String,
    state: tauri::State<'_, SharedState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let mut s = state.lock().await;
    s.region = region;
    let _ = app_handle.emit("app-state-changed", s.clone());
    save_config(&app_handle, &s);
    Ok(())
}

/// Watches for the LoL client to start, connects, runs the poll loop,
/// and reconnects automatically when the client restarts.
async fn watcher_loop(state: SharedState, app_handle: tauri::AppHandle) {
    loop {
        // Phase 1: Wait for LCU to become available
        let creds = loop {
            if let Some(creds) = lcu::read_lockfile() {
                if let Ok(summoner) = lcu::get_current_summoner(&creds).await {
                    let mut s = state.lock().await;
                    s.status = ConnectionStatus::Connected;
                    s.summoner_name = summoner.game_name.or(summoner.display_name);
                    s.profile_icon_id = summoner.profile_icon_id;
                    s.summoner_id = summoner.summoner_id;
                    let new_puuid = summoner.puuid.filter(|p| !p.is_empty());
                    // Hydrate lp_history from the right bucket on first
                    // connect or genuine account switch. We only treat
                    // Some(other) as a switch — None means the LCU didn't
                    // return puuid this poll, not that the account changed.
                    if let Some(ref puuid) = new_puuid {
                        let switched = s.summoner_puuid.as_ref() != Some(puuid);
                        s.summoner_puuid = Some(puuid.clone());
                        if switched {
                            let cfg = config::load(&app_handle);
                            let history = cfg.lp_history_for(puuid);
                            s.lp_history = history;
                            // Persist the migration so the next account
                            // that logs in doesn't inherit the legacy bucket.
                            if !cfg.legacy_lp_history.is_empty() && !cfg.accounts.contains_key(puuid) {
                                save_config(&app_handle, &s);
                            }
                        }
                    }
                    // Fetch match history and ranked stats
                    if let Ok(history) = lcu::get_match_history(&creds).await {
                        s.match_history = history;
                    }
                    if let Ok(ranked) = lcu::get_ranked_stats(&creds).await {
                        // Record initial LP if history is empty or LP changed
                        let should_record = s.lp_history.last()
                            .map(|last| last.lp != ranked.lp || last.tier != ranked.tier || last.rank != ranked.rank)
                            .unwrap_or(true);
                        if should_record && ranked.tier != "UNRANKED" {
                            s.lp_history.push(config::LpEntry {
                                timestamp: std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_millis() as i64,
                                lp: ranked.lp,
                                tier: ranked.tier.clone(),
                                rank: ranked.rank.clone(),
                            });
                        }
                        s.ranked = Some(ranked);
                    }
                    let _ = app_handle.emit("app-state-changed", s.clone());
                    log::info!("Connected to LCU");
                    break creds;
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        };

        // Phase 2: Run poll loop (returns when LCU disconnects)
        poll_loop(creds, Arc::clone(&state), app_handle.clone()).await;

        // Phase 3: LCU disconnected, loop back to watch
        log::info!("LCU disconnected, watching for reconnect...");
    }
}

async fn poll_loop(
    creds: models::LcuCredentials,
    state: SharedState,
    app_handle: tauri::AppHandle,
) {
    let mut last_build_key: Option<(i64, String)> = None;
    let mut last_draft_hash: u64 = 0;
    let mut last_swiftplay_hash: u64 = 0;
    let mut swiftplay_builds: std::collections::HashMap<
        (i64, String),
        (models::ChampionBuild, models::BuildAlternatives),
    > = std::collections::HashMap::new();
    // Bounded retries for backfilling the post-game gold chart from the LCU.
    let mut timeline_backfill_tries: u32 = 0;

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        let phase = match lcu::get_gameflow_phase(&creds).await {
            Ok(p) => {
                log::debug!("Phase: {}", p);
                p
            }
            Err(_) => {
                let mut s = state.lock().await;
                s.status = ConnectionStatus::Disconnected;
                s.champion_id = None;
                s.build = None;
                s.build_alternatives = None;
                s.counters.clear();
                s.draft = None;
                s.recommendations = vec![];
                let _ = app_handle.emit("app-state-changed", s.clone());
                break;
            }
        };

        if phase == "ReadyCheck" {
            let auto_accept = state.lock().await.auto_accept;
            if auto_accept {
                // Accept once, then wait for phase to change
                match lcu::accept_ready_check(&creds).await {
                    Ok(()) => {
                        // Wait for phase to transition, don't retry
                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    }
                    Err(_) => {} // Already accepted or expired, ignore
                }
            }
            continue;
        }

        // Swiftplay has no ChampSelect phase. Its two champion, role, rune,
        // and spell loadouts live in the lobby, so prepare both before queue.
        if phase == "Lobby" || phase == "Matchmaking" {
            let queue_id = lcu::get_current_queue(&creds).await.map(|q| q.0).unwrap_or(0);
            if queue_id == SWIFTPLAY_QUEUE_ID {
                if let Ok(mut slots) = lcu::get_swiftplay_slots(&creds).await {
                    use std::hash::{Hash, Hasher};
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    for slot in &slots {
                        slot.champion_id.hash(&mut hasher);
                        map_position(&slot.position_preference).hash(&mut hasher);
                    }
                    let slots_hash = hasher.finish();

                    if slots_hash != 0 && slots_hash != last_swiftplay_hash {
                        let (region, auto_apply, summoner_id) = {
                            let s = state.lock().await;
                            (s.region.clone(), s.auto_apply, s.summoner_id)
                        };
                        let mut fetched_all = true;
                        let mut first_result: Option<(i64, String, models::ChampionBuild, models::BuildAlternatives)> = None;

                        for slot in &mut slots {
                            let position = map_position(&slot.position_preference);
                            if slot.champion_id <= 0 || position.is_empty() {
                                continue;
                            }
                            match opgg::fetch_champion_data(&region, slot.champion_id, position).await {
                                Ok(result) => {
                                    let key = (slot.champion_id, position.to_string());
                                    swiftplay_builds.insert(
                                        key,
                                        (result.build.clone(), result.alternatives.clone()),
                                    );
                                    if first_result.is_none() {
                                        first_result = Some((
                                            slot.champion_id,
                                            position.to_string(),
                                            result.build.clone(),
                                            result.alternatives.clone(),
                                        ));
                                    }
                                    if auto_apply {
                                        if let Some(ref runes) = result.build.runes {
                                            slot.perks = lcu::swiftplay_perks_string(runes);
                                        }
                                        if let Some([spell1, spell2]) = result.build.summoner_spells {
                                            slot.spell1 = spell1;
                                            slot.spell2 = spell2;
                                        }
                                        if let Some(sid) = summoner_id {
                                            if let Err(e) = lcu::apply_item_set(
                                                &creds,
                                                sid,
                                                slot.champion_id,
                                                &result.build,
                                            ).await {
                                                log::warn!("Failed to apply Swiftplay item set: {}", e);
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    fetched_all = false;
                                    log::warn!("Failed to fetch Swiftplay build for {} {}: {}", slot.champion_id, position, e);
                                }
                            }
                        }

                        if auto_apply && fetched_all {
                            if let Err(e) = lcu::apply_swiftplay_slots(&creds, &slots).await {
                                fetched_all = false;
                                log::warn!("Failed to apply Swiftplay loadouts: {}", e);
                            }
                        }
                        if fetched_all {
                            last_swiftplay_hash = slots_hash;
                            if let Some((champion_id, position, build, alternatives)) = first_result {
                                let mut s = state.lock().await;
                                s.game_mode = "classic".to_string();
                                s.champion_id = Some(champion_id);
                                s.assigned_position = Some(position);
                                s.build = Some(build);
                                s.build_alternatives = Some(alternatives);
                                let _ = app_handle.emit("app-state-changed", s.clone());
                            }
                            log::info!("Swiftplay builds prepared for {} slot(s)", slots.len());
                        }
                    }
                }
                continue;
            }
        }

        if phase == "ChampSelect" {
            // Detect game mode + fetch ban suggestions + comfort picks on first tick
            let first_tick = state.lock().await.status != ConnectionStatus::ChampSelect;
            if first_tick {
                let mut is_aram_mode = false;
                if let Ok((queue_id, _)) = lcu::get_current_queue(&creds).await {
                    is_aram_mode = queue_id == 450 || queue_id == 900;
                    if !is_aram_mode && !is_supported_summoners_rift_queue(queue_id) {
                        log::debug!("Using generic classic support for queue {}", queue_id);
                    }
                    let mut s = state.lock().await;
                    s.game_mode = if is_aram_mode { "aram".to_string() } else { "classic".to_string() };
                }

                if !is_aram_mode {
                    // Extract what we need from state BEFORE making HTTP calls
                    let (region, history) = {
                        let s = state.lock().await;
                        (s.region.clone(), s.match_history.clone())
                    };

                    if let Ok(session) = lcu::get_champ_select_session(&creds).await {
                        let my_pos = session.my_team.iter()
                            .find(|p| p.cell_id == session.local_player_cell_id)
                            .and_then(|p| p.assigned_position.clone())
                            .unwrap_or_default()
                            .to_lowercase();
                        let opgg_pos = map_position(&my_pos);

                        // Fetch ban suggestions (no lock held during HTTP)
                        if let Ok(bans) = opgg::fetch_ban_suggestions(&region, opgg_pos).await {
                            state.lock().await.ban_suggestions = bans;
                        }

                        // Calculate comfort picks from match history
                        let mut champ_counts: std::collections::HashMap<i64, i32> = std::collections::HashMap::new();
                        for m in &history {
                            *champ_counts.entry(m.champion_id).or_insert(0) += 1;
                        }
                        let mut top_champs: Vec<(i64, i32)> = champ_counts.into_iter()
                            .filter(|(_, count)| *count >= 2)
                            .collect();
                        top_champs.sort_by(|a, b| b.1.cmp(&a.1));
                        top_champs.truncate(3);

                        if !top_champs.is_empty() {
                            let champ_ids: Vec<i64> = top_champs.iter().map(|(id, _)| *id).collect();
                            if let Ok(win_rates) = opgg::fetch_champion_win_rates(
                                &region, opgg_pos, &champ_ids
                            ).await {
                                let comfort: Vec<models::ComfortPick> = top_champs.iter().map(|(id, count)| {
                                    models::ComfortPick {
                                        champion_id: *id,
                                        games_played: *count,
                                        meta_win_rate: win_rates.get(id).copied().unwrap_or(0.5),
                                    }
                                }).collect();
                                state.lock().await.comfort_picks = comfort;
                            }
                        }
                    }
                }
            }
            let is_aram = state.lock().await.game_mode == "aram";

            let session = match lcu::get_champ_select_session(&creds).await {
                Ok(s) => s,
                Err(_) => continue,
            };

            let (mut champion_id, position, champion_locked) = lcu::extract_champion_from_session(&session);
            let draft = lcu::extract_draft_state(&session);

            // If our hovered/intended champion got banned, drop the pick so the
            // stale build is cleared and pick recommendations come back. You can
            // never have a locked champion that is banned, so this only ever fires
            // on a pre-lock intent during the ban phase.
            if champion_id > 0
                && (draft.ally_bans.contains(&champion_id) || draft.enemy_bans.contains(&champion_id))
            {
                log::info!("Intended champion {} was banned; clearing pick state", champion_id);
                champion_id = 0;
                if last_build_key.is_some() {
                    last_build_key = None;
                    let mut s = state.lock().await;
                    s.champion_id = None;
                    s.build = None;
                    s.build_alternatives = None;
                    s.counters.clear();
                    let _ = app_handle.emit("app-state-changed", s.clone());
                }
            }

            // Check if ban phase is still active
            let ban_active = session.actions.iter().flatten()
                .any(|a| a.action_type == "ban" && !a.completed);

            // Hash draft state to detect changes
            let draft_hash = {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                for a in &draft.allies { a.champion_id.hash(&mut hasher); }
                for e in &draft.enemies { e.champion_id.hash(&mut hasher); }
                for b in &draft.ally_bans { b.hash(&mut hasher); }
                for b in &draft.enemy_bans { b.hash(&mut hasher); }
                hasher.finish()
            };

            // Update status
            {
                let mut s = state.lock().await;
                if s.status != ConnectionStatus::ChampSelect {
                    s.status = ConnectionStatus::ChampSelect;
                    s.viewing_past_match = false;
                    s.post_game = None;
                    notify("LuvvyLoL", "Champion Select has started!");
                }
                s.draft = Some(draft.clone());
                s.ban_phase_active = ban_active;
                s.champion_locked = champion_locked;
                let _ = app_handle.emit("app-state-changed", s.clone());

                // ARAM: update bench champion IDs (after emit so UI always updates)
                if is_aram && !session.bench_champion_ids.is_empty() {
                    let bench_changed = s.aram_bench.iter().map(|b| b.champion_id).collect::<Vec<_>>()
                        != session.bench_champion_ids;
                    if bench_changed {
                        let all_ids: Vec<i64> = session.bench_champion_ids.iter()
                            .chain(session.my_team.iter().map(|p| &p.champion_id))
                            .filter(|&&id| id > 0)
                            .copied()
                            .collect();
                        let region = s.region.clone();
                        drop(s);
                        // Try ARAM tier list, fallback to ranked tier list
                        let rates = match opgg::fetch_aram_win_rates(&region, &all_ids).await {
                            Ok(r) => r,
                            Err(_) => opgg::fetch_champion_win_rates(&region, "aram", &all_ids).await.unwrap_or_default(),
                        };
                        if !rates.is_empty() {
                            let mut s = state.lock().await;
                            s.aram_bench = all_ids.iter().map(|&id| {
                                models::AramBenchChampion {
                                    champion_id: id,
                                    win_rate: *rates.get(&id).unwrap_or(&0.0),
                                }
                            }).collect();
                            let _ = app_handle.emit("app-state-changed", s.clone());
                        }
                    }
                }
            }

            // Re-fetch when either champion or assigned role changes. The role
            // often arrives a tick later than the champion in Draft/Ranked.
            let opgg_pos = if is_aram { "aram" } else { map_position(&position) };
            let build_key = (champion_id, opgg_pos.to_string());
            if champion_id > 0 && !opgg_pos.is_empty() && last_build_key.as_ref() != Some(&build_key) {
                last_build_key = Some(build_key);
                log::info!("Champion detected: {} (position: {})", champion_id, position);

                {
                    let mut s = state.lock().await;
                    s.champion_id = Some(champion_id);
                    s.assigned_position = Some(position.clone());
                    s.build = None;
                s.build_alternatives = None;
                s.counters.clear();
                    let _ = app_handle.emit("app-state-changed", s.clone());
                }

                let (region, auto_apply, summoner_id) = {
                    let s = state.lock().await;
                    (s.region.clone(), s.auto_apply, s.summoner_id)
                };

                match opgg::fetch_champion_data(&region, champion_id, opgg_pos).await {
                    Ok(result) => {
                        log::info!("Champion data fetched for {}", champion_id);
                        let build = result.build.clone();
                        {
                            let mut s = state.lock().await;
                            s.build = Some(result.build);
                            s.build_alternatives = Some(result.alternatives);
                            // Store counters with string keys for JSON serialization
                            s.counters = result.counters.iter()
                                .map(|(k, v)| (k.to_string(), *v))
                                .collect();
                            let _ = app_handle.emit("app-state-changed", s.clone());
                        }

                        if auto_apply {
                            if let Some(ref runes) = build.runes {
                                if let Err(e) = lcu::apply_runes(&creds, runes).await {
                                    log::warn!("Failed to apply runes: {}", e);
                                }
                            }
                            if let Some([s1, s2]) = build.summoner_spells {
                                if let Err(e) = lcu::apply_summoner_spells(&creds, s1, s2).await {
                                    log::warn!("Failed to apply spells: {}", e);
                                }
                            }
                            if let Some(sid) = summoner_id {
                                if let Err(e) = lcu::apply_item_set(&creds, sid, champion_id, &build).await {
                                    log::warn!("Failed to apply items: {}", e);
                                }
                            }
                            log::info!("Auto-apply completed");
                        }
                    }
                    Err(e) => log::warn!("Failed to fetch champion data: {}", e),
                }
            }

            // If draft changed and we haven't locked in yet, generate recommendations
            if draft_hash != last_draft_hash {
                last_draft_hash = draft_hash;

                let region = state.lock().await.region.clone();
                // Get position from draft state (more reliable than assigned_position)
                let my_pos = draft.allies.iter()
                    .find(|a| a.is_local)
                    .map(|a| a.position.clone())
                    .unwrap_or_default();
                let opgg_pos = map_position(&my_pos);

                // Only recommend until we've locked in our pick (a hovered champion
                // still counts as "not decided") and not ARAM.
                if !champion_locked && !opgg_pos.is_empty() && !is_aram {
                    let enemies_with_pos: Vec<(i64, String)> = draft.enemies.iter()
                        .filter(|e| e.champion_id > 0)
                        .map(|e| (e.champion_id, map_position(&e.position).to_string()))
                        .collect();
                    let ally_ids: Vec<i64> = draft.allies.iter()
                        .filter(|a| !a.is_local)
                        .map(|a| a.champion_id)
                        .filter(|&id| id > 0)
                        .collect();

                    let all_bans: Vec<i64> = draft.ally_bans.iter().chain(draft.enemy_bans.iter()).copied().collect();
                    let (comfort_map, ban_suggestion_ids): (std::collections::HashMap<i64, i32>, Vec<i64>) = {
                        let s = state.lock().await;
                        let cm = s.comfort_picks.iter()
                            .map(|c| (c.champion_id, c.games_played))
                            .collect();
                        let bs = s.ban_suggestions.iter().map(|b| b.champion_id).collect();
                        (cm, bs)
                    };
                    match opgg::recommend_picks(
                        &region, opgg_pos, &enemies_with_pos, &all_bans, &ally_ids,
                        &comfort_map, &ban_suggestion_ids,
                    ).await {
                        Ok(recs) => {
                            let mut s = state.lock().await;
                            s.recommendations = recs;
                            let _ = app_handle.emit("app-state-changed", s.clone());
                        }
                        Err(e) => log::warn!("Failed to get recommendations: {}", e),
                    }
                }

                // Fetch counters for visible enemies (even before we pick)
                let visible_enemies: Vec<(i64, String)> = draft.enemies.iter()
                    .filter(|e| e.champion_id > 0)
                    .map(|e| (e.champion_id, map_position(&e.position).to_string()))
                    .collect();

                if !visible_enemies.is_empty() && !is_aram {
                    let mut all_counters: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
                    for (enemy_id, enemy_pos) in &visible_enemies {
                        let pos = if enemy_pos.is_empty() { opgg_pos } else { enemy_pos.as_str() };
                        if let Ok(counters) = opgg::fetch_counters(&region, *enemy_id, pos).await {
                            // Store as "our WR vs this enemy" — invert the perspective
                            // counters contains enemy's WR against each champion
                            // We want: for each enemy, what's the average WR against them
                            let avg_wr = if counters.is_empty() { 0.5 } else {
                                counters.values().sum::<f64>() / counters.len() as f64
                            };
                            // avg_wr is the enemy's average WR against all champions
                            // So 1.0 - avg_wr is roughly "how beatable" the enemy is
                            all_counters.insert(enemy_id.to_string(), 1.0 - avg_wr);
                        }
                    }
                    if !all_counters.is_empty() {
                        let mut s = state.lock().await;
                        // Merge: only fill in entries we don't already have from the build's counter list.
                        // The build's counters are real per-matchup OP.GG data; all_counters is a generic
                        // "beatability" derivation. Real data should always win when available.
                        for (k, v) in all_counters {
                            s.counters.entry(k).or_insert(v);
                        }
                        let _ = app_handle.emit("app-state-changed", s.clone());
                    }

                    // Generate prediction if both teams have picks
                    if !visible_enemies.is_empty() {
                        let ally_champs: Vec<(i64, String)> = draft.allies.iter()
                            .filter(|a| a.champion_id > 0)
                            .map(|a| (a.champion_id, map_position(&a.position).to_string()))
                            .collect();

                        if !ally_champs.is_empty() {
                            match opgg::generate_prediction(&region, &ally_champs, &visible_enemies).await {
                                Ok(pred) => {
                                    let mut s = state.lock().await;
                                    s.prediction = Some(pred);
                                    let _ = app_handle.emit("app-state-changed", s.clone());
                                }
                                Err(e) => log::warn!("Failed to generate prediction: {}", e),
                            }
                        }
                    }
                }
            }
        } else if phase == "InProgress" || phase == "GameStart" {
            let (already_in_game, sid, my_name, region) = {
                let s = state.lock().await;
                (
                    s.status == ConnectionStatus::InGame && s.live_game.is_some(),
                    s.summoner_id,
                    s.summoner_name.clone().unwrap_or_default(),
                    s.region.clone(),
                )
            };
            if !already_in_game {
                // Let TAB show an immediate, visible loading panel while the
                // player/rank lookup finishes.
                {
                    let mut s = state.lock().await;
                    s.status = ConnectionStatus::InGame;
                    let _ = app_handle.emit("app-state-changed", s.clone());
                }
                // Fetch live game info once on transition
                match lcu::get_live_game(&creds, sid, &my_name).await {
                    Ok(mut live) => {
                        let live_key = local_live_build_key(&live, &my_name);
                        let mut recommended = live_key.as_ref()
                            .and_then(|key| swiftplay_builds.get(key).cloned());
                        if recommended.is_none() {
                            if let Some((champion_id, ref position)) = live_key {
                                match opgg::fetch_champion_data(&region, champion_id, position).await {
                                    Ok(result) => {
                                        recommended = Some((result.build, result.alternatives));
                                    }
                                    Err(e) => log::warn!("Live build fallback failed for {} {}: {}", champion_id, position, e),
                                }
                            }
                        }
                        let mut s = state.lock().await;
                        // Draft/Ranked preserve champ-select data. Swiftplay uses
                        // its matching preloaded slot, with a live API fallback.
                        if let Some((build, alternatives)) = recommended {
                            live.recommended_build = Some(build);
                            live.recommended_alternatives = Some(alternatives);
                        } else {
                            live.recommended_build = s.build.clone();
                            live.recommended_alternatives = s.build_alternatives.clone();
                        }
                        s.status = ConnectionStatus::InGame;
                        s.live_game = Some(live);
                        s.champion_id = None;
                        s.build = None;
                        s.build_alternatives = None;
                        s.counters.clear();
                        s.draft = None;
                        s.recommendations = vec![];
                        last_build_key = None;
                        last_draft_hash = 0;
                        let _ = app_handle.emit("app-state-changed", s.clone());
                        log::info!("Live game info loaded");
                    }
                    Err(e) => {
                        state.lock().await.status = ConnectionStatus::Connected;
                        log::warn!("Failed to fetch live game: {}", e);
                    }
                }
            } else {
                // Poll live client data API for real-time stats
                let mut s = state.lock().await;
                if let Some(ref mut live) = s.live_game {
                    match lcu::poll_live_game_data(live, &my_name).await {
                        Ok(_) => {
                            let _ = app_handle.emit("app-state-changed", s.clone());
                        }
                        Err(e) => {
                            log::debug!("Live client poll: {}", e);
                        }
                    }
                }
            }
        } else if phase == "WaitingForStats" || phase == "EndOfGame" {
            let mut s = state.lock().await;
            if s.status != ConnectionStatus::PostGame {
                // Transition into post-game: fetch stats once
                match lcu::get_end_of_game_stats(&creds).await {
                    Ok(mut stats) => {
                        s.status = ConnectionStatus::PostGame;
                        // Compute phase stats and gold timeline from live game snapshots
                        if let Some(ref live_game) = s.live_game {
                            if let Some(ref live_data) = live_game.live_data {
                                if !live_data.snapshots.is_empty() {
                                    log::info!("Computing phase stats from {} snapshots", live_data.snapshots.len());
                                    for team in &mut stats.teams {
                                        for player in &mut team.players {
                                            let phases = lcu::compute_phase_stats(&live_data.snapshots, &player.summoner_name);
                                            if !phases.is_empty() {
                                                player.phase_stats = phases;
                                            }
                                        }
                                    }
                                    // Gold timeline and death impacts
                                    let ally_names: Vec<String> = live_game.allies.iter().map(|p| p.summoner_name.clone()).collect();
                                    let enemy_names: Vec<String> = live_game.enemies.iter().map(|p| p.summoner_name.clone()).collect();
                                    let champions: std::collections::HashMap<String, i64> = live_game.allies.iter()
                                        .chain(live_game.enemies.iter())
                                        .map(|p| (p.summoner_name.clone(), p.champion_id))
                                        .collect();
                                    let (timeline, deaths) = lcu::compute_gold_timeline(&live_data.snapshots, &ally_names, &enemy_names, &champions);
                                    stats.gold_timeline = timeline;
                                    stats.death_events = deaths;
                                }
                            }
                        }
                        // No snapshots (app started mid-game, or Live Client
                        // Data never answered) means an empty chart. The LCU
                        // has the real timeline — try it now, and keep retrying
                        // below, since the match is indexed a few seconds after
                        // the game ends.
                        if stats.gold_timeline.is_empty() && stats.game_id > 0 {
                            match lcu::get_gold_timeline_for_game(&creds, stats.game_id).await {
                                Ok((timeline, deaths)) => {
                                    stats.gold_timeline = timeline;
                                    stats.death_events = deaths;
                                    log::info!("Gold timeline backfilled from LCU at post-game transition");
                                }
                                Err(e) => log::info!("Gold timeline not ready yet: {}", e),
                            }
                        }
                        timeline_backfill_tries = 0;
                        s.post_game = Some(stats);
                        s.live_game = None; // Clean up after consuming snapshots
                        s.champion_id = None;
                        s.build = None;
                s.build_alternatives = None;
                s.counters.clear();
                        s.draft = None;
                        s.recommendations = vec![];
                        last_build_key = None;
                        last_draft_hash = 0;
                        let _ = app_handle.emit("app-state-changed", s.clone());
                        log::info!("Post-game stats loaded");
                    }
                    Err(e) => log::warn!("Failed to fetch post-game stats: {}", e),
                }
            } else {
                // Already in post-game. Retry the backfill until the LCU has
                // indexed the match, so the user does not have to leave and
                // re-open the game to see the chart.
                let pending = s.post_game.as_ref()
                    .map(|p| p.gold_timeline.is_empty() && p.game_id > 0)
                    .unwrap_or(false);
                if pending && timeline_backfill_tries < 15 {
                    timeline_backfill_tries += 1;
                    let game_id = s.post_game.as_ref().map(|p| p.game_id).unwrap_or(0);
                    if let Ok((timeline, deaths)) = lcu::get_gold_timeline_for_game(&creds, game_id).await {
                        if !timeline.is_empty() {
                            if let Some(ref mut pg) = s.post_game {
                                pg.gold_timeline = timeline;
                                pg.death_events = deaths;
                            }
                            let _ = app_handle.emit("app-state-changed", s.clone());
                            log::info!("Gold timeline backfilled after {} tries", timeline_backfill_tries);
                        }
                    }
                }
            }
        } else {
            let mut s = state.lock().await;
            // When viewing a past match, only interrupt for important phases
            if s.viewing_past_match {
                drop(s);
                continue;
            }
            if s.status != ConnectionStatus::Connected {
                s.status = ConnectionStatus::Connected;
                s.champion_id = None;
                s.champion_name = None;
                s.assigned_position = None;
                s.build = None;
                s.build_alternatives = None;
                s.counters.clear();
                s.draft = None;
                s.recommendations = vec![];
                s.post_game = None;
                // Keep live_game until post-game has consumed snapshots
                if s.live_game.as_ref().and_then(|lg| lg.live_data.as_ref()).map(|ld| ld.snapshots.is_empty()).unwrap_or(true) {
                    s.live_game = None;
                }
                s.game_mode = "classic".to_string();
                last_build_key = None;
                last_draft_hash = 0;
                timeline_backfill_tries = 0;
                // Refresh match history and ranked stats (delay for API indexing)
                drop(s);
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                let mut s = state.lock().await;
                if let Ok(history) = lcu::get_match_history(&creds).await {
                    s.match_history = history;
                }
                if let Ok(ranked) = lcu::get_ranked_stats(&creds).await {
                    // Record LP if it changed
                    let should_record = s.ranked.as_ref()
                        .map(|old| old.lp != ranked.lp || old.tier != ranked.tier || old.rank != ranked.rank)
                        .unwrap_or(true);

                    if should_record && ranked.tier != "UNRANKED" {
                        let entry = config::LpEntry {
                            timestamp: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as i64,
                            lp: ranked.lp,
                            tier: ranked.tier.clone(),
                            rank: ranked.rank.clone(),
                        };
                        s.lp_history.push(entry);
                        // Keep last 50 entries
                        if s.lp_history.len() > 50 {
                            s.lp_history = s.lp_history[s.lp_history.len()-50..].to_vec();
                        }
                        // Persist
                        save_config(&app_handle, &s);
                    }

                    s.ranked = Some(ranked);
                }
                let _ = app_handle.emit("app-state-changed", s.clone());
            }
        }
    }
}

#[tauri::command]
async fn apply_build_now(state: tauri::State<'_, SharedState>) -> Result<(), String> {
    let s = state.lock().await;
    let creds = lcu::read_lockfile().ok_or("League client not found")?;

    if let Some(ref build) = s.build {
        if let Some(ref runes) = build.runes {
            lcu::apply_runes(&creds, runes).await?;
        }
        if let Some([s1, s2]) = build.summoner_spells {
            lcu::apply_summoner_spells(&creds, s1, s2).await?;
        }
        if let (Some(sid), Some(cid)) = (s.summoner_id, s.champion_id) {
            lcu::apply_item_set(&creds, sid, cid, build).await?;
        }
        Ok(())
    } else {
        Err("No build available".to_string())
    }
}

#[tauri::command]
async fn select_build_option(
    category: String,
    index: usize,
    state: tauri::State<'_, SharedState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let mut s = state.lock().await;

    // Clone what we need from alternatives before mutating build
    let alts = s.build_alternatives.clone().ok_or("No alternatives available")?;

    let build = s.build.as_mut().ok_or("No build available")?;
    match category.as_str() {
        "runes" => {
            let opt = alts.runes.get(index).ok_or("Invalid rune index")?;
            build.runes = Some(opt.build.clone());
        }
        "spells" => {
            let opt = alts.summoner_spells.get(index).ok_or("Invalid spell index")?;
            build.summoner_spells = Some(opt.ids);
        }
        "items" => {
            let opt = alts.core_items.get(index).ok_or("Invalid item index")?;
            build.core_items = opt.ids.clone();
        }
        _ => return Err("Unknown category".to_string()),
    }

    let _ = app_handle.emit("app-state-changed", s.clone());

    // Re-apply if auto_apply
    if s.auto_apply {
        let creds = lcu::read_lockfile().ok_or("League client not found")?;
        let build = s.build.clone().unwrap();
        let summoner_id = s.summoner_id;
        let champion_id = s.champion_id;
        drop(s);

        if let Some(ref runes) = build.runes {
            if category == "runes" {
                let _ = lcu::apply_runes(&creds, runes).await;
            }
        }
        if let Some([s1, s2]) = build.summoner_spells {
            if category == "spells" {
                let _ = lcu::apply_summoner_spells(&creds, s1, s2).await;
            }
        }
        if category == "items" {
            if let (Some(sid), Some(cid)) = (summoner_id, champion_id) {
                let _ = lcu::apply_item_set(&creds, sid, cid, &build).await;
            }
        }
    }

    Ok(())
}

#[tauri::command]
async fn set_auto_lock(
    enabled: bool,
    state: tauri::State<'_, SharedState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let mut s = state.lock().await;
    s.auto_lock = enabled;
    let _ = app_handle.emit("app-state-changed", s.clone());
    save_config(&app_handle, &s);
    Ok(())
}

#[tauri::command]
async fn set_auto_accept(
    enabled: bool,
    state: tauri::State<'_, SharedState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let mut s = state.lock().await;
    s.auto_accept = enabled;
    let _ = app_handle.emit("app-state-changed", s.clone());
    save_config(&app_handle, &s);
    Ok(())
}

#[tauri::command]
async fn set_tts_enabled(
    enabled: bool,
    state: tauri::State<'_, SharedState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let mut s = state.lock().await;
    s.tts_enabled = enabled;
    let _ = app_handle.emit("app-state-changed", s.clone());
    save_config(&app_handle, &s);
    Ok(())
}

#[tauri::command]
async fn speak(
    text: String,
    state: tauri::State<'_, SharedState>,
) -> Result<(), String> {
    let enabled = state.lock().await.tts_enabled;
    if enabled {
        tts::speak(&text);
    }
    Ok(())
}

#[tauri::command]
async fn pick_champion(
    champion_id: i64,
    state: tauri::State<'_, SharedState>,
) -> Result<(), String> {
    let creds = lcu::read_lockfile().ok_or("League client not found")?;
    let session = lcu::get_champ_select_session(&creds).await?;
    let auto_lock = state.lock().await.auto_lock;

    // Log all actions for debugging
    for (gi, group) in session.actions.iter().enumerate() {
        for action in group {
            if action.actor_cell_id == session.local_player_cell_id {
                log::info!(
                    "My action[{}]: id={} type={} champ={} completed={} inProgress={}",
                    gi, action.id, action.action_type, action.champion_id, action.completed, action.is_in_progress
                );
            }
        }
    }

    let action_id = lcu::find_my_action(&session, "pick")
        .ok_or("No active pick action — it's not your turn to pick")?;

    lcu::select_champion(&creds, action_id, champion_id, auto_lock).await
}

#[tauri::command]
async fn ban_champion(
    champion_id: i64,
    state: tauri::State<'_, SharedState>,
) -> Result<(), String> {
    let creds = lcu::read_lockfile().ok_or("League client not found")?;
    let session = lcu::get_champ_select_session(&creds).await?;
    let auto_lock = state.lock().await.auto_lock;

    for (gi, group) in session.actions.iter().enumerate() {
        for action in group {
            if action.actor_cell_id == session.local_player_cell_id {
                log::info!(
                    "My ban action[{}]: id={} type={} champ={} completed={} inProgress={}",
                    gi, action.id, action.action_type, action.champion_id, action.completed, action.is_in_progress
                );
            }
        }
    }

    let action_id = lcu::find_my_action(&session, "ban")
        .ok_or("No active ban action — it's not your turn to ban")?;

    lcu::select_champion(&creds, action_id, champion_id, auto_lock).await
}

#[derive(serde::Serialize)]
struct PlayerProfile {
    name: String,
    rank: String,
    matches: Vec<models::MatchHistoryEntry>,
}

#[tauri::command]
async fn view_player_profile(puuid: String) -> Result<PlayerProfile, String> {
    let creds = lcu::read_lockfile().ok_or("League client not found")?;
    let c1 = creds.clone();
    let c2 = creds.clone();
    let p1 = puuid.clone();
    let p2 = puuid.clone();

    let (name, rank, matches) = tokio::join!(
        lcu::get_summoner_name_by_puuid(&c1, &p1),
        lcu::get_player_rank(&c2, &p2),
        lcu::get_player_match_history(&creds, &puuid),
    );

    Ok(PlayerProfile {
        name,
        rank,
        matches: matches.unwrap_or_default(),
    })
}

#[tauri::command]
async fn view_match_details(
    game_id: i64,
    state: tauri::State<'_, SharedState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let creds = lcu::read_lockfile().ok_or("League client not found")?;
    let stats = lcu::get_match_details(&creds, game_id).await?;

    let mut s = state.lock().await;
    s.post_game = Some(stats);
    s.status = models::ConnectionStatus::PostGame;
    s.viewing_past_match = true;
    let _ = app_handle.emit("app-state-changed", s.clone());
    Ok(())
}

/// Returns past match details without changing the app's current screen.
/// The lobby uses this for the expandable ten player performance table.
#[tauri::command]
async fn get_match_details_preview(game_id: i64) -> Result<models::PostGameStats, String> {
    let creds = lcu::read_lockfile().ok_or("League client not found")?;
    lcu::get_match_details(&creds, game_id).await
}

#[tauri::command]
async fn swap_aram_bench(champion_id: i64) -> Result<(), String> {
    let creds = lcu::read_lockfile().ok_or("League client not found")?;
    lcu::swap_bench_champion(&creds, champion_id).await
}

#[tauri::command]
async fn back_to_lobby(
    state: tauri::State<'_, SharedState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let mut s = state.lock().await;
    s.status = models::ConnectionStatus::Connected;
    s.post_game = None;
    s.viewing_past_match = false;
    let _ = app_handle.emit("app-state-changed", s.clone());
    Ok(())
}

#[tauri::command]
async fn set_overlay_position(
    position: String,
    state: tauri::State<'_, SharedState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let mut s = state.lock().await;
    s.overlay_position = position.clone();
    save_config(&app_handle, &s);
    let _ = app_handle.emit("app-state-changed", s.clone());
    drop(s);
    position_overlay_window(&app_handle, &position)
}

#[tauri::command]
async fn test_overlay(
    state: tauri::State<'_, SharedState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let position = {
        let s = state.lock().await;
        if s.overlay_position == "off" {
            "top-right".to_string()
        } else {
            s.overlay_position.clone()
        }
    };
    position_overlay_window(&app_handle, &position)?;
    let window = app_handle
        .get_webview_window("overlay")
        .ok_or("Overlay window was not created")?;
    window.show().map_err(|e| format!("Failed to show overlay: {}", e))?;
    window
        .set_ignore_cursor_events(true)
        .map_err(|e| format!("Failed to make overlay click-through: {}", e))?;
    #[cfg(target_os = "macos")]
    configure_macos_overlay(&window, true)?;

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        let _ = window.hide();
    });
    Ok(())
}

fn position_overlay_window(app_handle: &tauri::AppHandle, position: &str) -> Result<(), String> {
    if position == "off" {
        if let Some(window) = app_handle.get_webview_window("overlay") {
            let _ = window.hide();
        }
        return Ok(());
    }
    if let Some(window) = app_handle.get_webview_window("overlay") {
        let scale = window.scale_factor().unwrap_or(1.0);
        let monitor = window.current_monitor()
            .map_err(|e| format!("Failed to get monitor: {}", e))?
            .ok_or("No monitor found")?;
        let screen = monitor.size();
        let origin = monitor.position();
        let outer = window.outer_size().unwrap_or(tauri::PhysicalSize::new(340, 620));
        let ow = outer.width as i32;
        let oh = outer.height as i32;
        let margin = (10.0 * scale) as i32;
        let sw = screen.width as i32;
        let sh = screen.height as i32;
        let (x, y) = match position {
            "top-left" => (origin.x + margin, origin.y + margin),
            "top-right" => (origin.x + sw - ow - margin, origin.y + margin),
            "bottom-left" => (origin.x + margin, origin.y + sh - oh - margin),
            "bottom-right" => (origin.x + sw - ow - margin, origin.y + sh - oh - margin),
            "center" => (origin.x + sw / 2 - ow / 2, origin.y + sh / 2 - oh / 2),
            _ => (origin.x + sw - ow - margin, origin.y + margin),
        };
        window.set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(x, y)))
            .map_err(|e| format!("Failed to set position: {}", e))?;
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
/// Monitors TAB key and shows/hides the overlay window during in-game.
async fn overlay_loop(state: SharedState, app_handle: tauri::AppHandle) {
    #[cfg(not(target_os = "macos"))]
    use device_query::{DeviceQuery, DeviceState, Keycode};

    // Keep retrying instead of permanently disabling the overlay when macOS
    // grants keyboard access after the app has already launched.
    #[cfg(not(target_os = "macos"))]
    let mut device_state: Option<DeviceState> = None;
    #[cfg(target_os = "macos")]
    let macos_tab_events = start_macos_tab_monitor();
    let mut was_visible = false;
    let mut permission_help_shown = false;

    loop {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        #[cfg(target_os = "macos")]
        if !macos_accessibility_trusted() {
            if !permission_help_shown {
                permission_help_shown = true;
                notify(
                    "LuvvyLoL overlay setup",
                    "Enable LuvvyLoL in Privacy & Security > Accessibility, then return to the game.",
                );
                let _ = std::process::Command::new("open")
                    .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
                    .spawn();
            }
        }

        let (overlay_allowed, overlay_off) = {
            let s = state.lock().await;
            (
                cfg!(target_os = "macos") || s.status == ConnectionStatus::InGame,
                s.overlay_position == "off",
            )
        };

        if overlay_off {
            if was_visible {
                if let Some(window) = app_handle.get_webview_window("overlay") {
                    let _ = window.hide();
                }
                was_visible = false;
            }
            continue;
        }

        if !overlay_allowed {
            if was_visible {
                if let Some(window) = app_handle.get_webview_window("overlay") {
                    let _ = window.hide();
                }
                was_visible = false;
            }
            continue;
        }

        #[cfg(not(target_os = "macos"))]
        if device_state.is_none() {
            match std::panic::catch_unwind(DeviceState::new) {
                Ok(ds) => {
                    device_state = Some(ds);
                    log::info!("Overlay keyboard listener active");
                }
                Err(_) => {
                    log::warn!("Waiting for macOS Accessibility permission for the TAB overlay");
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        let keys = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            device_state.as_ref().map(DeviceState::get_keys).unwrap_or_default()
        })) {
            Ok(keys) => keys,
            Err(_) => {
                device_state = None;
                continue;
            }
        };
        #[cfg(not(target_os = "macos"))]
        let tab_pressed = keys.contains(&Keycode::Tab);
        #[cfg(target_os = "macos")]
        let tab_pressed = macos_tab_events.load(std::sync::atomic::Ordering::Relaxed)
            || macos_tab_key_pressed();

        if tab_pressed && !was_visible {
            if let Some(window) = app_handle.get_webview_window("overlay") {
                let _ = window.show();
                let _ = window.set_ignore_cursor_events(true);
                #[cfg(target_os = "macos")]
                let _ = configure_macos_overlay(&window, true);
            }
            was_visible = true;
        } else if !tab_pressed && was_visible {
            if let Some(window) = app_handle.get_webview_window("overlay") {
                let _ = window.hide();
            }
            was_visible = false;
        }
    }
}

#[cfg(target_os = "macos")]
fn start_macos_tab_monitor() -> Arc<std::sync::atomic::AtomicBool> {
    use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
    use core_graphics::event::{
        CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
        CGEventType, EventField, KeyCode,
    };
    use std::sync::atomic::{AtomicBool, Ordering};

    let tab_pressed = Arc::new(AtomicBool::new(false));
    let listener_state = Arc::clone(&tab_pressed);
    let spawn_result = std::thread::Builder::new()
        .name("querylol-macos-tab-listener".to_string())
        .spawn(move || loop {
            let callback_state = Arc::clone(&listener_state);
            let result = CGEventTap::new(
                CGEventTapLocation::Session,
                CGEventTapPlacement::HeadInsertEventTap,
                CGEventTapOptions::ListenOnly,
                vec![CGEventType::KeyDown, CGEventType::KeyUp],
                move |_proxy, event_type, event| {
                    if event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE)
                        == KeyCode::TAB as i64
                    {
                        match event_type {
                            CGEventType::KeyDown => callback_state.store(true, Ordering::Relaxed),
                            CGEventType::KeyUp => callback_state.store(false, Ordering::Relaxed),
                            _ => {}
                        }
                    }
                    None
                },
            );
            let tap_unavailable = result.is_err();

            if let Ok(tap) = result {
                let loop_source = tap
                    .mach_port
                    .create_runloop_source(0)
                    .expect("Failed to create macOS TAB run loop source");
                unsafe {
                    CFRunLoop::get_current().add_source(&loop_source, kCFRunLoopCommonModes);
                }
                tap.enable();
                CFRunLoop::run_current();
            }

            listener_state.store(false, Ordering::Relaxed);
            if tap_unavailable {
                log::warn!("macOS TAB event tap unavailable; retrying in two seconds");
            } else {
                log::warn!("macOS TAB event tap stopped; restarting in two seconds");
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        });

    if let Err(error) = spawn_result {
        log::error!("Failed to start macOS TAB listener: {}", error);
    }
    tab_pressed
}

#[cfg(target_os = "macos")]
fn macos_tab_key_pressed() -> bool {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn CGEventSourceKeyState(state_id: i32, key: u16) -> bool;
    }

    // Query both documented session sources because packaged apps can receive
    // different state depending on the active Space and input permissions.
    const COMBINED_SESSION_STATE: i32 = 0;
    const HID_SYSTEM_STATE: i32 = 1;
    const TAB_KEY_CODE: u16 = 0x30;
    unsafe {
        CGEventSourceKeyState(COMBINED_SESSION_STATE, TAB_KEY_CODE)
            || CGEventSourceKeyState(HID_SYSTEM_STATE, TAB_KEY_CODE)
    }
}

#[cfg(target_os = "macos")]
fn macos_accessibility_trusted() -> bool {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> u8;
    }
    unsafe { AXIsProcessTrusted() != 0 }
}

#[cfg(target_os = "macos")]
fn configure_macos_overlay(
    window: &tauri::WebviewWindow,
    bring_to_front: bool,
) -> Result<(), String> {
    use cocoa::appkit::{NSWindow, NSWindowCollectionBehavior};
    use cocoa::base::{id, NO};

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn CGShieldingWindowLevel() -> i32;
    }

    let native_window = window.clone();
    window
        .run_on_main_thread(move || {
            let ns_window = match native_window.ns_window() {
                Ok(ns_window) => ns_window as id,
                Err(error) => {
                    log::warn!("Failed to get native overlay window: {}", error);
                    return;
                }
            };
            unsafe {
                // League's borderless game window can sit above normal always-on-top
                // windows. Use the display shielding level and explicitly order the
                // overlay forward without activating it or stealing game focus.
                let behavior =
                    NSWindowCollectionBehavior::NSWindowCollectionBehaviorCanJoinAllSpaces
                        | NSWindowCollectionBehavior::NSWindowCollectionBehaviorTransient
                        | NSWindowCollectionBehavior::NSWindowCollectionBehaviorIgnoresCycle
                        | NSWindowCollectionBehavior::NSWindowCollectionBehaviorFullScreenAuxiliary;
                ns_window.setLevel_(CGShieldingWindowLevel() as i64);
                ns_window.setCollectionBehavior_(behavior);
                ns_window.setHidesOnDeactivate_(NO);
                if bring_to_front {
                    ns_window.orderFrontRegardless();
                }
            }
        })
        .map_err(|error| format!("Failed to configure overlay on the main thread: {}", error))?;
    Ok(())
}

pub fn run() {
    env_logger::init();

    let state: SharedState = Arc::new(Mutex::new(AppState::default()));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            get_state,
            set_auto_apply,
            set_region,
            apply_build_now,
            select_build_option,
            set_auto_lock,
            set_auto_accept,
            pick_champion,
            ban_champion,
            view_player_profile,
            view_match_details,
            get_match_details_preview,
            back_to_lobby,
            swap_aram_bench,
            set_overlay_position,
            test_overlay,
            set_tts_enabled,
            speak,
        ])
        .setup(|app| {
            // Load persisted preferences
            let cfg = config::load(app.handle());
            let overlay_position = cfg.overlay_position.clone();
            let state = app.state::<SharedState>();
            {
                let mut s = tauri::async_runtime::block_on(state.lock());
                s.region = cfg.region;
                s.auto_apply = cfg.auto_apply;
                s.auto_lock = cfg.auto_lock;
                s.auto_accept = cfg.auto_accept;
                s.tts_enabled = cfg.tts_enabled;
                s.overlay_position = cfg.overlay_position;
                // lp_history stays empty until the watcher learns the active
                // puuid; at that point we hydrate from the right bucket.
                s.lp_history = vec![];
            }

            if let Some(window) = app.get_webview_window("overlay") {
                let _ = window.set_ignore_cursor_events(true);
                #[cfg(target_os = "macos")]
                if let Err(e) = configure_macos_overlay(&window, false) {
                    log::warn!("Could not configure macOS full-screen overlay: {}", e);
                }
            }
            if let Err(e) = position_overlay_window(app.handle(), &overlay_position) {
                log::warn!("Could not position overlay: {}", e);
            }

            // Spawn auto-reconnect watcher
            let state_clone = Arc::clone(&*app.state::<SharedState>());
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                watcher_loop(state_clone, app_handle).await;
            });

            // Spawn overlay keyboard listener (TAB hold-to-show)
            let overlay_state = Arc::clone(&*app.state::<SharedState>());
            let overlay_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                overlay_loop(overlay_state, overlay_handle).await;
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_requested_summoners_rift_queues() {
        for queue_id in [400, 420, 440, 490] {
            assert!(is_supported_summoners_rift_queue(queue_id));
        }
        assert!(!is_supported_summoners_rift_queue(450));
    }

    #[test]
    fn normalizes_lcu_positions_without_guessing() {
        assert_eq!(map_position("MIDDLE"), "mid");
        assert_eq!(map_position("BOTTOM"), "adc");
        assert_eq!(map_position("UTILITY"), "support");
        assert_eq!(map_position("UNSELECTED"), "");
    }

    #[test]
    fn serializes_swiftplay_runes_in_lcu_page_shape() {
        let build = models::RuneBuild {
            primary_style_id: 8000,
            sub_style_id: 8300,
            selected_perk_ids: vec![8005, 9111, 9104, 8014, 8304, 8347, 5005, 5008, 5001],
        };
        let value: serde_json::Value = serde_json::from_str(&lcu::swiftplay_perks_string(&build)).unwrap();
        assert_eq!(value["primaryStyleId"], 8000);
        assert_eq!(value["subStyleId"], 8300);
        assert_eq!(value["selectedPerkIds"].as_array().unwrap().len(), 9);
    }
}
