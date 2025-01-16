# Momo Bot

[![Crates.io](https://img.shields.io/crates/v/kovi-plugin-live-agent.svg)](https://crates.io/crates/kovi-plugin-live-agent) [![Documentation](https://docs.rs/kovi-plugin-live-agent/badge.svg)](https://docs.rs/kovi-plugin-live-agent) [![Crates.io](https://img.shields.io/crates/d/kovi-plugin-live-agent.svg)](https://crates.io/crates/kovi-plugin-live-agent)

Momo Bot是基于Kovi框架的群助手。

```toml
kovi-plugin-live-agent = "0.1"
```

#### 核心特性

1. 持久化聊天记录（仅支持Sqlite）
2. 群事件回应，包括龙王，加入群聊，离开群聊，禁言，设置管理员等播报

#### 可选特性（禁用的方式为删除相关配置项）

1. 哔哩哔哩直播间开播、下播通知
2. 对聊天记录、消息时间、发送者有认知的OpenAI助理
   1. 回应艾特和戳一戳

3. 自动上传聊天图片和语音到对象存储（自定义上传脚本，会在后文展开说明）
4. 命令导出最近n条聊天记录或日志为csv，并回复上传文件url（需启用对象存储）
5. 常用群命令
   1. 禁言机器人
   2. 取消禁言机器人
   3. 更换AI模型
   4. 导出最近N条日志
   5. 导出最近N条本群内消息记录


#### 最少配置如下（仅记录聊天记录）

```toml
[global]
max_sleep_sec = 8

[database]
max_connections = 5
log_table_name = "bot_log"
group_table_prefix = "message"
```

1. `max_sleep_sec = 8`: 所有的事件（除了聊天记录）都会在随机睡眠0到**8**秒后调用处理函数
2. `max_connections = 5`: Sqlite连接池的最大连接数
3. `log_table_name = "bot_log"`: 所有持久化的日志都会写入名为`bot_log` 的数据库表
4. `group_table_prefix = "message"`: 群号1234的日志会被写入名为`message1234`的数据库表

初次启动时会生成一个完整配置模板，修改后重启即可

```toml
[global]
max_sleep_sec = 8

[database]
max_connections = 5
log_table_name = "bot_log"
group_table_prefix = "message"

[object_storage]
script_path = "/a/b/c"

[[groups]]
id = 12345678

[groups.live]
room_id = "12345678"
online_msg = "XX开播了"
offline_msg = "XX下播了"
query_message = "查询直播间"
poll_interval_sec = 60

[groups.agent]
api_url = "https://api.openai.com/v1/chat/completions"
api_key = "API KEY"
model = "chatgpt-4o-latest"
dev_prompt = """
You are a cute and smart catgirl with a strong anime-style personality.
You are the loyal attendant of 你的昵称 and participate in group chats with a playful and engaging demeanor.
Speak only in Mandarin Chinese, and ensure your responses are concise, limited to 4 sentences.
"""
user_prompt = """
Group Members:
<!members!>

Recent Chat History:
<!history!>

New message from someone you <!know!>:
<!message!>

Please respond to this new message in the tone of a playful and lively catgirl.
Speak only in Mandarin Chinese, keep your response under 4 sentences, and stay in character.
"""
aware_history_segments = 30

[groups.agent.known_members]
12345678 = [
    "你的昵称",
    "你的主人",
]
23456789 = [
    "张三",
    "你的敌人",
]

[groups.command]
mute = "禁用聊天回复"
unmute = "启用聊天回复"
switch_model = "更换模型"
dump_history = "最近聊天记录"
dump_log = "最近日志"
admin_ids = [
    1234,
    5678,
]

[[groups]]
id = 12345678

[groups.live]
room_id = "12345678"
online_msg = "XX开播了"
offline_msg = "XX下播了"
query_message = "查询直播间"
poll_interval_sec = 60

[groups.agent]
api_url = "https://api.openai.com/v1/chat/completions"
api_key = "API KEY"
model = "chatgpt-4o-latest"
dev_prompt = """
You are a cute and smart catgirl with a strong anime-style personality.
You are the loyal attendant of 你的昵称 and participate in group chats with a playful and engaging demeanor.
Speak only in Mandarin Chinese, and ensure your responses are concise, limited to 4 sentences.
"""
user_prompt = """
Group Members:
<!members!>

Recent Chat History:
<!history!>

New message from someone you <!know!>:
<!message!>

Please respond to this new message in the tone of a playful and lively catgirl.
Speak only in Mandarin Chinese, keep your response under 4 sentences, and stay in character.
"""
aware_history_segments = 30

[groups.agent.known_members]
23456789 = [
    "张三",
    "你的敌人",
]
12345678 = [
    "你的昵称",
    "你的主人",
]

[groups.command]
mute = "禁用聊天回复"
unmute = "启用聊天回复"
switch_model = "更换模型"
dump_history = "最近聊天记录"
dump_log = "最近日志"
admin_ids = [
    1234,
    5678,
]
```

1. `script_path = "/a/b/c"`: 导出命令、写入图片或语音类型群消息历史记录时被调用的可执行文件路径
   1. 插件会将文件的路径作为第一个命令行参数传入
   2. 插件会收集标准输出并存入数据库（历史记录）或发送到群聊（导出命令）
   3. 当配置的可执行文件运行失败时，插件会收集标准错误并保存到日志
   4. 后文包含了一个示例脚本
2. `groups`
   1. `id = 12345678`: QQ群号为12345678
   2. `live`
      1. `room_id = "12345678"`: 哔哩哔哩直播间号为12345678
      2. `online_msg = "XX开播了"`: 开播时会播报的信息前缀
      3. `offline_msg = "XX下播了"`: 下播时会播报的信息前缀
         1. 开播和下播通知会包含直播间标题，简介，热度，关注，关键帧或封面
      4. `query_message = "查询直播间"`: 在本群内发送“查询直播间”时回复本群主播的直播间信息
      5. `poll_interval_sec = 60`: 每60秒轮询一次直播间状态
   3. `agent`
      1. `api_url = "https://api.openai.com/v1/chat/completions"`: 不要改，目前仅支持OpenAI，配置留作后续可能支持的其他语言模型厂商
      2. `api_key = "API KEY"`: OpenAI的密钥
      3. `model = "chatgpt-4o-latest"`: 仅支持如下几个模型
         1. gpt-4o
         2. chatgpt-4o-latest
         3. gpt-4o-mini
         4. o1-mini
         5. o1-preview
      4. `dev_prompt`, `user_prompt`
         1. 运行期插件会自动使用相应信息替换占位符
            1. `<!members!>`: 配置的`known_members`
            2. `<!history!>`: 从数据库读取的历史记录
            3. `<!message!>`: 用户艾特时发送的信息
            4. `<!know!>`: 用户是否在`known_members`记录中
               1. 会展开为"know/don't know"
      5. `aware_history_segments`: 对话时读取的消息记录，单位是`Segment`而不是`Message`，即一个对话框内每一种消息占用一个位置
   4. `command`: 插件运行时会在标准输出日志内包含每一个命令的正则表达式
      1. `mute = "禁用聊天回复"`: 后面不跟参数
      2. `unmute = "启用聊天回复"`: 后面不跟参数
      3. `switch_model = "更换模型"`: 发送`更换模型 o1-preview`更换模型为`o1-preview`或其他前文提到的支持模型
      4. `dump_history = "最近聊天记录"`: 发送`最近聊天记录 N`调取N个记录
      5. `dump_log = "最近日志"`: 发送`最近日志 N`调取N个记录
      6. `admin_ids = [1234, 5678]`: 仅QQ号为1234或5678的人有权限调用命令

在默认的配置下，匹配的命令正则如下

```
mute: 禁用聊天回复
unmute: 启用聊天回复
switch_model: 更换模型\s+(?<model>gpt4o|chatgpt-4o-latest|gpt-4o-mini|o1-mini|o1-preview)
dump_history: 最近聊天记录\s+(?<count>\d+)
dump_log: 最近日志\s+(?<count>\d+)
```

#### 示例上传脚本

```bash
#!/usr/bin/env bash

if [ $# -lt 1 ]; then
    echo "ERROR: upload.sh missing args"
    exit 1
fi

SOURCE_FILE="$1"
FILE_NAME="$(basename "$SOURCE_FILE")"
EXTENSION="${FILE_NAME##*.}"

NEW_NAME="$(cat /proc/sys/kernel/random/uuid).$EXTENSION"
BUCKET="YOUR_BUCKET"
REGION="YOUR_REGION"

sudo cp "$SOURCE_FILE" "./$NEW_NAME"
sudo chown "$(whoami):" "./$NEW_NAME"
aws s3 cp "./$NEW_NAME" "s3://$BUCKET" > /dev/null
rm "./$NEW_NAME"
echo -n "https://${BUCKET}.s3.${REGION}.amazonaws.com/${NEW_NAME}"
```

#### 表结构

`bot_log`

1. time: ISO8601 时间, `YYYY-MM-DD HH:MM:SS` 
2. level: DEBUG, INFO, WARN, ERROR
3. content: 日志内容

`message_table_prefix_XXXXXXX`

1. message_id: 消息id，详情参考Onebot v11文档
2. time: ISO8601 时间， `YYYY-MM-DD HH:MM:SS`
3. sender_id: 发送者qq号
4. sender_name: 发送者名称，优先级从高到低为 配置文件、群昵称，用户昵称，qq号
5. type: Segment type，详情参考Onebot v11文档
6. content: 原始Onebot Json返回的内容，图片和语音会被替换成本地路径
7. interpret: 当类型是图片或语音时为上传后的url，其余情况下为附带信息
