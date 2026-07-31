use crate::core::{ChampSelectSession, Champion, ChampionCatalog};
use std::ffi::c_void;
use std::fs;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Networking::WinHttp::{
    SECURITY_FLAG_IGNORE_CERT_CN_INVALID, SECURITY_FLAG_IGNORE_CERT_DATE_INVALID,
    SECURITY_FLAG_IGNORE_CERT_WRONG_USAGE, SECURITY_FLAG_IGNORE_UNKNOWN_CA,
    WINHTTP_ACCESS_TYPE_NO_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_OPTION_SECURITY_FLAGS,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect,
    WinHttpOpen, WinHttpOpenRequest, WinHttpQueryDataAvailable, WinHttpQueryHeaders,
    WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetOption,
};

pub struct LcuClient {
    port: u16,
    password: String,
}

pub struct LcuResponse {
    pub status: u32,
    pub body: Vec<u8>,
}

impl LcuClient {
    pub fn discover() -> Result<Self, String> {
        let lockfile =
            discover_lockfile().ok_or_else(|| "Leagueクライアントを待機中".to_owned())?;
        let content = fs::read_to_string(&lockfile)
            .map_err(|_| "League lockfileを読み取れません".to_owned())?;
        let parts: Vec<&str> = content.trim().split(':').collect();
        if parts.len() < 5 {
            return Err("League lockfileの形式が不正です".into());
        }
        let port = parts[2]
            .parse::<u16>()
            .map_err(|_| "Leagueのローカルポートが不正です".to_owned())?;
        Ok(Self {
            port,
            password: parts[3].to_owned(),
        })
    }

    pub fn phase(&self) -> Result<String, String> {
        let response = self.request("GET", "/lol-gameflow/v1/gameflow-phase", None)?;
        if response.status != 200 {
            return Err(format!("ゲーム状態の取得に失敗 ({})", response.status));
        }
        serde_json::from_slice(&response.body).map_err(|error| error.to_string())
    }

    pub fn accept_ready_check(&self) -> Result<bool, String> {
        let response = self.request("POST", "/lol-matchmaking/v1/ready-check/accept", Some(""))?;
        Ok((200..300).contains(&response.status))
    }

    pub fn champion_catalog(&self) -> Result<ChampionCatalog, String> {
        self.champions().map(ChampionCatalog::new)
    }

    pub fn champions(&self) -> Result<Vec<Champion>, String> {
        let response = self.request(
            "GET",
            "/lol-game-data/assets/v1/champion-summary.json",
            None,
        )?;
        if response.status != 200 {
            return Err(format!(
                "チャンピオン一覧の取得に失敗 ({})",
                response.status
            ));
        }
        serde_json::from_slice(&response.body).map_err(|error| error.to_string())
    }

    pub fn asset_bytes(&self, path: &str) -> Result<Vec<u8>, String> {
        if !path.starts_with("/lol-game-data/assets/") {
            return Err("画像パスが不正です".into());
        }
        let response = self.request("GET", path, None)?;
        if response.status != 200 {
            return Err(format!(
                "チャンピオン画像の取得に失敗 ({})",
                response.status
            ));
        }
        Ok(response.body)
    }

    pub fn champ_select_session(&self) -> Result<Option<ChampSelectSession>, String> {
        let response = self.request("GET", "/lol-champ-select/v1/session", None)?;
        if response.status == 404 {
            return Ok(None);
        }
        if response.status != 200 {
            return Err(format!(
                "チャンピオン選択状態の取得に失敗 ({})",
                response.status
            ));
        }
        serde_json::from_slice(&response.body)
            .map(Some)
            .map_err(|error| error.to_string())
    }

    pub fn complete_champion_action(&self, action_id: i64, champion_id: i64) -> Result<(), String> {
        let path = format!("/lol-champ-select/v1/session/actions/{action_id}");
        let body = champion_action_body(champion_id);
        let response = self.request("PATCH", &path, Some(&body))?;
        if (200..300).contains(&response.status) {
            Ok(())
        } else {
            Err(format!("自動選択・確定に失敗 ({})", response.status))
        }
    }

    fn request(&self, method: &str, path: &str, body: Option<&str>) -> Result<LcuResponse, String> {
        unsafe {
            let user_agent = wide("LanePilot/0.1");
            let session = WinHttpOpen(
                user_agent.as_ptr(),
                WINHTTP_ACCESS_TYPE_NO_PROXY,
                null(),
                null(),
                0,
            );
            if session.is_null() {
                return Err("WinHTTPセッションを開始できません".into());
            }
            let _session = InternetHandle(session);

            let host = wide("127.0.0.1");
            let connection = WinHttpConnect(session, host.as_ptr(), self.port, 0);
            if connection.is_null() {
                return Err("Leagueクライアントへ接続できません".into());
            }
            let _connection = InternetHandle(connection);

            let method = wide(method);
            let path = wide(path);
            let request = WinHttpOpenRequest(
                connection,
                method.as_ptr(),
                path.as_ptr(),
                null(),
                null(),
                null(),
                WINHTTP_FLAG_SECURE,
            );
            if request.is_null() {
                return Err("Leagueへのリクエストを作成できません".into());
            }
            let _request = InternetHandle(request);

            let security_flags = SECURITY_FLAG_IGNORE_UNKNOWN_CA
                | SECURITY_FLAG_IGNORE_CERT_CN_INVALID
                | SECURITY_FLAG_IGNORE_CERT_DATE_INVALID
                | SECURITY_FLAG_IGNORE_CERT_WRONG_USAGE;
            if WinHttpSetOption(
                request,
                WINHTTP_OPTION_SECURITY_FLAGS,
                &security_flags as *const u32 as *mut c_void,
                size_of::<u32>() as u32,
            ) == 0
            {
                return Err("Leagueのローカル証明書を設定できません".into());
            }

            let credentials = base64_encode(format!("riot:{}", self.password).as_bytes());
            let headers = wide(&format!(
                "Authorization: Basic {credentials}\r\nContent-Type: application/json\r\nAccept: application/json\r\n"
            ));
            let body_bytes = body.unwrap_or_default().as_bytes();
            let (body_pointer, body_length) = if body.is_some() {
                (body_bytes.as_ptr() as *mut c_void, body_bytes.len() as u32)
            } else {
                (null_mut(), 0)
            };

            if WinHttpSendRequest(
                request,
                headers.as_ptr(),
                (headers.len() - 1) as u32,
                body_pointer,
                body_length,
                body_length,
                0,
            ) == 0
            {
                return Err("Leagueへのリクエスト送信に失敗しました".into());
            }
            if WinHttpReceiveResponse(request, null_mut()) == 0 {
                return Err("Leagueから応答を受信できません".into());
            }

            let mut status = 0u32;
            let mut status_size = size_of::<u32>() as u32;
            if WinHttpQueryHeaders(
                request,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                null(),
                &mut status as *mut u32 as *mut c_void,
                &mut status_size,
                null_mut(),
            ) == 0
            {
                return Err("LeagueのHTTP状態を取得できません".into());
            }

            let mut bytes = Vec::new();
            loop {
                let mut available = 0u32;
                if WinHttpQueryDataAvailable(request, &mut available) == 0 {
                    return Err("Leagueの応答サイズを取得できません".into());
                }
                if available == 0 {
                    break;
                }
                let start = bytes.len();
                bytes.resize(start + available as usize, 0);
                let mut read = 0u32;
                if WinHttpReadData(
                    request,
                    bytes[start..].as_mut_ptr() as *mut c_void,
                    available,
                    &mut read,
                ) == 0
                {
                    return Err("Leagueの応答を読み取れません".into());
                }
                bytes.truncate(start + read as usize);
            }

            Ok(LcuResponse {
                status,
                body: bytes,
            })
        }
    }
}

fn champion_action_body(champion_id: i64) -> String {
    format!(r#"{{"championId":{champion_id},"completed":true}}"#)
}

struct InternetHandle(*mut c_void);

impl Drop for InternetHandle {
    fn drop(&mut self) {
        unsafe {
            WinHttpCloseHandle(self.0);
        }
    }
}

fn discover_lockfile() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = install_path_from_metadata() {
        candidates.push(path.join("lockfile"));
    }
    for drive in ["C:", "D:", "E:", "F:"] {
        candidates.push(
            Path::new(drive)
                .join("Riot Games")
                .join("League of Legends")
                .join("lockfile"),
        );
    }
    candidates.into_iter().find(|path| path.is_file())
}

fn install_path_from_metadata() -> Option<PathBuf> {
    let program_data = std::env::var_os("PROGRAMDATA")?;
    let path = PathBuf::from(program_data)
        .join("Riot Games")
        .join("Metadata")
        .join("league_of_legends.live")
        .join("league_of_legends.live.product_settings.yaml");
    let yaml = fs::read_to_string(path).ok()?;
    let value = yaml
        .lines()
        .find_map(|line| line.trim().strip_prefix("product_install_full_path:"))?
        .trim()
        .trim_matches(['\'', '"'])
        .replace('/', "\\");
    Some(PathBuf::from(value))
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        encoded.push(TABLE[(first >> 2) as usize] as char);
        encoded.push(TABLE[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
        if chunk.len() > 1 {
            encoded.push(TABLE[(((second & 0x0f) << 2) | (third >> 6)) as usize] as char);
        } else {
            encoded.push('=');
        }
        if chunk.len() > 2 {
            encoded.push(TABLE[(third & 0x3f) as usize] as char);
        } else {
            encoded.push('=');
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completes_champion_action_in_single_patch() {
        assert_eq!(
            champion_action_body(103),
            r#"{"championId":103,"completed":true}"#
        );
    }

    #[test]
    fn encodes_basic_auth_value() {
        assert_eq!(base64_encode(b"riot:password"), "cmlvdDpwYXNzd29yZA==");
    }

    #[test]
    #[ignore = "起動中のLeagueクライアントが必要"]
    fn connects_to_running_client() {
        let client = LcuClient::discover().expect("Leagueクライアントを検出できません");
        let phase = client.phase().expect("ゲーム状態を取得できません");
        assert!(!phase.is_empty());
        let champions = client
            .champions()
            .expect("チャンピオン一覧を取得できません");
        let champion = champions
            .into_iter()
            .find(|champion| champion.id > 0)
            .expect("有効なチャンピオンがありません");
        let icon = client
            .asset_bytes(&champion.square_portrait_path)
            .expect("チャンピオン画像を取得できません");
        assert!(icon.starts_with(b"\x89PNG"));
    }
}
