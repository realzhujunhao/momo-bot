//! High level abstractions

use kovi::{
    tokio::time::{interval, sleep},
    Message,
};
use rand::{thread_rng, Rng};
use serde::{Deserialize, Serialize};
use std::{future::Future, path::PathBuf, time::Duration};
use time::{
    macros::{format_description, offset},
    OffsetDateTime,
};

use crate::{
    db_warn, exception::PluginResult, global_state, std_db_error, std_info, store, BOT_QQ, CONFIG,
};

/// Schedule a periodic task that blocks current task forever.
pub async fn schedule_task_blocking<F, Fut>(duration: Duration, mut task: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = ()>,
{
    let mut timer = interval(duration);
    loop {
        timer.tick().await;
        task().await;
    }
}

pub async fn sleep_rand_time() {
    let config = CONFIG.get().unwrap();
    let max_sleep_sec = config.global.max_sleep_sec as f64;
    let rand_time = {
        let mut rng = thread_rng();
        rng.gen_range(0.0..max_sleep_sec)
    };
    std_info!("Sleep {rand_time} seconds.");
    sleep(Duration::from_secs_f64(rand_time)).await;
}

pub async fn extract_text(msg: &Message) -> String {
    let text_segs = msg.get("text");
    let mut buf = String::new();
    for seg in text_segs {
        let Ok(content) = serde_json::from_value::<String>(seg.data["text"].clone()) else {
            std_db_error!(
                "
                Extract text: data object inside text segment has no text field
                Data: {}
                ",
                seg.data
            );
            return buf;
        };
        buf.push_str(&content);
        buf.push('\n');
    }
    buf
}

pub async fn extract_segments<T>(msg: T) -> Vec<(String, String)>
where
    T: Into<Message>,
{
    let message = msg.into();
    let len = message.iter().len();
    let mut list = Vec::with_capacity(len);
    for seg in message.iter() {
        let seg_type = seg.type_.clone();
        let content: Option<String> = match seg_type.as_str() {
            "text" => serde_json::from_value(seg.data["text"].clone()).ok(),
            "image" | "record" | "video" => serde_json::from_value(seg.data["file"].clone()).ok(),
            "at" => serde_json::from_value(seg.data["qq"].clone()).ok(),
            "share" => serde_json::from_value(seg.data["url"].clone()).ok(),
            "contact" | "reply" | "forward" | "node" => {
                serde_json::from_value(seg.data["id"].clone()).ok()
            }
            _ => None,
        };
        let Some(content) = content else {
            db_warn!(
                "
                Skip extract segment that is not pre-defined: 
                Data: {}
                ",
                seg.data
            );
            continue;
        };
        list.push((seg_type, content));
    }
    list
}

/// Obtain "[year-month-day hour:minute:second]".
pub fn cur_time_iso8601() -> String {
    let offset = offset!(+8);
    let datetime = OffsetDateTime::now_utc().to_offset(offset);
    let desc = format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");
    datetime.format(desc).unwrap()
}

/// Convert unix timestamp to "[year-month-day hour:minute:second]".  
/// This may fail if the timestamp passed in is before 1970.
pub fn iso8601_from_timestamp(timestamp: i64) -> PluginResult<String> {
    let offset = offset!(+8);
    let datetime = OffsetDateTime::from_unix_timestamp(timestamp)?.to_offset(offset);
    let desc = format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");
    Ok(datetime.format(desc)?)
}

/// There is a one-way conversion from timestamp to iso8601, because this plugin exclusively uses
/// iso8601.
pub enum TimeRepr {
    Iso8601(String),
    UnixTimeStamp(i64),
}

impl TimeRepr {
    /// Silently log time error and return None on failure.
    pub async fn to_iso8601(&self) -> Option<String> {
        match self {
            Self::Iso8601(t) => Some(t.clone()),
            Self::UnixTimeStamp(t) => match iso8601_from_timestamp(*t) {
                Ok(t) => Some(t),
                Err(err) => {
                    std_db_error!("{err}");
                    None
                }
            },
        }
    }
}

impl Default for TimeRepr {
    fn default() -> Self {
        Self::Iso8601(cur_time_iso8601())
    }
}

/// Get human readable name of a user in specified group with best effort.  
///
/// Returns one of the following in descending priority:  
/// 0. known member config  
/// 1. card, the nickname used exclusively in specified group  
/// 2. username, the global nickname for user account  
/// 3. user id, wouldn't bother querying stranger info  
pub async fn get_name_in_group(group_id: i64, user_id: i64) -> String {
    // decide to nest for short circuit 0
    // if let else syntax cannot fall through normal control
    let config = CONFIG.get().unwrap();
    if let Some(ref groups) = config.groups {
        if let Some(group) = groups.iter().find(|&g| g.id == group_id) {
            if let Some(ref agent) = group.agent {
                // is a known member -> return configured name
                if let Some((name, _)) = agent.known_members.get(&user_id.to_string()) {
                    return name.to_string();
                }
            }
        }
    }

    // fallback to 1, 2, 3
    let bot = global_state::get_bot();
    let group_member_api = bot.get_group_member_info(group_id, user_id, false).await;

    match group_member_api {
        Ok(api) => {
            // request success
            let group_member_info =
                serde_json::from_value::<GroupMemberInfoResponse>(api.data.clone());
            match group_member_info {
                Ok(info) => {
                    // deserialize success
                    // 1, 2
                    let first_non_empty = [info.card, info.nickname]
                        .into_iter()
                        .find(|x| !x.is_empty());
                    match first_non_empty {
                        Some(name) => name,
                        // 3
                        None => user_id.to_string(),
                    }
                }
                Err(err) => {
                    // deserialize fail
                    // 3
                    std_db_error!(
                        "
                        GroupMemberInfo deserialize failed.
                        Cause: {err}
                        Data: {}
                        ",
                        api.data
                    );
                    // 3
                    user_id.to_string()
                }
            }
        }
        Err(err) => {
            // request fail
            // 3
            std_db_error!(
                "
                GroupMemberInfo api request failed.
                Cause: {err}
                "
            );
            user_id.to_string()
        }
    }
}

/// For somewhat reason [bot.send_group_msg][kovi::RuntimeBot::send_group_msg] invokes [From] thus
/// clone is inevitable here.
pub async fn send_group_and_log<T>(group_id: i64, message: T)
where
    T: Into<Message>,
    T: Serialize,
{
    let bot = global_state::get_bot();
    let message: Message = message.into();
    let sender_id = *BOT_QQ.get().unwrap();
    bot.send_group_msg(group_id, message.clone());
    store::write_group_msg(group_id, 0, None, sender_id, message).await;
}

/// Execute the configured script to upload a file and return its stdout.  
///
/// It is safe to call it without [object config][global_state::Config::object_storage], or with a
/// script that does not function correctly. In such cases the return value will fallback to file
/// path thus no data loss.
pub async fn call_upload(file_path_str: &str) -> String {
    let config = CONFIG.get().unwrap();
    // object storage not configured, return original file path
    let Some(ref obj) = config.object_storage else {
        return file_path_str.to_string();
    };

    // script path
    let exec_path_str = &obj.script_path;
    let exec_path = PathBuf::from(exec_path_str);
    let Ok(abs_exec) = exec_path.canonicalize() else {
        std_db_error!("Script path cannot be parsed to an absolute path: {exec_path_str}");
        return file_path_str.to_string();
    };
    let abs_exec_str = abs_exec.to_string_lossy().to_string();

    // file path to be uploaded
    let file_path = PathBuf::from(file_path_str);
    let abs_file_str = file_path.to_string_lossy().to_string();

    std_info!("Execute script: {abs_exec_str}, Argument: {abs_file_str}");

    // launch child process
    let mut cmd = kovi::tokio::process::Command::new(abs_exec_str);
    let output = match cmd.arg(abs_file_str).output().await {
        Ok(out) => out,
        Err(err) => {
            std_db_error!("Launch process failed: {err}");
            return file_path_str.to_string();
        }
    };

    if !output.status.success() {
        // script terminates with code != 0
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        std_db_error!(
            "
            Upload script failed.
            Stderr: {stderr}
            "
        );
        return file_path_str.to_string();
    }
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    std_info!("Upload script succeed with online path: {stdout}");
    stdout
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
pub struct GroupMemberInfoResponse {
    group_id: i64,
    user_id: i64,
    nickname: String,
    card: String,
    sex: String,
    age: i32,
    area: String,
    join_time: i32,
    last_sent_time: i32,
    level: String,
    role: String,
    unfriendly: bool,
    title: String,
    title_expire_time: i32,
    card_changeable: bool,
}
