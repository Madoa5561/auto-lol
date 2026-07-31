use crate::config::Settings;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Champion {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub alias: String,
    #[serde(default)]
    pub square_portrait_path: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChampSelectSession {
    pub actions: Vec<Vec<ChampSelectAction>>,
    #[serde(default)]
    pub bans: Bans,
    pub local_player_cell_id: i64,
    #[serde(default)]
    pub my_team: Vec<TeamMember>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChampSelectAction {
    pub id: i64,
    pub actor_cell_id: i64,
    #[serde(default)]
    pub champion_id: i64,
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub is_in_progress: bool,
    #[serde(rename = "type")]
    pub action_type: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bans {
    #[serde(default)]
    pub my_team_bans: Vec<i64>,
    #[serde(default)]
    pub their_team_bans: Vec<i64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamMember {
    pub cell_id: i64,
    #[serde(default)]
    pub assigned_position: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickDecision {
    pub action_id: i64,
    pub champion_id: i64,
    pub champion_name: String,
    pub assigned_position: String,
}

pub struct ChampionCatalog {
    by_name: HashMap<String, Champion>,
}

impl ChampionCatalog {
    pub fn new(champions: Vec<Champion>) -> Self {
        let mut by_name = HashMap::new();
        for champion in champions.into_iter().filter(|champion| champion.id > 0) {
            by_name.insert(normalize_name(&champion.name), champion.clone());
            if !champion.alias.is_empty() {
                by_name.insert(normalize_name(&champion.alias), champion);
            }
        }
        Self { by_name }
    }

    fn get(&self, name: &str) -> Option<&Champion> {
        self.by_name.get(&normalize_name(name))
    }
}

pub fn decide_pick(
    settings: &Settings,
    catalog: &ChampionCatalog,
    session: &ChampSelectSession,
) -> Option<PickDecision> {
    let local_member = session
        .my_team
        .iter()
        .find(|member| member.cell_id == session.local_player_cell_id)?;
    decide_action(
        settings.picks_for_position(&local_member.assigned_position),
        "pick",
        catalog,
        session,
        local_member,
    )
}

pub fn decide_ban(
    settings: &Settings,
    catalog: &ChampionCatalog,
    session: &ChampSelectSession,
) -> Option<PickDecision> {
    let local_member = session
        .my_team
        .iter()
        .find(|member| member.cell_id == session.local_player_cell_id)?;
    decide_action(
        settings.bans_for_position(&local_member.assigned_position),
        "ban",
        catalog,
        session,
        local_member,
    )
}

fn decide_action(
    candidates: &[String],
    action_type: &str,
    catalog: &ChampionCatalog,
    session: &ChampSelectSession,
    local_member: &TeamMember,
) -> Option<PickDecision> {
    if candidates.is_empty() {
        return None;
    }

    let active_action = session.actions.iter().flatten().find(|action| {
        action.actor_cell_id == session.local_player_cell_id
            && action.action_type == action_type
            && action.is_in_progress
            && !action.completed
    })?;

    let unavailable = unavailable_champions(session);
    candidates.iter().find_map(|candidate| {
        let champion = catalog.get(candidate)?;
        (!unavailable.contains(&champion.id)).then(|| PickDecision {
            action_id: active_action.id,
            champion_id: champion.id,
            champion_name: champion.name.clone(),
            assigned_position: local_member.assigned_position.clone(),
        })
    })
}

fn unavailable_champions(session: &ChampSelectSession) -> HashSet<i64> {
    let mut unavailable = HashSet::new();
    unavailable.extend(session.bans.my_team_bans.iter().copied());
    unavailable.extend(session.bans.their_team_bans.iter().copied());
    unavailable.extend(
        session
            .actions
            .iter()
            .flatten()
            .filter(|action| {
                action.completed
                    && matches!(action.action_type.as_str(), "pick" | "ban")
                    && action.champion_id > 0
            })
            .map(|action| action.champion_id),
    );
    unavailable
}

fn normalize_name(name: &str) -> String {
    name.chars()
        .flat_map(char::to_lowercase)
        .filter(|character| character.is_alphanumeric())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn champion(id: i64, name: &str, alias: &str) -> Champion {
        Champion {
            id,
            name: name.into(),
            alias: alias.into(),
            square_portrait_path: format!("/lol-game-data/assets/v1/champion-icons/{id}.png"),
        }
    }

    fn session() -> ChampSelectSession {
        ChampSelectSession {
            actions: vec![vec![ChampSelectAction {
                id: 77,
                actor_cell_id: 3,
                champion_id: 0,
                completed: false,
                is_in_progress: true,
                action_type: "pick".into(),
            }]],
            bans: Bans::default(),
            local_player_cell_id: 3,
            my_team: vec![TeamMember {
                cell_id: 3,
                assigned_position: "MIDDLE".into(),
            }],
        }
    }

    #[test]
    fn falls_back_when_first_candidate_is_banned() {
        let settings = Settings {
            middle: vec!["Ahri".into(), "Lux".into()],
            ..Settings::default()
        };
        let catalog = ChampionCatalog::new(vec![
            champion(103, "アーリ", "Ahri"),
            champion(99, "ラックス", "Lux"),
        ]);
        let mut session = session();
        session.bans.their_team_bans.push(103);

        let decision = decide_pick(&settings, &catalog, &session).unwrap();
        assert_eq!(decision.champion_id, 99);
        assert_eq!(decision.action_id, 77);
    }

    #[test]
    fn ignores_other_roles_and_inactive_actions() {
        let settings = Settings {
            top: vec!["Garen".into()],
            ..Settings::default()
        };
        let catalog = ChampionCatalog::new(vec![champion(86, "ガレン", "Garen")]);
        assert!(decide_pick(&settings, &catalog, &session()).is_none());

        let settings = Settings {
            middle: vec!["Ahri".into()],
            ..Settings::default()
        };
        let catalog = ChampionCatalog::new(vec![champion(103, "アーリ", "Ahri")]);
        let mut session = session();
        session.actions[0][0].is_in_progress = false;
        assert!(decide_pick(&settings, &catalog, &session).is_none());
    }

    #[test]
    fn keeps_active_hover_and_skips_completed_teammate_pick() {
        let settings = Settings {
            middle: vec!["Ahri".into(), "Lux".into()],
            ..Settings::default()
        };
        let catalog = ChampionCatalog::new(vec![
            champion(103, "アーリ", "Ahri"),
            champion(99, "ラックス", "Lux"),
        ]);
        let mut session = session();
        session.actions[0][0].champion_id = 103;

        let own_hover = decide_pick(&settings, &catalog, &session).unwrap();
        assert_eq!(own_hover.champion_id, 103);

        session.actions.push(vec![ChampSelectAction {
            id: 76,
            actor_cell_id: 2,
            champion_id: 103,
            completed: true,
            is_in_progress: false,
            action_type: "pick".into(),
        }]);
        let fallback = decide_pick(&settings, &catalog, &session).unwrap();
        assert_eq!(fallback.champion_id, 99);
    }

    #[test]
    fn chooses_role_specific_ban_fallback() {
        let settings = Settings {
            ban_middle: vec!["Zed".into(), "Yasuo".into()],
            ..Settings::default()
        };
        let catalog = ChampionCatalog::new(vec![
            champion(238, "ゼド", "Zed"),
            champion(157, "ヤスオ", "Yasuo"),
        ]);
        let mut session = session();
        session.actions[0][0].action_type = "ban".into();
        session.bans.my_team_bans.push(238);

        let decision = decide_ban(&settings, &catalog, &session).unwrap();
        assert_eq!(decision.champion_id, 157);
    }
}
