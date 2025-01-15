//! OpenAI module.

use crate::{
    exception::{PluginError, PluginResult},
    global_state, std_db_error, std_db_info, std_info,
    store::{self, GroupChatSegment},
    util::{self, TimeRepr},
    AgentSetting, BOT_QQ, CONFIG,
};
use kovi::{Message, MsgEvent};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

pub async fn logger(e: Arc<MsgEvent>) {
    let Some(group_id) = e.group_id else {
        return;
    };
    let sender_id = e.sender.user_id;
    let time = TimeRepr::UnixTimeStamp(e.time);
    store::write_group_msg(
        group_id,
        e.message_id,
        Some(time),
        sender_id,
        e.message.clone(),
    )
    .await;
}

pub async fn at_me_handler(e: Arc<MsgEvent>) {
    let bot = global_state::get_bot();
    // no-op if not group message
    let Some(group_id) = e.group_id else {
        return;
    };
    let at_segs = e.message.get("at");
    let bot_qq = BOT_QQ.get().unwrap();
    let bot_qq_str = bot_qq.to_string();

    // this will never fail, good to log if api changes
    let missing_field = at_segs
        .iter()
        .map(|x| {
            (
                x.data.clone(),
                serde_json::from_value::<String>(x.data["qq"].clone()),
            )
        })
        .find(|(_, res)| res.is_err());

    if let Some((data, _)) = missing_field {
        std_db_error!("At message without qq field\n{:?}", data);
    }

    // no-op if not at me
    let at_me =
        at_segs.iter().find(
            |&x| match serde_json::from_value::<String>(x.data["qq"].clone()) {
                Ok(target) => bot_qq_str == target,
                Err(_) => false,
            },
        );
    if at_me.is_none() {
        return;
    }

    // no-op if no group config
    let config = CONFIG.get().unwrap();
    let Some(ref groups) = config.groups else {
        return;
    };
    let Some(group) = groups.iter().find(|&g| g.id == group_id) else {
        bot.send_group_msg(group_id, "该群聊未配置");
        return;
    };

    // no-op if no agent config
    let Some(ref agent) = group.agent else {
        return;
    };
    // no-op if mute
    if agent.is_mute() {
        return;
    }

    let time = TimeRepr::UnixTimeStamp(e.time);
    let sender_id = e.sender.user_id;
    let content = util::extract_text(&e.message).await;
    if let Some(answer) = agent
        .group_query(group_id, Some(time), sender_id, &content)
        .await
    {
        let message = Message::from(answer);
        e.reply_and_quote(message);
    }
}

// Mimic an "at me" as if someone asks agent a question, then send answer to group.
pub async fn query_with_id_msg(
    group_id: i64,
    sender_id: i64,
    message: String,
) -> PluginResult<String> {
    let invoke_no_agent = Err(PluginError::AgentRequest(
        "Call query_with_id_msg without agent config".to_string(),
    ));

    // no-op if no agent config
    let config = CONFIG.get().unwrap();
    let Some(ref groups) = config.groups else {
        return invoke_no_agent;
    };
    let Some(group) = groups.iter().find(|&g| g.id == group_id) else {
        return invoke_no_agent;
    };
    let Some(ref agent) = group.agent else {
        return invoke_no_agent;
    };

    // no-op if mute
    let agent_mute = Err(PluginError::AgentRequest("Mute".to_string()));
    if agent.is_mute() {
        return agent_mute;
    }

    let query_fail =
        PluginError::AgentRequest("Agent query failed, check log for details.".to_string());
    agent
        .group_query(group_id, None, sender_id, &message)
        .await
        .ok_or(query_fail)
}

impl AgentSetting {
    pub async fn group_query(
        &self,
        group_id: i64,
        time: Option<TimeRepr>,
        sender_id: i64,
        content: &str,
    ) -> Option<String> {
        // obtain iso8601
        let time = match time.unwrap_or_default() {
            TimeRepr::Iso8601(t) => t,
            TimeRepr::UnixTimeStamp(t) => match util::iso8601_from_timestamp(t) {
                Ok(t) => t,
                Err(err) => {
                    std_db_error!("{err}");
                    return None;
                }
            },
        };

        // search member table
        let (sender_name, know) = match self.known_members.get(&sender_id.to_string()) {
            Some((name, _)) => (name, true),
            None => (&util::get_name_in_group(group_id, sender_id).await, false),
        };

        // load history
        let n = self.aware_history_segments;
        let history = match store::db_load_n_group_segment(group_id, n).await {
            Ok(v) => v,
            Err(err) => {
                std_db_error!("Load chat history failed: {err}");
                return None;
            }
        };
        let message = format!("{time} {sender_name}: {content}");
        let (dev_prompt, user_prompt) = self.substitute_dev_user(&history, &message, know);
        std_info!(
            "
            Developer prompt: {dev_prompt}
            User Prompt:{user_prompt}
            "
        );

        match self.api_request(&dev_prompt, &user_prompt).await {
            Ok(resp) => {
                let model = resp.model;
                let tokens = resp.usage.total_tokens;
                std_db_info!("{model} consumed {tokens} tokens");
                let Some(answer) = resp.choices.first() else {
                    std_db_error!("OpenAI API response has no choice");
                    return None;
                };
                let sol = &answer.message.content;
                Some(sol.to_string())
            }
            Err(e) => {
                std_db_error!("OpenAI request failed: {e}");
                None
            }
        }
    }

    async fn api_request(&self, dev_prompt: &str, user_prompt: &str) -> PluginResult<GptResponse> {
        let model = self.get_model().await;

        let payload = match model.as_ref() {
            "o1" | "o1-mini" | "o1-preview" => {
                json!({
                    "model": model,
                    "messages": [
                        {
                            "role": "user",
                            "content": format!("{dev_prompt}\n{user_prompt}")
                        }
                    ]
                })
            }
            _ => {
                json!({
                    "model": model,
                    "messages": [
                        {
                            "role": "developer",
                            "content": dev_prompt
                        },
                        {
                            "role": "user",
                            "content": user_prompt
                        }
                    ]
                })
            }
        };
        let client = reqwest::Client::new();
        let response = client
            .post(&self.api_url)
            .header(CONTENT_TYPE, "application/json")
            .header(AUTHORIZATION, format!("Bearer {}", self.api_key))
            .json(&payload)
            .send()
            .await?;
        let resp_str = response.text().await.unwrap();
        std_info!(
            "
            OpenAI response:
            {resp_str}
            "
        );
        let response = client
            .post(&self.api_url)
            .header(CONTENT_TYPE, "application/json")
            .header(AUTHORIZATION, format!("Bearer {}", self.api_key))
            .json(&payload)
            .send()
            .await?;
        Ok(response.json().await?)
    }

    /// Replace placeholders for know, message, and history by their runtime value.
    fn substitute_dev_user(
        &self,
        history: &Vec<GroupChatSegment>,
        message: &str,
        know: bool,
    ) -> (String, String) {
        let know = if know { "know" } else { "don't know" };
        let dev_know = self.dev_prompt.replace("<!know!>", know);
        let user_know = self.user_prompt.replace("<!know!>", know);

        let dev_msg = dev_know.replace("<!message!>", message);
        let user_msg = user_know.replace("<!message!>", message);

        let mut buf = String::new();
        for seg in history {
            match seg.seg_type.as_str() {
                "text" => {
                    let time_sender_content =
                        format!("{} {}: {}\n", seg.time, seg.sender_name, seg.content);
                    buf.push_str(&time_sender_content);
                }
                "at" => {
                    let time_sender_receiver =
                        format!("{} {} AT {}\n", seg.time, seg.sender_name, seg.interpret);
                    buf.push_str(&time_sender_receiver);
                }
                _ => (),
            }
        }
        let dev_all = dev_msg.replace("<!history!>", &buf);
        let user_all = user_msg.replace("<!history!>", &buf);

        (dev_all, user_all)
    }
}

#[derive(Deserialize, Debug, Default)]
pub struct GptResponse {
    pub id: String,
    pub model: String,
    pub choices: Vec<Choice>,
    pub usage: Usage,
}

#[derive(Deserialize, Debug)]
pub struct Choice {
    pub message: Answer,
    pub finish_reason: String,
}

#[derive(Deserialize, Debug)]
pub struct Answer {
    pub content: String,
}

#[derive(Deserialize, Debug, Default)]
pub struct Usage {
    pub total_tokens: usize,
}
