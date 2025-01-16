//! Bilibili live module

use std::{fmt::Display, sync::Arc, time::Duration};

use indoc::{formatdoc, writedoc};
use kovi::{Message, MsgEvent};
use serde::{Deserialize, Deserializer};

use crate::{
    exception::PluginResult,
    global_state::{self, LiveSwitch},
    std_error, std_info,
    util::schedule_task_blocking,
    CONFIG,
};

async fn query_liveroom(room_id: &str) -> PluginResult<LiveRoom> {
    let url = "https://api.live.bilibili.com/room/v1/Room/get_info";
    let params = [("room_id", room_id)];
    let client = reqwest::Client::new();
    let room = client.get(url).query(&params).send().await?.json().await?;
    Ok(room)
}

async fn query_handler(e: Arc<MsgEvent>, room_id: &str, online_msg: &str, offline_msg: &str) {
    // no-op if not group message
    if e.group_id.is_none() {
        return;
    };

    let room = match query_liveroom(room_id).await {
        Ok(room) => room,
        Err(err) => {
            std_error!("Query liveroom failed: {err}");
            return;
        }
    };
    if !room.exist {
        let message = Message::from(format!("直播间{}不存在", room_id));
        e.reply(message);
        return;
    }
    let status_str = if room.data.is_streaming {
        online_msg
    } else {
        offline_msg
    };
    let resp = formatdoc!(
        "
        {status_str}
        链接:{}
        {}
        ",
        LiveRoom::url_from_id(room_id),
        room
    );
    let mut message = Message::new().add_text(resp);
    // add key_frame if exists, otherwise fallback to user_cover
    let fallback_list = [room.data.keyframe, room.data.user_cover];
    let image = fallback_list.iter().find(|&x| !x.is_empty());
    if let Some(img) = image {
        message = message.add_image(img);
    }
    e.reply(message);
}

pub async fn general_query_handler(e: Arc<MsgEvent>) {
    // no-op if no text
    let Some(msg) = e.borrow_text() else {
        return;
    };
    let query_message = "查询直播间";
    if !msg.contains(query_message) {
        return;
    }
    let msg = msg.replace(query_message, "");
    let room_id = msg.trim();
    if room_id.parse::<usize>().is_err() {
        e.reply("直播间不存在");
        return;
    }
    query_handler(e, room_id, "直播中", "不在直播").await;
}

pub async fn local_query_handler(e: Arc<MsgEvent>) {
    // no-op if not group message
    let Some(group_id) = e.group_id else {
        return;
    };
    // no-op if no text
    let Some(msg) = e.borrow_text() else {
        return;
    };
    // no-op if no group config
    let config = CONFIG.get().unwrap();
    let Some(ref groups) = config.groups else {
        return;
    };
    let Some(group) = groups.iter().find(|&g| g.id == group_id) else {
        return;
    };
    // no-op if no live config
    let Some(ref live) = group.live else {
        return;
    };

    // now pre-configured group found, and it has live setting
    // check query_msg
    if msg.contains(&live.query_message) {
        query_handler(e, &live.room_id, &live.online_msg, &live.offline_msg).await;
    }
}

pub async fn subscribe_live() {
    let config = CONFIG.get().unwrap();

    // no-op if no group config
    let Some(ref groups) = config.groups else {
        return;
    };

    let id_lives = groups
        .iter()
        .filter_map(|g| g.live.as_ref().map(|live| (g.id, live)));

    // spawn a task for each live room
    for (group_id, live) in id_lives {
        kovi::spawn(async move {
            let duration = Duration::from_secs(live.poll_interval_sec);
            schedule_task_blocking(duration, move || {
                async move {
                    let room = match query_liveroom(&live.room_id).await {
                        Ok(v) => v,
                        Err(err) => {
                            std_error!("Query live room failed: {err}");
                            return;
                        }
                    };
                    if !room.exist {
                        std_error!("直播间{}不存在", live.room_id);
                        return;
                    }
                    let bot = global_state::get_bot();
                    match live.get_switch() {
                        LiveSwitch::On => {
                            // used to be online, send msg only if offline
                            if !room.data.is_streaming {
                                std_info!("not streaming, offline notification");
                                let msg = Message::new().add_text(&live.offline_msg);
                                bot.send_group_msg(group_id, msg);
                                live.set_switch(LiveSwitch::Off);
                            }
                        }
                        LiveSwitch::Off => {
                            // used to be offline, send msg only if online
                            if room.data.is_streaming {
                                std_info!("streaming, online notification");
                                let resp = formatdoc!(
                                    "
                                    {}
                                    链接:{}
                                    {}
                                    ",
                                    &live.online_msg,
                                    LiveRoom::url_from_id(&live.room_id),
                                    room
                                );
                                let mut message = Message::new().add_text(resp);

                                // add key_frame if exists, otherwise fallback to user_cover
                                let fallback_list = [room.data.keyframe, room.data.user_cover];
                                let image = fallback_list.iter().find(|&x| !x.is_empty());
                                if let Some(img) = image {
                                    message = message.add_image(img);
                                }
                                bot.send_group_msg(group_id, message);
                                live.set_switch(LiveSwitch::On);
                            }
                        }
                        LiveSwitch::Init => {
                            // avoid online notification on launching
                            std_info!("Live switch: Init");
                            match room.data.is_streaming {
                                true => live.set_switch(LiveSwitch::On),
                                false => live.set_switch(LiveSwitch::Off),
                            }
                        }
                        LiveSwitch::Trap => {
                            // if I were myself 2 years ago I would use unreachable!()
                            std_error!(
                                "Subscribe live in trap state: group id = {}",
                                &live.room_id
                            );
                        }
                    }
                }
            })
            .await;
        });
    }
}

#[derive(Deserialize, Debug)]
pub struct LiveRoom {
    #[serde(rename = "code", deserialize_with = "parse_code")]
    pub exist: bool,
    pub data: LiveData,
}

impl LiveRoom {
    pub fn url_from_id(room_id: &str) -> String {
        format!("https://live.bilibili.com/{}", room_id)
    }
}

impl Display for LiveRoom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writedoc!(
            f,
            "
            分区:{}
            标题:{}
            简介:{}
            热度:{}, 关注:{}
            ",
            self.data.area_name,
            self.data.title,
            self.data.description,
            self.data.online,
            self.data.attention
        )
    }
}

#[derive(Deserialize, Debug)]
pub struct LiveData {
    #[serde(rename = "live_status", deserialize_with = "parse_status")]
    pub is_streaming: bool,
    pub online: usize,
    pub attention: usize,
    pub keyframe: String,
    pub user_cover: String,
    pub area_name: String,
    pub description: String,
    pub title: String,
}

fn parse_code<'de, D>(d: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    let Ok(code) = i32::deserialize(d) else {
        return Ok(false);
    };
    match code {
        0 => Ok(true),
        _ => Ok(false),
    }
}

fn parse_status<'de, D>(d: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    let Ok(code) = i32::deserialize(d) else {
        return Ok(false);
    };
    match code {
        1 => Ok(true),
        _ => Ok(false),
    }
}
