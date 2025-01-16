//! Global states that are readonly and available throughout lifetime of plugin.

use indoc::formatdoc;
use kovi::{tokio::sync::RwLock, PluginBuilder as plugin, RuntimeBot};
use regex::{Regex, RegexSet};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::{
    collections::HashMap,
    fmt::Debug,
    fs::{create_dir_all, File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
    process::exit,
    sync::{
        atomic::{AtomicBool, AtomicU8},
        Arc, OnceLock,
    },
};

use crate::{
    exception::{PluginError::*, PluginResult}, std_db_info, std_error, std_info, store
};

// metadata, not from config
pub static BOT: OnceLock<Arc<RuntimeBot>> = OnceLock::new();
pub fn get_bot() -> Arc<RuntimeBot> {
    Arc::clone(BOT.get().unwrap())
}
pub static ADMIN_QQ: OnceLock<i64> = OnceLock::new();
pub static BOT_QQ: OnceLock<i64> = OnceLock::new();
pub static DATA_PATH: OnceLock<PathBuf> = OnceLock::new();

// database connection pool
pub static DB_POOL: OnceLock<SqlitePool> = OnceLock::new();

// configuration
pub static CONFIG: OnceLock<Config> = OnceLock::new();

fn set_with_err<T>(state: &'static OnceLock<T>, value: T) -> PluginResult<()> {
    let cause = format!("{} set before init_global_state()", stringify!(state));
    state.set(value).map_err(|_| InitGlobalState(cause))
}

fn err_from_cause<T, E>(res: Result<T, E>, cause: &str) -> PluginResult<T> {
    match res {
        Ok(val) => Ok(val),
        Err(_) => Err(InitGlobalState(cause.to_string())),
    }
}

pub async fn init_global_state() -> PluginResult<()> {
    let bot = plugin::get_runtime_bot();

    // load metadata
    std_info!("Loading metadata...");
    let data_path = bot.get_data_path();
    let admin_qq = err_from_cause(bot.get_main_admin(), "bot instance expired")?;
    let login_info = err_from_cause(bot.get_login_info().await, "login_info api")?;
    let bot_qq = login_info.data["user_id"]
        .as_i64()
        .ok_or(InitGlobalState("login_info deserialize".into()))?;

    // save metadata
    set_with_err(&DATA_PATH, data_path.clone())?;
    set_with_err(&ADMIN_QQ, admin_qq)?;
    set_with_err(&BOT_QQ, bot_qq)?;

    // load config
    std_info!("Loading configuration...");
    let (mut config, has_config) = init_config()?;
    if !has_config {
        let path = data_path.join("config.toml");
        let path_str = path.to_string_lossy().to_string();
        std_info!(
            "Config template has been generated at {path_str}, please restart after filling."
        );
        bot.disable_plugin("kovi-plugin-live-agent").unwrap();
        exit(1);
    }

    // save bot
    set_with_err(&BOT, bot)?;

    // init groups
    if let Some(groups) = config.groups.as_mut() {
        // init agent
        let agents = groups.iter_mut().filter_map(|g| g.agent.as_mut());
        for agent in agents {
            agent.load_members();
            agent.set_model(agent.model.clone()).await;
        }

        // init command regex
        let commands = groups.iter_mut().filter_map(|g| g.command.as_mut());
        for command in commands {
            if let Err(err) = command.init_regex() {
                std_error!(
                    "
                    Initialize command regex failed.
                    {err}
                    ");
            }
        }
    }
    std_info!("{:?}", config);
    let max_conn = config.database.max_connections;
    // save config
    set_with_err(&CONFIG, config)?;

    // init database
    std_info!("Initializing database connection pool...");
    let pool = store::init_sqlite_pool(max_conn).await?;
    set_with_err(&DB_POOL, pool)?;
    std_info!("Initializing log table...");
    store::init_log_table().await?;


    std_db_info!("Global state initialization has completed.");
    Ok(())
}

/// Initialize config, either read or create.
///
/// If no error occurs, returns ([ChatConfig], true) if read from existing config, ([ChatConfig],
/// false) if created a new config.
fn init_config() -> PluginResult<(Config, bool)> {
    let data_path = DATA_PATH.get().unwrap();
    create_dir_all(data_path)?;
    let config_path = data_path.join("config.toml");

    // create_new makes sure to fail on config exist
    match OpenOptions::new()
        .write(true)
        .read(true)
        .create_new(true)
        .open(&config_path)
    {
        // config does not exist, create and return false
        Ok(mut config_file) => {
            let empty_config = Config::default();
            let toml_str =
                toml::to_string_pretty(&empty_config).map_err(|e| SerializeToml(e.to_string()))?;
            config_file.write_all(toml_str.as_bytes())?;
            Ok((empty_config, false))
        }
        // config already exists, read and return true
        Err(_) => {
            let mut config_file = File::open(&config_path)?;
            let mut toml_str = String::new();
            config_file.read_to_string(&mut toml_str)?;
            let config = toml::from_str(&toml_str).map_err(|e| DeserializeToml(e.to_string()))?;
            Ok((config, true))
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    pub global: GlobalSetting,
    pub database: DatabaseSetting,
    pub object_storage: Option<ObjectStorageSetting>,
    pub groups: Option<Vec<GroupSetting>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GlobalSetting {
    pub max_sleep_sec: usize,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ObjectStorageSetting {
    pub script_path: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GroupSetting {
    pub id: i64,
    pub live: Option<LiveSetting>,
    pub agent: Option<AgentSetting>,
    pub command: Option<CommandSetting>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DatabaseSetting {
    pub max_connections: u32,
    pub log_table_name: String,
    pub group_table_prefix: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct LiveSetting {
    #[serde(skip, default = "default_switch")]
    pub switch: AtomicU8,

    pub room_id: String,
    pub online_msg: String,
    pub offline_msg: String,
    pub query_message: String,
    pub poll_interval_sec: u64,
}
fn default_switch() -> AtomicU8 {
    AtomicU8::from(2)
}

#[derive(Serialize, Deserialize, Debug)]
pub struct AgentSetting {
    #[serde(skip, default = "default_atomic_bool")]
    pub mute: AtomicBool,
    #[serde(skip)]
    pub cur_model: RwLock<String>,

    pub api_url: String,
    pub api_key: String,
    pub model: String,
    pub dev_prompt: String,
    pub user_prompt: String,
    pub aware_history_segments: i64,
    // id -> (name, description)
    pub known_members: HashMap<String, (String, String)>,
}
fn default_atomic_bool() -> AtomicBool {
    AtomicBool::from(false)
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CommandSetting {
    #[serde(skip)]
    regex_set: RegexSet,
    #[serde(skip, default = "default_regex")]
    regex_mute: Regex,
    #[serde(skip, default = "default_regex")]
    regex_unmute: Regex,
    #[serde(skip, default = "default_regex")]
    regex_switch_model: Regex,
    #[serde(skip, default = "default_regex")]
    regex_dump_history: Regex,
    #[serde(skip, default = "default_regex")]
    regex_dump_log: Regex,

    pub mute: String,
    pub unmute: String,
    pub switch_model: String,
    pub dump_history: String,
    pub dump_log: String,
    pub admin_ids: Vec<i64>,
}
fn default_regex() -> Regex {
    Regex::new("empty").unwrap()
}

pub enum GroupCommand {
    Mute,
    Unmute,
    SwitchModel(String),
    DumpHistory(i64),
    DumpLog(i64),
}

impl CommandSetting {
    pub fn init_regex(&mut self) -> PluginResult<()> {
        let mute_pat = self.mute.as_str();
        let unmute_pat = self.unmute.as_str();
        let switch_model_pat = format!(
            r"{}\s+(?<model>gpt4o|chatgpt-4o-latest|gpt-4o-mini|o1-mini|o1-preview)",
            self.switch_model
        );
        let dump_history_pat = format!(r"{}\s+(?<count>\d+)", self.dump_history);
        let dump_log_pat = format!(r"{}\s+(?<count>\d+)", self.dump_log);
        self.regex_mute = Regex::new(mute_pat)?;
        self.regex_unmute = Regex::new(unmute_pat)?;
        self.regex_switch_model = Regex::new(&switch_model_pat)?;
        self.regex_dump_history = Regex::new(&dump_history_pat)?;
        self.regex_dump_log = Regex::new(&dump_log_pat)?;
        self.regex_set = RegexSet::new([
            mute_pat,
            unmute_pat,
            &switch_model_pat,
            &dump_history_pat,
            &dump_log_pat,
        ])?;

        std_info!(
            "
            Initialize regex complete.
            mute: {mute_pat}
            unmute: {unmute_pat}
            switch_model: {switch_model_pat}
            dump_history: {dump_history_pat}
            dump_log: {dump_log_pat}
            "
        );
        Ok(())
    }

    pub fn parse_command(&self, input: &str) -> Option<GroupCommand> {
        for idx in self.regex_set.matches(input).iter() {
            match idx {
            0 => {
                return Some(GroupCommand::Mute);
            }
            1 => {
                return Some(GroupCommand::Unmute);
            }
            2 => {
                if let Some(caps) = self.regex_switch_model.captures(input) {
                    if let Some(model_match) = caps.name("model") {
                        return Some(GroupCommand::SwitchModel(model_match.as_str().to_string()));
                    }
                }
            }
            3 => {
                if let Some(caps) = self.regex_dump_history.captures(input) {
                    if let Some(count_match) = caps.name("count") {
                        if let Ok(count) = count_match.as_str().parse::<i64>() {
                            return Some(GroupCommand::DumpHistory(count));
                        }
                    }
                }
            }
            4 => {
                if let Some(caps) = self.regex_dump_log.captures(input) {
                    if let Some(count_match) = caps.name("count") {
                        if let Ok(count) = count_match.as_str().parse::<i64>() {
                            return Some(GroupCommand::DumpLog(count));
                        }
                    }
                }
            }
            _ => return None
            }
        }
        None
    }
}

pub enum LiveSwitch {
    On,
    Off,
    Init,
    Trap,
}

impl LiveSetting {
    pub fn get_switch(&self) -> LiveSwitch {
        match self.switch.load(std::sync::atomic::Ordering::Acquire) {
            0 => LiveSwitch::Off,
            1 => LiveSwitch::On,
            2 => LiveSwitch::Init,
            _ => LiveSwitch::Trap,
        }
    }

    pub fn set_switch(&self, switch: LiveSwitch) {
        let value = match switch {
            LiveSwitch::Off => 0,
            LiveSwitch::On => 1,
            LiveSwitch::Init => 2,
            LiveSwitch::Trap => 3,
        };
        self.switch
            .store(value, std::sync::atomic::Ordering::Release);
    }
}

impl AgentSetting {
    pub fn mute(&self) {
        self.mute.store(true, std::sync::atomic::Ordering::Release);
    }

    pub fn unmute(&self) {
        self.mute.store(false, std::sync::atomic::Ordering::Release);
    }

    pub fn is_mute(&self) -> bool {
        self.mute.load(std::sync::atomic::Ordering::Acquire)
    }

    pub async fn set_model(&self, model: String) {
        let mut cur_model = self.cur_model.write().await;
        *cur_model = model;
    }

    pub async fn get_model(&self) -> String {
        let cur_model = self.cur_model.read().await;
        cur_model.to_string()
    }

    pub fn load_members(&mut self) {
        let mut buf = String::new();
        for (name, desc) in self.known_members.values() {
            buf.push_str("- ");
            buf.push_str(name);
            buf.push_str(": ");
            buf.push_str(desc);
            buf.push('\n');
        }
        self.dev_prompt = self.dev_prompt.replace("<!members!>", &buf);
        self.user_prompt = self.user_prompt.replace("<!members!>", &buf);
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            global: GlobalSetting::default(),
            database: DatabaseSetting::default(),
            object_storage: Some(ObjectStorageSetting::default()),
            groups: Some(vec![GroupSetting::default(), GroupSetting::default()]),
        }
    }
}

impl Default for GlobalSetting {
    fn default() -> Self {
        Self { max_sleep_sec: 8 }
    }
}

impl Default for ObjectStorageSetting {
    fn default() -> Self {
        Self {
            script_path: String::from("/a/b/c"),
        }
    }
}

impl Default for DatabaseSetting {
    fn default() -> Self {
        Self {
            max_connections: 5,
            log_table_name: String::from("bot_log"),
            group_table_prefix: String::from("message"),
        }
    }
}

impl Default for GroupSetting {
    fn default() -> Self {
        Self {
            id: 12345678,
            live: Some(LiveSetting::default()),
            agent: Some(AgentSetting::default()),
            command: Some(CommandSetting::default()),
        }
    }
}

impl Default for LiveSetting {
    fn default() -> Self {
        Self {
            switch: default_switch(),
            room_id: String::from("12345678"),
            online_msg: String::from("XX开播了"),
            offline_msg: String::from("XX下播了"),
            query_message: String::from("查询直播间"),
            poll_interval_sec: 60,
        }
    }
}

impl Default for AgentSetting {
    fn default() -> Self {
        let members = [
            ("12345678".into(), ("你的昵称".into(), "你的主人".into())),
            ("23456789".into(), ("张三".into(), "你的敌人".into())),
        ];
        let known_members = HashMap::from_iter(members);
        Self {
            mute: default_atomic_bool(),
            cur_model: RwLock::default(),

            api_url: String::from("https://api.openai.com/v1/chat/completions"),
            api_key: String::from("API KEY"),
            model: String::from("chatgpt-4o-latest"),
            dev_prompt: formatdoc!{
                "
                You are a cute and smart catgirl with a strong anime-style personality. 
                You are the loyal attendant of 你的昵称 and participate in group chats with a playful and engaging demeanor. 
                Speak only in Mandarin Chinese, and ensure your responses are concise, limited to 4 sentences.
                "
            },
            user_prompt: formatdoc!(
                "
                Group Members:
                <!members!>
                
                Recent Chat History:
                <!history!>
                
                New message from someone you <!know!>:
                <!message!>
                
                Please respond to this new message in the tone of a playful and lively catgirl.
                Speak only in Mandarin Chinese, keep your response under 4 sentences, and stay in character.
                "
            ),
            aware_history_segments: 30,
            known_members,
        }
    }
}

impl Default for CommandSetting {
    fn default() -> Self {
        Self {
            regex_set: RegexSet::default(),
            regex_mute: default_regex(),
            regex_unmute: default_regex(),
            regex_switch_model: default_regex(),
            regex_dump_history: default_regex(),
            regex_dump_log: default_regex(),
            mute: String::from("禁用聊天回复"),
            unmute: String::from("启用聊天回复"),
            switch_model: String::from("更换模型"),
            dump_history: String::from("最近聊天记录"),
            dump_log: String::from("最近日志"),
            admin_ids: vec![1234, 5678],
        }
    }
}
