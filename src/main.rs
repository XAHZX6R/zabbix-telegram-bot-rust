use std::{collections::HashMap, collections::HashSet, env, fs, path::PathBuf, sync::Arc};

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use reqwest::Client;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use teloxide::{
    prelude::*,
    types::Me,
    utils::command::BotCommands,
};
use tokio::sync::RwLock;
use tracing::{info, warn, error};

#[derive(Parser, Debug)]
#[command(name = "zabbixbot", version, about = "Zabbix ↔ Telegram bot and setup utility")]
struct Cli {
    /// Subcommand. If omitted, runs the Telegram bot.
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Configure Zabbix using JSON-RPC API (media + action)
    ZbxSetup,
}

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Доступные команды:")]
enum Command {
    /// Показать это сообщение
    Help,
    /// Проверка доступа
    Start,
    /// Показать ваш Telegram ID
    Id,
}

struct AppState {
    allowed_users: RwLock<HashSet<i64>>, // хранится под Arc сверху
}

fn read_allowed_users(path: &PathBuf) -> Result<HashSet<i64>> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            warn!(path = %path.display(), "Файл allowed_users не найден, продолжаю с пустым списком");
            return Ok(HashSet::new());
        }
        Err(e) => {
            return Err(e).with_context(|| format!("Не удалось прочитать файл allowed_users: {}", path.display()));
        }
    };
    let mut set = HashSet::new();
    for (i, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        match line.parse::<i64>() {
            Ok(id) => { set.insert(id); },
            Err(_) => {
                warn!(line = %line, row = i + 1, "Не удалось преобразовать строку в i64. Пропускаю...");
            }
        }
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    async fn test_read_allowed_users_parses_ids_and_skips_comments() {
        let mut fpath = std::env::temp_dir();
        fpath.push(format!("allowed_users_test_{}.txt", std::process::id()));
        let mut file = std::fs::File::create(&fpath).unwrap();
        writeln!(file, "# comment").unwrap();
        writeln!(file, "  12345  ").unwrap();
        writeln!(file, "").unwrap();
        writeln!(file, "notanumber").unwrap();
        writeln!(file, "67890").unwrap();
        drop(file);

        let set = read_allowed_users(&fpath).unwrap();
        assert!(set.contains(&12345));
        assert!(set.contains(&67890));
        assert_eq!(set.len(), 2);

        std::fs::remove_file(&fpath).ok();
    }
}

#[derive(Serialize)]
struct RpcRequest<'a, T> {
    jsonrpc: &'static str,
    method: &'a str,
    params: T,
    id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth: Option<String>,
}

#[derive(Deserialize, Debug)]
struct RpcResponse {
    jsonrpc: String,
    #[serde(default)]
    result: Option<serde_json::Value>,
    #[serde(default)]
    error: Option<RpcError>,
    id: u64,
}

#[derive(Deserialize, Debug)]
struct RpcError {
    code: i64,
    message: String,
    data: Option<serde_json::Value>,
}

struct ZbxClient {
    url: String,
    client: Client,
    auth: Option<String>,
}

impl ZbxClient {
    fn new(url: String) -> Self { Self { url, client: Client::builder().build().unwrap(), auth: None } }

    async fn rpc<T: Serialize, R: DeserializeOwned>(&self, method: &str, params: &T) -> Result<R> {
        let req = RpcRequest { jsonrpc: "2.0", method, params, id: 1u64, auth: self.auth.clone() };
        let resp = self.client.post(&self.url).json(&req).send().await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(anyhow!("Zabbix API HTTP error: {}: {}", status, body));
        }
        let parsed: RpcResponse = serde_json::from_str(&body)
            .with_context(|| format!("Unable to parse JSON-RPC response: {}", body))?;
        if let Some(err) = parsed.error {
            return Err(anyhow!("Zabbix API error {}: {} {:?}", err.code, err.message, err.data));
        }
        let val = parsed.result.context("Missing result in JSON-RPC response")?;
        let res: R = serde_json::from_value(val)
            .with_context(|| "Unable to deserialize JSON-RPC result to expected type")?;
        Ok(res)
    }

    async fn login(&mut self, user: &str, password: &str) -> Result<()> {
        // Zabbix 6.4+ expects {"username","password"}; older accepts {"user","password"}
        #[derive(Serialize)]
        struct ParamsNew<'a> { username: &'a str, password: &'a str }
        #[derive(Serialize)]
        struct ParamsOld<'a> { user: &'a str, password: &'a str }

        // Try new first
        let try_new: Result<String> = self.rpc("user.login", &ParamsNew { username: user, password }).await;
        match try_new {
            Ok(token) => { self.auth = Some(token); return Ok(()); }
            Err(e) => {
                let msg = format!("{}", e);
                if msg.contains("Invalid params") || msg.contains("unexpected parameter \"username\"") {
                    // Fallback to old
                    let token: String = self.rpc("user.login", &ParamsOld { user, password }).await?;
                    self.auth = Some(token);
                    return Ok(());
                } else {
                    return Err(e);
                }
            }
        }
    }
}

#[derive(Deserialize, Debug)]
struct MediaType {
    mediatypeid: String,
    name: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    parameters: Option<Vec<HashMap<String, String>>>,
}

#[derive(Deserialize, Debug)]
struct UserShort { userid: String, alias: Option<String>, name: Option<String> }

#[derive(Deserialize, Debug, Clone, Serialize)]
struct UserMedia {
    mediatypeid: String,
    sendto: String,
    active: String,      // 0: enabled
    severity: String,    // 63: all
    period: String,      // e.g., 1-7,00:00-24:00
}

async fn zbx_setup() -> Result<()> {
    // Читаем конфиг из окружения
    let url = env::var("ZBX_API_URL").context("ZBX_API_URL is required, e.g. http://zabbix.local/zabbix/api_jsonrpc.php")?;
    let user = env::var("ZBX_USER").unwrap_or_else(|_| "Admin".to_string());
    let pass = env::var("ZBX_PASSWORD").context("ZBX_PASSWORD is required")?;
    let user_alias = env::var("ZBX_USER_ALIAS").unwrap_or_else(|_| "Admin".to_string());
    let chat_id = env::var("ZBX_CHAT_ID").context("ZBX_CHAT_ID is required (e.g., 1349552926)")?;
    let action_name = env::var("ZBX_ACTION_NAME").unwrap_or_else(|_| "Send Telegram alerts".to_string());

    let mut zbx = ZbxClient::new(url);
    zbx.login(&user, &pass).await?;
    info!("Logged in to Zabbix API as {}", user);

    // Найдём медиа тип Telegram
    #[derive(Serialize)]
    struct MtGetParams<'a> {
        output: &'a [&'a str],
        filter: HashMap<&'a str, &'a str>,
    }
    let mut filter = HashMap::new(); filter.insert("name", "Telegram");
    let mtypes: Vec<MediaType> = zbx.rpc("mediatype.get", &MtGetParams { output: &["mediatypeid", "name", "parameters", "status"], filter }).await?;
    let mt = mtypes.first().ok_or_else(|| anyhow!("Media type 'Telegram' not found in Zabbix"))?;
    let mediatypeid = mt.mediatypeid.clone();
    info!(mediatypeid = %mediatypeid, "Found media type 'Telegram'");

    // Обновим токен бота в параметрах media type, если есть соответствующий параметр
    if let Ok(bot_token) = env::var("TELEGRAM_BOT_TOKEN")
        .or_else(|_| env::var("ZBX_BOT_TOKEN"))
    {
        if let Some(params) = mt.parameters.clone() {
            let mut needs_update = false;
            let mut updated_params: Vec<HashMap<String, String>> = Vec::with_capacity(params.len());
            for mut p in params.into_iter() {
                if let Some(name) = p.get("name").cloned() {
                    let lname = name.to_lowercase();
                    if lname == "token" || lname == "bottoken" {
                        if p.get("value").map(|v| v != &bot_token).unwrap_or(true) {
                            p.insert("value".into(), bot_token.clone());
                            needs_update = true;
                        }
                    }
                }
                updated_params.push(p);
            }
            if needs_update {
                #[derive(Serialize)]
                struct MtUpdateParams { mediatypeid: String, parameters: Vec<HashMap<String, String>> }
                let _upd: serde_json::Value = zbx.rpc("mediatype.update", &MtUpdateParams { mediatypeid: mediatypeid.clone(), parameters: updated_params }).await?;
                info!("Updated Telegram media type token via mediatype.update");
            } else {
                info!("Telegram media type token parameter not changed or not present — skipping update");
            }
        } else {
            warn!("Telegram media type has no parameters array — cannot set token automatically");
        }
    } else {
        warn!("No TELEGRAM_BOT_TOKEN or ZBX_BOT_TOKEN in env — skipping media type token update");
    }

    // Получим пользователя по alias
    #[derive(Serialize)]
    struct UserGetParams<'a> { output: &'a [&'a str], filter: HashMap<&'a str, String>, selectMedias: &'a str }
    let mut ufilter = HashMap::new(); ufilter.insert("alias", user_alias.clone());
    let users: Vec<serde_json::Value> = zbx.rpc("user.get", &UserGetParams { output: &["userid", "alias", "name"], filter: ufilter, selectMedias: "extend" }).await?;
    let u = users.first().ok_or_else(|| anyhow!("User with alias '{}' not found", user_alias))?;
    let userid = u.get("userid").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("userid missing in user.get"))?.to_string();
    let mut medias: Vec<UserMedia> = u.get("medias")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|m| serde_json::from_value::<UserMedia>(m.clone()).ok()).collect())
        .unwrap_or_default();

    let exists = medias.iter().any(|m| m.mediatypeid == mediatypeid && m.sendto == chat_id);
    if !exists {
        medias.push(UserMedia { mediatypeid: mediatypeid.clone(), sendto: chat_id.clone(), active: "0".into(), severity: "63".into(), period: "1-7,00:00-24:00".into() });

        #[derive(Serialize)]
        struct UserUpdateParams { userid: String, medias: Vec<UserMedia> }
        let _update: serde_json::Value = zbx.rpc("user.update", &UserUpdateParams { userid: userid.clone(), medias: medias.clone() }).await?;
        info!(userid = %userid, chat_id = %chat_id, "Attached Telegram media to user");
    } else {
        info!(userid = %userid, chat_id = %chat_id, "Telegram media already attached to user");
    }

    // Проверим/создадим Action
    #[derive(Serialize)]
    struct ActionGetParams<'a> { output: &'a [&'a str], filter: HashMap<&'a str, String> }
    let mut afilter = HashMap::new(); afilter.insert("name", action_name.clone());
    let actions: Vec<serde_json::Value> = zbx.rpc("action.get", &ActionGetParams { output: &["actionid", "name"], filter: afilter }).await?;
    if actions.is_empty() {
        #[derive(Serialize)]
        struct OpMessage { default_msg: i32, mediatypeid: String, subject: String, message: String }
        #[derive(Serialize)]
        struct OpUser { userid: String }
        #[derive(Serialize)]
        struct Operation { operationtype: i32, opmessage: OpMessage, opmessage_usr: Vec<OpUser> }
        #[derive(Serialize)]
        struct ActionCreate {
            name: String,
            eventsource: i32, // 0: triggers
            status: i32,      // 0: enabled
            operations: Vec<Operation>,
        }
        let op = Operation {
            operationtype: 0,
            opmessage: OpMessage {
                default_msg: 0,
                mediatypeid: mediatypeid.clone(),
                subject: "{HOST.NAME} | Problem: {EVENT.NAME}".into(),
                message: "Problem started at {EVENT.TIME} on {EVENT.DATE}\nProblem name: {EVENT.NAME}\nHost: {HOST.NAME}\nSeverity: {TRIGGER.SEVERITY}\nOriginal problem ID: #{EVENT.ID}\n{TRIGGER.URL}".into(),
            },
            opmessage_usr: vec![OpUser { userid: userid.clone() }],
        };
        let payload = ActionCreate { name: action_name.clone(), eventsource: 0, status: 0, operations: vec![op] };
        let created: serde_json::Value = zbx.rpc("action.create", &payload).await?;
        info!(?created, "Created action");
    } else {
        let aid = actions[0].get("actionid").and_then(|v| v.as_str()).unwrap_or("");
        info!(actionid = aid, "Action already exists");
    }

    info!("Zabbix setup completed");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    // Логи
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .init();

    // Конфигурация
    dotenvy::dotenv().ok(); // подхватить .env, если есть

    // Проверка режима запуска: CLI субкоманда или RUN_MODE
    let cli = Cli::parse();
    let run_mode_env = env::var("RUN_MODE").unwrap_or_default();
    match (cli.command, run_mode_env.as_str()) {
        (Some(Commands::ZbxSetup), _) | (None, "zbx-setup") => {
            return zbx_setup().await;
        }
        _ => {
            // Бот по умолчанию
        }
    }

    let token = env::var("TELEGRAM_BOT_TOKEN")
        .context("Переменная окружения TELEGRAM_BOT_TOKEN не задана")?;

    let allowed_path = env::var("ALLOWED_USERS_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/bot/allowed_users.txt"));

    let allowed_users = read_allowed_users(&allowed_path)?;
    info!(count = allowed_users.len(), path = %allowed_path.display(), "Список разрешенных пользователей загружен");

    let state = Arc::new(AppState { allowed_users: RwLock::new(allowed_users) });

    let bot = Bot::new(token);
    let me: Me = bot.get_me().await?;
    info!(username = %me.username(), id = %me.id, "Бот запущен");

    // Роутинг команд
    let handler = Update::filter_message()
        .branch(dptree::entry()
            .filter_command::<Command>()
            .endpoint(handle_command))
        .branch(dptree::endpoint(handle_message));

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![state.clone()])
        .enable_ctrlc_handler()
        .default_handler(|upd| async move {
            warn!(?upd, "Необработанное событие");
        })
        .error_handler(LoggingErrorHandler::with_custom_text("Ошибка в диспетчере"))
        .build()
        .dispatch()
        .await;

    Ok(())
}

async fn is_authorized(state: &AppState, user_id: i64) -> bool {
    let guard = state.allowed_users.read().await;
    guard.contains(&user_id)
}

async fn handle_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
    state: Arc<AppState>,
) -> Result<()> {
    let user_id = msg.from().map(|u| u.id.0 as i64);
    match (cmd, user_id) {
        (Command::Help, _) => {
            bot.send_message(msg.chat.id, Command::descriptions().to_string()).await?;
        }
        (Command::Start, Some(uid)) => {
            if !is_authorized(&state, uid).await {
                warn!(user_id = uid, "Неавторизованный пользователь. Игнорирую...");
                bot.send_message(msg.chat.id, "Access denied").await?;
            } else {
                info!(user_id = uid, "Авторизованный пользователь");
                bot.send_message(msg.chat.id, "Login successful").await?;
            }
        }
        (Command::Id, Some(uid)) => {
            bot.send_message(msg.chat.id, format!("Ваш Telegram ID: {}", uid)).await?;
        }
        (_, None) => {
            warn!("Сообщение без поля from");
        }
    }
    Ok(())
}

async fn handle_message(
    bot: Bot,
    msg: Message,
    state: Arc<AppState>,
) -> Result<()> {
    let uid = match msg.from() { Some(u) => u.id.0 as i64, None => { return Ok(()); } };
    if !is_authorized(&state, uid).await {
        warn!(user_id = uid, "Неавторизованный пользователь. Игнорирую...");
        bot.send_message(msg.chat.id, "Access denied").await.ok();
        return Ok(());
    }

    // Экономно: просто отвечаем подсказкой на любое сообщение
    bot.send_message(msg.chat.id, "Используйте /start для проверки доступа или /id для получения вашего ID").await?;
    Ok(())
}
