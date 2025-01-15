//! Database access module.
//!
//! All functions except [init_sqlite_pool] must be called after
//! [crate::global_state::init_global_state].  
//! db_* functions interact with database.
use crate::{
    exception::{PluginError, PluginResult},
    global_state, std_db_error, std_error, std_info,
    util::{self, TimeRepr},
    CONFIG, DATA_PATH, DB_POOL,
};
use kovi::{
    tokio::{fs::File, io::AsyncWriteExt},
    ApiReturn, Message,
};
use sqlx::{migrate::MigrateDatabase, prelude::FromRow, Pool, Sqlite};

/// Write log to log_bot table, fallback to kovi log on failure.
pub async fn db_write_bot_log(time: String, level: String, content: String) {
    let pool = DB_POOL.get().unwrap();
    let query = insert_log();
    let res = sqlx::query(&query)
        .bind(&time)
        .bind(&level)
        .bind(&content)
        .execute(pool)
        .await;
    if let Err(e) = res {
        std_error!(
            "
            Write bot log to database failed: {e}
            Log: {content}
            "
        );
    }
}

/// Initialize sqlite_pool
pub async fn init_sqlite_pool(max_conn: u32) -> PluginResult<Pool<Sqlite>> {
    let data_path = DATA_PATH.get().unwrap();
    let db_path = data_path.join("store.db");
    let db_url = format!("sqlite://{}", db_path.to_string_lossy());

    if Sqlite::database_exists(&db_url).await? {
        std_info!("Building connection pool from existing database...");
    } else {
        Sqlite::create_database(&db_url).await?;
        std_info!("Building connection pool from newly created database...");
    }

    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(max_conn)
        .connect(&db_url)
        .await?;
    Ok(pool)
}

/// Pre-defined tables that are known to exist at compile time.
pub async fn init_log_table() -> PluginResult<()> {
    let pool = DB_POOL.get().unwrap();
    std_info!("Initializing bot log table...");
    let query = create_log_table();
    sqlx::query(&query).execute(pool).await?;
    Ok(())
}

/// Parse to human accessible format with best effort and persist all segments. Invoke upload
/// script if necessary.
pub async fn write_group_msg<T>(
    group_id: i64,
    message_id: i32,
    time: Option<TimeRepr>,
    sender_id: i64,
    message: T,
) where
    T: Into<Message>,
{
    let bot = global_state::get_bot();

    let Some(time) = time.unwrap_or_default().to_iso8601().await else {
        return;
    };
    let sender_name = util::get_name_in_group(group_id, sender_id).await;
    let segments = util::extract_segments(message).await;
    for (seg_type, seg_content) in segments {
        let (content, interpret) = match seg_type.as_str() {
            "share" => (seg_content, "url".to_string()),
            "video" => (seg_content, "not supported".to_string()),
            "record" => {
                let res = bot.get_record(&seg_content, "mp3").await;
                let path = extract_api(res, "file");
                if path.starts_with('/') {
                    (path.clone(), util::call_upload(&path).await)
                } else {
                    (path, String::new())
                }
            }
            "image" => {
                let res = bot.get_image(&seg_content).await;
                let path = extract_api(res, "file");
                if path.starts_with('/') {
                    (path.clone(), util::call_upload(&path).await)
                } else {
                    (path.clone(), String::new())
                }
            }
            "at" => {
                let Ok(receiver_id) = seg_content.parse::<i64>() else {
                    std_db_error!("At message has content not i64: {seg_content}");
                    continue;
                };
                (
                    receiver_id.to_string(),
                    util::get_name_in_group(group_id, receiver_id).await,
                )
            }
            "reply" => (seg_content, "message_id".to_string()),
            "text" => (seg_content, "text".to_string()),
            _ => (String::new(), String::new()),
        };
        let res = db_write_group_msg(
            group_id,
            message_id,
            &time,
            sender_id,
            &sender_name,
            &seg_type,
            &content,
            &interpret,
        )
        .await;
        if let Err(e) = res {
            std_db_error!("Write group message failed: {e}");
        }
    }
}

fn extract_api(res: Result<ApiReturn, ApiReturn>, field: &str) -> String {
    match res {
        Ok(api) => match serde_json::from_value(api.data[field].clone()) {
            Ok(v) => v,
            Err(e) => e.to_string(),
        },
        Err(e) => e.to_string(),
    }
}

async fn db_write_group_msg(
    group_id: i64,
    message_id: i32,
    time: &str,
    sender_id: i64,
    sender_name: &str,
    seg_type: &str,
    content: &str,
    interpret: &str,
) -> PluginResult<()> {
    let pool = DB_POOL.get().unwrap();
    let table_name = get_group_msg_table_name(group_id);
    let query = create_group_msg_table(&table_name);
    sqlx::query(&query).execute(pool).await?;

    let query = insert_group_msg(&table_name);
    sqlx::query(&query)
        .bind(message_id)
        .bind(time)
        .bind(sender_id)
        .bind(sender_name)
        .bind(seg_type)
        .bind(content)
        .bind(interpret)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn db_load_n_group_segment(group_id: i64, n: i64) -> PluginResult<Vec<GroupChatSegment>> {
    let pool = DB_POOL.get().unwrap();
    let table_name = get_group_msg_table_name(group_id);

    let query = load_n_latest_msg(&table_name);
    let segs: Vec<GroupChatSegment> = sqlx::query_as(&query).bind(n).fetch_all(pool).await?;
    Ok(segs)
}

async fn dump_csv(filename: &str, query: &str) -> PluginResult<String> {
    let data_path = DATA_PATH.get().unwrap();
    let file_path = data_path.join(filename);
    let file_path_str = file_path.to_string_lossy().to_string();
    let db_path = data_path.join("store.db");
    let db_path_str = db_path.to_string_lossy().to_string();

    let mut cmd = kovi::tokio::process::Command::new("sqlite3");
    let output = cmd
        .args(["-header", "-csv", &db_path_str, query])
        .output()
        .await?;
    if output.status.success() {
        let mut csv_file = File::create(file_path).await?;
        csv_file.write_all(&output.stdout).await?;
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(PluginError::ChildProcess("sqlite3".into(), stderr));
    }
    Ok(file_path_str)
}

pub async fn dump_log_csv(filename: &str, n: i64) -> PluginResult<String> {
    let query = load_n_latest_log();
    let query = query.replace("$1", &n.to_string());
    dump_csv(filename, &query).await
}

pub async fn dump_history_csv(group_id: i64, filename: &str, n: i64) -> PluginResult<String> {
    let table_name = get_group_msg_table_name(group_id);
    let query = load_n_latest_msg(&table_name);
    let query = query.replace("$1", &n.to_string());
    dump_csv(filename, &query).await
}

pub async fn db_find_segment_by_id(
    group_id: i64,
    message_id: i32,
) -> PluginResult<Vec<GroupChatSegment>> {
    let pool = DB_POOL.get().unwrap();
    let table_name = get_group_msg_table_name(group_id);

    let query = find_segment_by_id(&table_name);
    let segs: Vec<GroupChatSegment> = sqlx::query_as(&query)
        .bind(message_id)
        .fetch_all(pool)
        .await?;
    Ok(segs)
}

fn get_group_msg_table_name(group_id: i64) -> String {
    let config = CONFIG.get().unwrap();
    let prefix = &config.database.group_table_prefix;
    format!("{}{}", prefix, group_id)
}

use sql_query::*;
mod sql_query {
    use crate::CONFIG;
    use indoc::{formatdoc, indoc};

    const CREATE_TABLE_IF_NOT_EXISTS: &str = "CREATE TABLE IF NOT EXISTS";
    const CREATE_INDEX_IF_NOT_EXISTS: &str = "CREATE INDEX IF NOT EXISTS";
    const INSERT_INTO: &str = "INSERT INTO";
    const GROUP_MSG_SCHEMA: &str = indoc!(
        "
        (
            auto_id INTEGER PRIMARY KEY,
            message_id INTEGER,
            time TEXT,
            sender_id INTEGER,
            sender_name TEXT,
            type TEXT,
            content TEXT,
            interpret TEXT
        )
        "
    );
    pub const INSERT_GROUP_MSG_SCHEMA: &str = indoc!(
        "
        (message_id, time, sender_id, sender_name, type, content, interpret)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        "
    );

    pub fn create_log_table() -> String {
        let config = CONFIG.get().unwrap();
        let table_name = &config.database.log_table_name;
        formatdoc!(
            "
            {CREATE_TABLE_IF_NOT_EXISTS} {table_name}(
                auto_id INTEGER PRIMARY KEY,
                time TEXT,
                level TEXT,
                content TEXT
            );
            {CREATE_INDEX_IF_NOT_EXISTS} log_time
            ON {table_name}(time);
            "
        )
    }

    pub fn insert_log() -> String {
        let config = CONFIG.get().unwrap();
        let table_name = &config.database.log_table_name;
        formatdoc!(
            "
            INSERT INTO {table_name} (time, level, content)
            VALUES($1, $2, $3);
            "
        )
    }

    pub fn create_group_msg_table(table_name: &str) -> String {
        formatdoc!(
            "
            {CREATE_TABLE_IF_NOT_EXISTS} {table_name} {GROUP_MSG_SCHEMA};
            {CREATE_INDEX_IF_NOT_EXISTS} msg_id
            ON {table_name}(message_id);
            {CREATE_INDEX_IF_NOT_EXISTS} msg_time
            ON {table_name}(time);
            "
        )
    }

    pub fn insert_group_msg(table_name: &str) -> String {
        format!("{INSERT_INTO} {table_name} {INSERT_GROUP_MSG_SCHEMA};")
    }

    pub fn load_n_latest_msg(table_name: &str) -> String {
        formatdoc!(
            "
            SELECT 
                message_id, 
                time, 
                sender_id, 
                sender_name, 
                type, 
                content, 
                interpret
            FROM {table_name}
            WHERE time IN (
                SELECT DISTINCT time
                FROM {table_name}
                ORDER BY time DESC
                LIMIT $1
            )
            ORDER BY time ASC;
            "
        )
    }

    pub fn load_n_latest_log() -> String {
        let config = CONFIG.get().unwrap();
        let table_name = &config.database.log_table_name;
        formatdoc!(
            "
            SELECT
                time,
                level,
                content
            FROM {table_name}
            WHERE time IN (
                SELECT DISTINCT time
                FROM {table_name}
                ORDER BY time DESC
                LIMIT $1
            )
            ORDER BY time ASC;
            "
        )
    }

    pub fn find_segment_by_id(table_name: &str) -> String {
        formatdoc!(
            "
            SELECT 
                message_id, 
                time, 
                sender_id, 
                sender_name, 
                type, 
                content, 
                interpret
            FROM {table_name}
            WHERE message_id = $1;
            "
        )
    }
}

#[derive(FromRow, Debug)]
pub struct GroupChatSegment {
    pub message_id: i32,
    pub time: String,
    pub sender_id: i64,
    pub sender_name: String,
    #[sqlx(rename = "type")]
    pub seg_type: String,
    pub content: String,
    pub interpret: String,
}

impl GroupChatSegment {
    pub async fn db_store(&self, group_id: i64) -> PluginResult<()> {
        db_write_group_msg(
            group_id,
            self.message_id,
            &self.time,
            self.sender_id,
            &self.sender_name,
            &self.seg_type,
            &self.content,
            &self.interpret,
        )
        .await
    }
}
