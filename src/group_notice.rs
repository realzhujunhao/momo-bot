//! Strong typed implementation of group notice handler.

use crate::{
    agent, db_error, db_warn, std_db_info, std_error,
    store::{self, GroupChatSegment},
    util, BOT_QQ,
};
use kovi::{log::error, Message, NoticeEvent};
use serde::Deserialize;
use std::sync::Arc;

pub async fn act(e: Arc<NoticeEvent>) {
    // deserialize notice
    let notice = match serde_json::from_value::<NoticeResponse>(e.original_json.clone()) {
        Ok(v) => v,
        Err(err) => {
            error!(
                "NoticeResponse deserialize failed, skip group interact\ncause:{err}\nraw:{}",
                e.original_json
            );
            return;
        }
    };

    use Notify::*;
    // dispatch event
    match notice {
        NoticeResponse::GroupUpload(_notice) => (),
        NoticeResponse::GroupAdmin(notice) => handle_admin(notice).await,
        NoticeResponse::GroupDecrease(notice) => handle_decrease(notice).await,
        NoticeResponse::GroupIncrease(notice) => handle_increase(notice).await,
        NoticeResponse::GroupBan(notice) => handle_ban(notice).await,
        NoticeResponse::FriendAdd(_notice) => (),
        NoticeResponse::GroupRecall(notice) => handle_recall(notice).await,
        NoticeResponse::Notify(notice) => match notice {
            Poke(notice) => handle_poke(notice).await,
            Honor(notice) => handle_honor(notice).await,
        },
    }
}

async fn handle_admin(notice: GroupAdmin) {
    let user_name = util::get_name_in_group(notice.group_id, notice.user_id).await;
    use GroupAdminSubType::*;
    let msg_str = match notice.sub_type {
        Set => format!("{user_name}被群主赐予了管理员之力!"),
        Unset => format!("{user_name}被群主剥夺了管理员之力!"),
    };
    let message = Message::from(msg_str);
    util::send_group_and_log(notice.group_id, message).await;
}

async fn handle_decrease(notice: GroupDecrease) {
    let group_id = notice.group_id;
    use GroupDecreaseSubType::*;
    let user_name = util::get_name_in_group(notice.group_id, notice.user_id).await;
    let op_name = util::get_name_in_group(notice.group_id, notice.operator_id).await;
    let msg_str = match notice.sub_type {
        Leave => {
            format!("{user_name}忍一时越想越气,退一步越想越亏,怒发冲冠下将所有人踢出了群聊!")
        }
        Kick => format!("{user_name}由于讨厌{op_name}选择将所有人踢出群聊!"),
        KickMe => return,
    };
    let message = Message::from(msg_str);
    util::send_group_and_log(group_id, message).await;
}

async fn handle_increase(notice: GroupIncrease) {
    let group_id = notice.group_id;
    use GroupIncreaseSubType::*;
    let user_name = util::get_name_in_group(notice.group_id, notice.user_id).await;
    let op_name = util::get_name_in_group(notice.group_id, notice.operator_id).await;
    let msg_str = match notice.sub_type {
        Approve => format!("{user_name}大发慈悲、勉为其难地允许了{op_name}通过ta的入群申请~"),
        Invite => format!("{user_name}在{op_name}的苦苦哀求下加入了我们~"),
    };
    let message = Message::from(msg_str);
    util::send_group_and_log(group_id, message).await;
}

async fn handle_ban(notice: GroupBan) {
    let group_id = notice.group_id;
    use GroupBanSubType::*;
    let user_name = util::get_name_in_group(notice.group_id, notice.user_id).await;
    let op_name = util::get_name_in_group(notice.group_id, notice.operator_id).await;
    let duration = notice.duration;
    let msg_str = match notice.sub_type {
        Ban => format!("{user_name}因为讨厌{op_name}决定在{duration}秒内冷暴力大家!"),
        LiftBan => format!("{op_name}哄好了{user_name},TA现在愿意和我们说话了!"),
    };
    let message = Message::from(msg_str);
    util::send_group_and_log(group_id, message).await;
}

async fn handle_recall(notice: GroupRecall) {
    let group_id = notice.group_id;
    let timestamp = notice.time;
    let sender_id = BOT_QQ.get().unwrap();
    let user_name = util::get_name_in_group(group_id, notice.user_id).await;
    let op_name = util::get_name_in_group(group_id, notice.operator_id).await;
    let message_id = notice.message_id;
    let segs = match store::db_find_segment_by_id(group_id, message_id as i32).await {
        Ok(segs) => segs,
        Err(e) => {
            db_error!("Find segment by id failed: {e}");
            return;
        }
    };
    if segs.is_empty() {
        db_warn!("Recalled message not found.\ngroup_id={group_id}, msg_id={message_id}");
        return;
    }
    let Ok(time) = util::iso8601_from_timestamp(timestamp) else {
        db_error!("Recall notice timestamp error, value = {timestamp}");
        return;
    };
    let msg = format!("{op_name} 撤回了 {user_name} 的消息, id={message_id}");
    let recall_indicator = GroupChatSegment {
        message_id: 0,
        time,
        sender_id: *sender_id,
        sender_name: "RECALL_INDICATOR".to_string(),
        seg_type: "text".to_string(),
        content: msg,
        interpret: "RECALL_INDICATOR".to_string(),
    };

    let store_segs = std::iter::once(recall_indicator).chain(segs);
    for seg in store_segs {
        if let Err(e) = seg.db_store(group_id).await {
            db_error!(
                "Call db_store on GroupChatSegment failed: {e}\nContent: {:?}",
                seg
            );
            continue;
        }
    }
}

async fn handle_poke(notice: Poke) {
    let bot_qq = *BOT_QQ.get().unwrap();

    if bot_qq == notice.target_id {
        let user_id = notice.user_id;
        let group_id = notice.group_id;

        match agent::query_with_id_msg(group_id, user_id, String::from("戳了戳你")).await {
            Ok(ans) => {
                util::send_group_and_log(group_id, ans).await;
            }
            Err(err) => {
                std_error!("{err}");
            }
        };
    }
}

async fn handle_honor(notice: Honor) {
    std_db_info!("Trigger handle honor.");
    use HonorType::*;
    let user_name = util::get_name_in_group(notice.group_id, notice.user_id).await;
    match notice.honor_type {
        Talkative => {
            let message = Message::from(format!("恭喜龙王{user_name}登基!"));
            util::send_group_and_log(notice.group_id, message).await;
        }
        Performer => (),
        Emotion => (),
    }
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
#[serde(tag = "notice_type", rename_all = "snake_case")]
pub enum NoticeResponse {
    GroupUpload(GroupUpload),
    GroupAdmin(GroupAdmin),
    GroupDecrease(GroupDecrease),
    GroupIncrease(GroupIncrease),
    GroupBan(GroupBan),
    FriendAdd(FriendAdd),
    GroupRecall(GroupRecall),
    Notify(Notify),
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
pub struct GroupUpload {
    pub time: i64,
    pub self_id: i64,
    pub group_id: i64,
    pub user_id: i64,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
pub struct GroupDecrease {
    pub time: i64,
    pub self_id: i64,
    pub sub_type: GroupDecreaseSubType,
    pub group_id: i64,
    pub operator_id: i64,
    pub user_id: i64,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
pub struct GroupIncrease {
    pub time: i64,
    pub self_id: i64,
    pub sub_type: GroupIncreaseSubType,
    pub group_id: i64,
    pub operator_id: i64,
    pub user_id: i64,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
pub struct GroupAdmin {
    pub time: i64,
    pub self_id: i64,
    pub sub_type: GroupAdminSubType,
    pub group_id: i64,
    pub user_id: i64,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
pub struct GroupBan {
    time: i64,
    self_id: i64,
    sub_type: GroupBanSubType,
    group_id: i64,
    operator_id: i64,
    user_id: i64,
    duration: i64,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
pub struct FriendAdd {
    time: i64,
    self_id: i64,
    user_id: i64,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
pub struct GroupRecall {
    time: i64,
    self_id: i64,
    group_id: i64,
    user_id: i64,
    operator_id: i64,
    message_id: i64,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
#[serde(tag = "sub_type", rename_all = "snake_case")]
pub enum Notify {
    Poke(Poke),
    Honor(Honor),
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
pub struct Poke {
    time: i64,
    self_id: i64,
    group_id: i64,
    user_id: i64,
    target_id: i64,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
pub struct Honor {
    time: i64,
    self_id: i64,
    group_id: i64,
    honor_type: HonorType,
    user_id: i64,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GroupAdminSubType {
    Set,
    Unset,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GroupDecreaseSubType {
    Leave,
    Kick,
    KickMe,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GroupIncreaseSubType {
    Approve,
    Invite,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GroupBanSubType {
    Ban,
    LiftBan,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HonorType {
    Talkative,
    Performer,
    Emotion,
}

#[allow(unused)]
mod tests {
    use super::*;
    #[test]
    fn test_deserialize_notify_honor() {
        let json_data = r#"
        {
            "notice_type": "notify",
            "sub_type": "honor",
            "time": 1627847284,
            "self_id": 123456789,
            "group_id": 987654321,
            "honor_type": "talkative",
            "post_type": "notice",
            "user_id": 1122334455
        }
        "#;

        let expected = NoticeResponse::Notify(Notify::Honor(Honor {
            time: 1627847284,
            self_id: 123456789,
            group_id: 987654321,
            honor_type: HonorType::Talkative,
            user_id: 1122334455,
        }));

        let result: NoticeResponse =
            serde_json::from_str(json_data).expect("Deserialization failed");
        assert_eq!(result, expected);
    }

    #[test]
    fn test_deserialize_admin() {
        let json_data = r#"
        {
            "notice_type": "group_admin",
            "sub_type": "set",
            "time": 1234,
            "self_id": 5678,
            "group_id": 91011,
            "post_type": "notice",
            "user_id": 1122334455
        }
        "#;

        let expected = NoticeResponse::GroupAdmin(GroupAdmin {
            time: 1234,
            self_id: 5678,
            group_id: 91011,
            user_id: 1122334455,
            sub_type: GroupAdminSubType::Set,
        });

        let result: NoticeResponse =
            serde_json::from_str(json_data).expect("Deserialization failed");
        assert_eq!(result, expected);
    }
}
