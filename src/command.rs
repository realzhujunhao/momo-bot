//! Detect group message and respond to commands.

use kovi::{tokio::fs, Message, MsgEvent};
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    global_state, std_db_error, store,
    util::{self, call_upload},
    CONFIG, DATA_PATH,
};

pub async fn act(e: Arc<MsgEvent>) {
    let Some(text) = e.borrow_text() else {
        return;
    };
    let Some(group_id) = e.group_id else {
        return;
    };
    let config = CONFIG.get().unwrap();
    let Some(ref groups) = config.groups else {
        return;
    };
    let Some(group) = groups.iter().find(|&g| g.id == group_id) else {
        return;
    };
    let Some(ref command) = group.command else {
        return;
    };
    if !command.admin_ids.contains(&e.sender.user_id) {
        return;
    }
    let Some(cmd) = command.parse_command(text) else {
        return;
    };

    match cmd {
        crate::GroupCommand::Mute => {
            let Some(ref agent) = group.agent else {
                util::send_group_and_log(group_id, "未配置agent").await;
                return;
            };
            if agent.is_mute() {
                util::send_group_and_log(group_id, "...").await;
                return;
            }
            agent.mute();
            util::send_group_and_log(group_id, "接下来我将冷暴力你们所有人,直到主人哀求我").await;
        }
        crate::GroupCommand::Unmute => {
            let Some(ref agent) = group.agent else {
                util::send_group_and_log(group_id, "未配置agent").await;
                return;
            };
            if !agent.is_mute() {
                util::send_group_and_log(group_id, "...").await;
                return;
            }
            agent.unmute();
            util::send_group_and_log(group_id, "我勉为其难地同意和你们聊天").await;
        }
        crate::GroupCommand::SwitchModel(model) => {
            let Some(ref agent) = group.agent else {
                util::send_group_and_log(group_id, "未配置agent").await;
                return;
            };
            agent.set_model(model.clone()).await;
            let msg = format!("我的脑子被换成了{model}");
            util::send_group_and_log(group_id, msg).await;
        }
        crate::GroupCommand::DumpHistory(count) => {
            if count < 1 {
                return;
            }
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let csv_name = format!("{group_id}-{timestamp}.csv");
            let history = store::dump_history_csv(group_id, &csv_name, count).await;
            match history {
                Ok(csv_path) => {
                    let url = call_upload(&csv_path).await;
                    let msg = format!("导出了{count}条聊天记录: {url}");
                    util::send_group_and_log(group_id, msg).await;
                }
                Err(err) => {
                    std_db_error!(
                        "
                        Dump history failed.
                        Cause: {err}
                        "
                    );
                }
            }
        }
        crate::GroupCommand::DumpLog(count) => {
            if count < 1 {
                return;
            }
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let csv_name = format!("{group_id}-{timestamp}.csv");
            let history = store::dump_log_csv(&csv_name, count).await;
            match history {
                Ok(csv_path) => {
                    let url = call_upload(&csv_path).await;
                    let msg = format!("导出了{count}条日志: {url}");
                    util::send_group_and_log(group_id, msg).await;
                }
                Err(err) => {
                    std_db_error!(
                        "
                        Dump history failed.
                        Cause: {err}
                        "
                    );
                }
            }
        }
    }
}

pub async fn dump_history(e: Arc<MsgEvent>, n: i64) {
    let Some(group_id) = e.group_id else {
        return;
    };
    let bot = global_state::get_bot();
    let data_path = DATA_PATH.get().unwrap();
    let now = SystemTime::now();
    let timestamp = now
        .duration_since(UNIX_EPOCH)
        .expect("time backward")
        .as_secs();
    let csv_path = data_path.join(format!("{timestamp}.csv"));
    let csv_path_str = csv_path.to_string_lossy().to_string();
    if let Err(err) = store::dump_history_csv(group_id, &csv_path_str, n).await {
        std_db_error!(
            "
            Dump history command failed.
            Cause: {err}
            "
        );
    }
    let url = call_upload(&csv_path_str).await;
    let message = Message::from(format!("导出{n}条记录: {url}"));
    if let Err(err) = fs::remove_file(csv_path).await {
        std_db_error!(
            "
            Delete file failed.
            Cause: {err}
            "
        );
    }
    bot.send_group_msg(group_id, message);
}
