//! Datatypes for exceptions caused by plugin and user.
use thiserror::Error;
pub type PluginResult<T> = Result<T, PluginError>;

#[derive(Error, Debug)]
pub enum PluginError {
    #[error("IO error: {0}.")]
    IO(#[from] std::io::Error),
    #[error("Database error: {0}.")]
    Database(#[from] sqlx::Error),
    #[error("Timestamp out of bound: {0}.")]
    TimeStampOutOfBound(#[from] time::error::ComponentRange),
    #[error("Time format error: {0}.")]
    TimeFormat(#[from] time::error::Format),
    #[error("Reqwest error: {0}.")]
    HttpRequest(#[from] reqwest::Error),
    #[error("Regex error: {0}.")]
    Regex(#[from] regex::Error),
    #[error("Agent request error: {0}.")]
    AgentRequest(String),
    #[error("Serialize to toml failed, cause: {0}")]
    SerializeToml(String),
    #[error("Deserialize to toml failed, cause: {0}")]
    DeserializeToml(String),
    #[error("Path not available: {0}.")]
    PathNotAvailable(String),
    #[error("Launched child process {0} failed, cause: {1}")]
    ChildProcess(String, String),
    #[error("Initialize global state failed, cause: {0}")]
    InitGlobalState(String),
    #[error("Trap to logically unreachable control.")]
    Unreachable,
}
